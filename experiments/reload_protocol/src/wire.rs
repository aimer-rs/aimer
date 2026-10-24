use std::io::{Read, Write};

use ring::aead;
use ring::hkdf;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use super::{
    ModuleMetadata, ProtocolError, ProtocolLimits, ReloadCommand, ReloadResult, SessionCredentials,
    TransferAcknowledgement,
};

const PROTOCOL_MAJOR: u16 = 1;
const PROTOCOL_MINOR: u16 = 0;
const HANDSHAKE_MAGIC: &[u8; 4] = b"AMRH";
const FRAME_MAGIC: &[u8; 4] = b"AMRL";
const CHALLENGE_LEN: usize = 40;
const CLIENT_AUTH_LEN: usize = 88;
const SERVER_AUTH_LEN: usize = 40;
const FRAME_HEADER_LEN: usize = 88;
const AUTH_TAG_START: usize = 56;
const AUTH_TAG_END: usize = 88;
const KIND_MODULE_BEGIN: u16 = 1;
const KIND_MODULE_CHUNK: u16 = 2;
const KIND_MODULE_END: u16 = 3;
const KIND_UPLOAD_ACCEPTED: u16 = 4;
const KIND_RELOAD_BEGIN: u16 = 5;
const KIND_RELOAD_RESULT: u16 = 6;
const KIND_RESULT_QUERY: u16 = 7;
const KIND_RESULT_UNAVAILABLE: u16 = 8;
const RELOAD_BEGIN_LEN: usize = 108;

#[derive(Clone, Copy)]
struct FrameKeyLength;

impl hkdf::KeyType for FrameKeyLength {
    fn len(&self) -> usize {
        32
    }
}

struct DirectionalKeys {
    client_to_server: FrameKeys,
    server_to_client: FrameKeys,
}

struct FrameKeys {
    authentication: hmac::Key,
    encryption: aead::LessSafeKey,
}

struct Frame {
    kind: u16,
    request_id: u64,
    sequence: u64,
    payload: Vec<u8>,
}

/// The authenticated operation served by one accepted connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadConnectionOutcome {
    /// A complete command executed or recovered its idempotent result.
    Command(ReloadResult),
    /// A reconnect query recovered a terminal result, or found none retained.
    Query(Option<ReloadResult>),
}

/// Authenticates and sends one complete module over an ordered byte stream.
pub fn send_module<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
    module: &[u8],
) -> Result<TransferAcknowledgement, ProtocolError>
where
    S: Read + Write,
{
    if module.len() > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module.len(),
            maximum: limits.max_module_bytes(),
        });
    }
    if !module.is_empty() && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    let keys = client_handshake(stream, credentials)?;
    let digest: [u8; 32] = Sha256::digest(module).into();
    let mut begin = Vec::with_capacity(40);
    begin.extend_from_slice(&(module.len() as u64).to_le_bytes());
    begin.extend_from_slice(&digest);
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_BEGIN,
        request_id,
        0,
        &begin,
    )?;
    let chunk_limit = limits.chunk_bytes_limit().max(1);
    let mut sequence = 1_u64;
    for (chunk_index, bytes) in module.chunks(chunk_limit).enumerate() {
        let offset = chunk_index
            .checked_mul(chunk_limit)
            .ok_or(ProtocolError::InvalidFrame("module chunk offset overflow"))?;
        let mut chunk = Vec::with_capacity(8 + bytes.len());
        chunk.extend_from_slice(&(offset as u64).to_le_bytes());
        chunk.extend_from_slice(bytes);
        write_frame(
            stream,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_CHUNK,
            request_id,
            sequence,
            &chunk,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        request_id,
        sequence,
        &[],
    )?;
    stream.flush()?;

    let acknowledgement = read_frame(stream, &keys.server_to_client, credentials.session_id(), 40)?;
    if acknowledgement.kind != KIND_UPLOAD_ACCEPTED
        || acknowledgement.request_id != request_id
        || acknowledgement.sequence != 0
        || acknowledgement.payload.len() != 40
    {
        return Err(ProtocolError::InvalidFrame(
            "invalid upload acknowledgement",
        ));
    }
    let acknowledged_len = usize_from_u64(read_u64(&acknowledgement.payload[0..8])?)?;
    let mut acknowledged_digest = [0_u8; 32];
    acknowledged_digest.copy_from_slice(&acknowledgement.payload[8..40]);
    if acknowledged_len != module.len() || !bool::from(acknowledged_digest.ct_eq(&digest)) {
        return Err(ProtocolError::InvalidFrame(
            "acknowledgement does not match upload",
        ));
    }

    Ok(TransferAcknowledgement {
        request_id,
        module_len: acknowledged_len,
        module_digest: acknowledged_digest,
    })
}

/// Sends one complete reload command and waits for its safe-point result.
///
/// Upload acknowledgement is deliberately not returned as a terminal status.
/// This call succeeds only after the app reports committed, rejected, or
/// cancelled through the authenticated response direction.
pub fn send_reload_command<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
    metadata: ModuleMetadata,
    module: &[u8],
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
{
    validate_outgoing_module(limits, module)?;
    let keys = client_handshake(stream, credentials)?;
    let digest: [u8; 32] = Sha256::digest(module).into();
    let mut begin = Vec::with_capacity(RELOAD_BEGIN_LEN);
    begin.extend_from_slice(&(module.len() as u64).to_le_bytes());
    begin.extend_from_slice(&digest);
    encode_metadata(metadata, &mut begin);
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_RELOAD_BEGIN,
        request_id,
        0,
        &begin,
    )?;
    let end_sequence = write_module_chunks(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        limits,
        request_id,
        module,
    )?;
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        request_id,
        end_sequence,
        &[],
    )?;
    stream.flush()?;

    let acknowledgement = read_frame(stream, &keys.server_to_client, credentials.session_id(), 40)?;
    validate_acknowledgement(&acknowledgement, request_id, module.len(), &digest)?;
    let max_result_bytes = limits
        .diagnostic_bytes_limit()
        .checked_add(20)
        .ok_or(ProtocolError::InvalidFrame("reload result limit overflow"))?;
    let terminal = read_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        max_result_bytes,
    )?;
    if terminal.kind != KIND_RELOAD_RESULT
        || terminal.request_id != request_id
        || terminal.sequence != 1
    {
        return Err(ProtocolError::InvalidFrame("invalid reload-result frame"));
    }
    ReloadResult::decode(&terminal.payload, limits.diagnostic_bytes_limit())
}

/// Receives one authenticated command and returns its terminal host result.
///
/// The command callback is entered only after all chunks, total length, and
/// digest validate. The accepted-upload frame is sent first; the callback's
/// result is then reported as the authoritative terminal outcome.
pub fn receive_reload_command<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    execute: F,
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
{
    let keys = server_handshake(stream, credentials)?;
    let begin = read_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        RELOAD_BEGIN_LEN,
    )?;
    receive_reload_command_after_begin(stream, credentials, limits, &keys, begin, execute)
}

/// Serves either a complete command or a reconnect result query.
pub fn receive_reload_connection<S, F, Q>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    execute: F,
    query: Q,
) -> Result<ReloadConnectionOutcome, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
    Q: FnOnce(u64) -> Option<ReloadResult>,
{
    let keys = server_handshake(stream, credentials)?;
    let first = read_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        RELOAD_BEGIN_LEN,
    )?;
    match first.kind {
        KIND_RELOAD_BEGIN => Ok(ReloadConnectionOutcome::Command(
            receive_reload_command_after_begin(stream, credentials, limits, &keys, first, execute)?,
        )),
        KIND_RESULT_QUERY => {
            if first.sequence != 0 || !first.payload.is_empty() {
                return Err(ProtocolError::InvalidFrame("invalid result-query frame"));
            }
            let result = query(first.request_id);
            match &result {
                Some(result) => {
                    let payload = result.encode(limits.diagnostic_bytes_limit())?;
                    write_frame(
                        stream,
                        &keys.server_to_client,
                        credentials.session_id(),
                        KIND_RELOAD_RESULT,
                        first.request_id,
                        0,
                        &payload,
                    )?;
                }
                None => write_frame(
                    stream,
                    &keys.server_to_client,
                    credentials.session_id(),
                    KIND_RESULT_UNAVAILABLE,
                    first.request_id,
                    0,
                    &[],
                )?,
            }
            stream.flush()?;
            Ok(ReloadConnectionOutcome::Query(result))
        }
        _ => Err(ProtocolError::InvalidFrame(
            "unexpected authenticated command kind",
        )),
    }
}

/// Queries a terminal result after reconnecting to the same app session.
pub fn query_reload_result<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
) -> Result<Option<ReloadResult>, ProtocolError>
where
    S: Read + Write,
{
    let keys = client_handshake(stream, credentials)?;
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_RESULT_QUERY,
        request_id,
        0,
        &[],
    )?;
    stream.flush()?;
    let max_result_bytes = limits
        .diagnostic_bytes_limit()
        .checked_add(20)
        .ok_or(ProtocolError::InvalidFrame("reload result limit overflow"))?;
    let response = read_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        max_result_bytes,
    )?;
    if response.request_id != request_id || response.sequence != 0 {
        return Err(ProtocolError::InvalidFrame("invalid result-query response"));
    }
    match response.kind {
        KIND_RELOAD_RESULT => Ok(Some(ReloadResult::decode(
            &response.payload,
            limits.diagnostic_bytes_limit(),
        )?)),
        KIND_RESULT_UNAVAILABLE if response.payload.is_empty() => Ok(None),
        _ => Err(ProtocolError::InvalidFrame("invalid result-query response")),
    }
}

fn receive_reload_command_after_begin<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    keys: &DirectionalKeys,
    begin: Frame,
    execute: F,
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
{
    if begin.kind != KIND_RELOAD_BEGIN
        || begin.sequence != 0
        || begin.payload.len() != RELOAD_BEGIN_LEN
    {
        return Err(ProtocolError::InvalidFrame("invalid reload-begin frame"));
    }
    let module_len = usize_from_u64(read_u64(&begin.payload[0..8])?)?;
    if module_len > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module_len,
            maximum: limits.max_module_bytes(),
        });
    }
    let mut declared_digest = [0_u8; 32];
    declared_digest.copy_from_slice(&begin.payload[8..40]);
    let metadata = decode_metadata(&begin.payload[40..RELOAD_BEGIN_LEN])?;
    let (module, actual_digest, end_sequence) = read_module_chunks(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        limits,
        begin.request_id,
        module_len,
    )?;
    let end = read_frame(stream, &keys.client_to_server, credentials.session_id(), 0)?;
    if end.kind != KIND_MODULE_END
        || end.request_id != begin.request_id
        || end.sequence != end_sequence
        || !end.payload.is_empty()
    {
        return Err(ProtocolError::InvalidFrame("invalid module-end frame"));
    }
    if !bool::from(actual_digest.ct_eq(&declared_digest)) {
        return Err(ProtocolError::DigestMismatch);
    }

    write_acknowledgement(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        begin.request_id,
        module_len,
        &actual_digest,
    )?;
    stream.flush()?;
    let result = execute(ReloadCommand::from_parts(
        begin.request_id,
        metadata,
        actual_digest,
        module,
    ));
    let payload = result.encode(limits.diagnostic_bytes_limit())?;
    write_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        KIND_RELOAD_RESULT,
        begin.request_id,
        1,
        &payload,
    )?;
    stream.flush()?;
    Ok(result)
}

/// Authenticates, receives, dispatches, and acknowledges one complete module.
pub fn receive_module_and_acknowledge<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    accept: F,
) -> Result<TransferAcknowledgement, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(Vec<u8>) -> Result<(), String>,
{
    let keys = server_handshake(stream, credentials)?;
    let begin = read_frame(stream, &keys.client_to_server, credentials.session_id(), 40)?;
    if begin.kind != KIND_MODULE_BEGIN || begin.sequence != 0 || begin.payload.len() != 40 {
        return Err(ProtocolError::InvalidFrame("invalid module-begin frame"));
    }
    let module_len = usize_from_u64(read_u64(&begin.payload[0..8])?)?;
    if module_len > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module_len,
            maximum: limits.max_module_bytes(),
        });
    }
    let mut declared_digest = [0_u8; 32];
    declared_digest.copy_from_slice(&begin.payload[8..40]);
    if module_len != 0 && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }

    let mut module = Vec::with_capacity(module_len);
    let mut digest = Sha256::new();
    let mut sequence = 1_u64;
    while module.len() < module_len {
        let remaining = module_len - module.len();
        let chunk_bytes = remaining.min(limits.chunk_bytes_limit());
        let chunk_payload_len = chunk_bytes
            .checked_add(8)
            .ok_or(ProtocolError::InvalidFrame("module chunk length overflow"))?;
        let chunk = read_frame(
            stream,
            &keys.client_to_server,
            credentials.session_id(),
            chunk_payload_len,
        )?;
        if chunk.kind != KIND_MODULE_CHUNK
            || chunk.request_id != begin.request_id
            || chunk.sequence != sequence
            || chunk.payload.len() < 8
            || chunk.payload.len() > chunk_payload_len
            || usize_from_u64(read_u64(&chunk.payload[0..8])?)? != module.len()
            || chunk.payload.len() == 8
        {
            return Err(ProtocolError::InvalidFrame("invalid module-chunk frame"));
        }
        let bytes = &chunk.payload[8..];
        digest.update(bytes);
        module.extend_from_slice(bytes);
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }

    let end = read_frame(stream, &keys.client_to_server, credentials.session_id(), 0)?;
    if end.kind != KIND_MODULE_END
        || end.request_id != begin.request_id
        || end.sequence != sequence
        || !end.payload.is_empty()
    {
        return Err(ProtocolError::InvalidFrame("invalid module-end frame"));
    }
    let actual_digest: [u8; 32] = digest.finalize().into();
    if !bool::from(actual_digest.ct_eq(&declared_digest)) {
        return Err(ProtocolError::DigestMismatch);
    }

    accept(module).map_err(ProtocolError::SinkRejected)?;
    let mut acknowledgement_payload = Vec::with_capacity(40);
    acknowledgement_payload.extend_from_slice(&(module_len as u64).to_le_bytes());
    acknowledgement_payload.extend_from_slice(&actual_digest);
    write_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        KIND_UPLOAD_ACCEPTED,
        begin.request_id,
        0,
        &acknowledgement_payload,
    )?;
    stream.flush()?;

    Ok(TransferAcknowledgement {
        request_id: begin.request_id,
        module_len,
        module_digest: actual_digest,
    })
}

fn client_handshake<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
) -> Result<DirectionalKeys, ProtocolError>
where
    S: Read + Write,
{
    let mut challenge = [0_u8; CHALLENGE_LEN];
    stream.read_exact(&mut challenge)?;
    if &challenge[0..4] != HANDSHAKE_MAGIC
        || read_u16(&challenge[4..6])? != PROTOCOL_MAJOR
        || read_u16(&challenge[6..8])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    let mut server_nonce = [0_u8; 32];
    server_nonce.copy_from_slice(&challenge[8..40]);
    let client_nonce = random_array()?;
    let transcript = transcript(credentials.session_id(), &server_nonce, &client_nonce);
    let client_tag = authentication_tag(credentials, b"client", &transcript);
    let mut client_auth = [0_u8; CLIENT_AUTH_LEN];
    client_auth[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    client_auth[4..20].copy_from_slice(credentials.session_id());
    client_auth[20..52].copy_from_slice(&client_nonce);
    client_auth[52..54].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    client_auth[54..56].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    client_auth[56..88].copy_from_slice(client_tag.as_ref());
    stream.write_all(&client_auth)?;
    stream.flush()?;

    let mut server_auth = [0_u8; SERVER_AUTH_LEN];
    stream.read_exact(&mut server_auth)?;
    if &server_auth[0..4] != HANDSHAKE_MAGIC
        || read_u16(&server_auth[4..6])? != PROTOCOL_MAJOR
        || read_u16(&server_auth[6..8])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    verify_authentication_tag(credentials, b"server", &transcript, &server_auth[8..40])?;
    derive_keys(credentials, &transcript)
}

fn server_handshake<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
) -> Result<DirectionalKeys, ProtocolError>
where
    S: Read + Write,
{
    let server_nonce = random_array()?;
    let mut challenge = [0_u8; CHALLENGE_LEN];
    challenge[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    challenge[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    challenge[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    challenge[8..40].copy_from_slice(&server_nonce);
    stream.write_all(&challenge)?;
    stream.flush()?;

    let mut client_auth = [0_u8; CLIENT_AUTH_LEN];
    stream.read_exact(&mut client_auth)?;
    if &client_auth[0..4] != HANDSHAKE_MAGIC
        || !bool::from(client_auth[4..20].ct_eq(credentials.session_id()))
        || read_u16(&client_auth[52..54])? != PROTOCOL_MAJOR
        || read_u16(&client_auth[54..56])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    let mut client_nonce = [0_u8; 32];
    client_nonce.copy_from_slice(&client_auth[20..52]);
    let transcript = transcript(credentials.session_id(), &server_nonce, &client_nonce);
    verify_authentication_tag(credentials, b"client", &transcript, &client_auth[56..88])?;
    let server_tag = authentication_tag(credentials, b"server", &transcript);
    let mut server_auth = [0_u8; SERVER_AUTH_LEN];
    server_auth[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    server_auth[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    server_auth[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    server_auth[8..40].copy_from_slice(server_tag.as_ref());
    stream.write_all(&server_auth)?;
    stream.flush()?;
    derive_keys(credentials, &transcript)
}

fn transcript(session_id: &[u8; 16], server_nonce: &[u8; 32], client_nonce: &[u8; 32]) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(84);
    transcript.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    transcript.extend_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    transcript.extend_from_slice(session_id);
    transcript.extend_from_slice(server_nonce);
    transcript.extend_from_slice(client_nonce);
    transcript
}

fn authentication_tag(
    credentials: &SessionCredentials,
    role: &[u8],
    transcript: &[u8],
) -> hmac::Tag {
    let key = hmac::Key::new(hmac::HMAC_SHA256, credentials.token.as_ref());
    let mut authenticated = Vec::with_capacity(role.len() + transcript.len());
    authenticated.extend_from_slice(role);
    authenticated.extend_from_slice(transcript);
    hmac::sign(&key, &authenticated)
}

fn verify_authentication_tag(
    credentials: &SessionCredentials,
    role: &[u8],
    transcript: &[u8],
    tag: &[u8],
) -> Result<(), ProtocolError> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, credentials.token.as_ref());
    let mut authenticated = Vec::with_capacity(role.len() + transcript.len());
    authenticated.extend_from_slice(role);
    authenticated.extend_from_slice(transcript);
    hmac::verify(&key, &authenticated, tag).map_err(|_| ProtocolError::Authentication)
}

fn derive_keys(
    credentials: &SessionCredentials,
    transcript: &[u8],
) -> Result<DirectionalKeys, ProtocolError> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, transcript);
    let pseudo_random_key = salt.extract(credentials.token.as_ref());
    let client_to_server = derive_frame_keys(
        &pseudo_random_key,
        b"aimer-reload-client-to-server-auth",
        b"aimer-reload-client-to-server-encryption",
    )?;
    let server_to_client = derive_frame_keys(
        &pseudo_random_key,
        b"aimer-reload-server-to-client-auth",
        b"aimer-reload-server-to-client-encryption",
    )?;
    Ok(DirectionalKeys {
        client_to_server,
        server_to_client,
    })
}

fn derive_frame_keys(
    pseudo_random_key: &hkdf::Prk,
    authentication_label: &'static [u8],
    encryption_label: &'static [u8],
) -> Result<FrameKeys, ProtocolError> {
    let mut authentication_bytes = expand_key(pseudo_random_key, authentication_label)?;
    let authentication = hmac::Key::new(hmac::HMAC_SHA256, &authentication_bytes);
    authentication_bytes.zeroize();
    let mut encryption_bytes = expand_key(pseudo_random_key, encryption_label)?;
    let unbound = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &encryption_bytes)
        .map_err(|_| ProtocolError::Cryptography)?;
    encryption_bytes.zeroize();
    Ok(FrameKeys {
        authentication,
        encryption: aead::LessSafeKey::new(unbound),
    })
}

fn expand_key(
    pseudo_random_key: &hkdf::Prk,
    label: &'static [u8],
) -> Result<[u8; 32], ProtocolError> {
    let labels = [label];
    let output = pseudo_random_key
        .expand(&labels, FrameKeyLength)
        .map_err(|_| ProtocolError::Cryptography)?;
    let mut key = [0_u8; 32];
    output
        .fill(&mut key)
        .map_err(|_| ProtocolError::Cryptography)?;
    Ok(key)
}

fn write_frame<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    kind: u16,
    request_id: u64,
    sequence: u64,
    payload: &[u8],
) -> Result<(), ProtocolError>
where
    S: Write,
{
    let encrypted_len = payload
        .len()
        .checked_add(aead::CHACHA20_POLY1305.tag_len())
        .ok_or(ProtocolError::InvalidFrame(
            "encrypted payload length overflow",
        ))?;
    let mut header = [0_u8; FRAME_HEADER_LEN];
    header[0..4].copy_from_slice(FRAME_MAGIC);
    header[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    header[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    header[8..10].copy_from_slice(&kind.to_le_bytes());
    header[12..14].copy_from_slice(&(FRAME_HEADER_LEN as u16).to_le_bytes());
    header[16..24].copy_from_slice(&(encrypted_len as u64).to_le_bytes());
    header[24..40].copy_from_slice(session_id);
    header[40..48].copy_from_slice(&request_id.to_le_bytes());
    header[48..56].copy_from_slice(&sequence.to_le_bytes());
    let mut encrypted_payload = Vec::with_capacity(encrypted_len);
    encrypted_payload.extend_from_slice(payload);
    keys.encryption
        .seal_in_place_append_tag(
            frame_nonce(sequence),
            aead::Aad::from(&header),
            &mut encrypted_payload,
        )
        .map_err(|_| ProtocolError::Cryptography)?;
    let mut authenticated = Vec::with_capacity(FRAME_HEADER_LEN + encrypted_payload.len());
    authenticated.extend_from_slice(&header);
    authenticated.extend_from_slice(&encrypted_payload);
    let tag = hmac::sign(&keys.authentication, &authenticated);
    header[AUTH_TAG_START..AUTH_TAG_END].copy_from_slice(tag.as_ref());
    stream.write_all(&header)?;
    stream.write_all(&encrypted_payload)?;
    Ok(())
}

fn read_frame<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    max_payload: usize,
) -> Result<Frame, ProtocolError>
where
    S: Read,
{
    let mut header = [0_u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut header)?;
    if &header[0..4] != FRAME_MAGIC
        || read_u16(&header[4..6])? != PROTOCOL_MAJOR
        || read_u16(&header[6..8])? != PROTOCOL_MINOR
        || read_u16(&header[10..12])? != 0
        || read_u16(&header[12..14])? as usize != FRAME_HEADER_LEN
        || read_u16(&header[14..16])? != 0
        || !bool::from(header[24..40].ct_eq(session_id))
    {
        return Err(ProtocolError::InvalidFrame("invalid authenticated header"));
    }
    let payload_len = usize_from_u64(read_u64(&header[16..24])?)?;
    let max_encrypted_payload = max_payload
        .checked_add(aead::CHACHA20_POLY1305.tag_len())
        .ok_or(ProtocolError::InvalidFrame("payload limit overflow"))?;
    if payload_len < aead::CHACHA20_POLY1305.tag_len() || payload_len > max_encrypted_payload {
        return Err(ProtocolError::InvalidFrame("payload exceeds message limit"));
    }
    let mut payload = vec![0_u8; payload_len];
    stream.read_exact(&mut payload)?;
    let mut tag = [0_u8; 32];
    tag.copy_from_slice(&header[AUTH_TAG_START..AUTH_TAG_END]);
    header[AUTH_TAG_START..AUTH_TAG_END].fill(0);
    let mut authenticated = Vec::with_capacity(FRAME_HEADER_LEN + payload_len);
    authenticated.extend_from_slice(&header);
    authenticated.extend_from_slice(&payload);
    hmac::verify(&keys.authentication, &authenticated, &tag)
        .map_err(|_| ProtocolError::Authentication)?;

    let sequence = read_u64(&header[48..56])?;
    let plaintext = keys
        .encryption
        .open_in_place(
            frame_nonce(sequence),
            aead::Aad::from(&header),
            &mut payload,
        )
        .map_err(|_| ProtocolError::Authentication)?;
    let plaintext_len = plaintext.len();
    payload.truncate(plaintext_len);

    Ok(Frame {
        kind: read_u16(&header[8..10])?,
        request_id: read_u64(&header[40..48])?,
        sequence,
        payload,
    })
}

fn frame_nonce(sequence: u64) -> aead::Nonce {
    let mut nonce = [0_u8; 12];
    nonce[4..12].copy_from_slice(&sequence.to_be_bytes());
    aead::Nonce::assume_unique_for_key(nonce)
}

fn validate_outgoing_module(limits: ProtocolLimits, module: &[u8]) -> Result<(), ProtocolError> {
    if module.len() > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module.len(),
            maximum: limits.max_module_bytes(),
        });
    }
    if !module.is_empty() && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    Ok(())
}

fn write_module_chunks<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    limits: ProtocolLimits,
    request_id: u64,
    module: &[u8],
) -> Result<u64, ProtocolError>
where
    S: Write,
{
    let chunk_limit = limits.chunk_bytes_limit().max(1);
    let mut sequence = 1_u64;
    for (chunk_index, bytes) in module.chunks(chunk_limit).enumerate() {
        let offset = chunk_index
            .checked_mul(chunk_limit)
            .ok_or(ProtocolError::InvalidFrame("module chunk offset overflow"))?;
        let mut chunk = Vec::with_capacity(8 + bytes.len());
        chunk.extend_from_slice(&(offset as u64).to_le_bytes());
        chunk.extend_from_slice(bytes);
        write_frame(
            stream,
            keys,
            session_id,
            KIND_MODULE_CHUNK,
            request_id,
            sequence,
            &chunk,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    Ok(sequence)
}

fn read_module_chunks<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    limits: ProtocolLimits,
    request_id: u64,
    module_len: usize,
) -> Result<(Vec<u8>, [u8; 32], u64), ProtocolError>
where
    S: Read,
{
    if module_len != 0 && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    let mut module = Vec::with_capacity(module_len);
    let mut digest = Sha256::new();
    let mut sequence = 1_u64;
    while module.len() < module_len {
        let remaining = module_len - module.len();
        let chunk_payload_limit = remaining
            .min(limits.chunk_bytes_limit())
            .checked_add(8)
            .ok_or(ProtocolError::InvalidFrame("module chunk length overflow"))?;
        let chunk = read_frame(stream, keys, session_id, chunk_payload_limit)?;
        if chunk.kind != KIND_MODULE_CHUNK
            || chunk.request_id != request_id
            || chunk.sequence != sequence
            || chunk.payload.len() < 9
            || chunk.payload.len() > chunk_payload_limit
            || usize_from_u64(read_u64(&chunk.payload[0..8])?)? != module.len()
        {
            return Err(ProtocolError::InvalidFrame("invalid module-chunk frame"));
        }
        let bytes = &chunk.payload[8..];
        digest.update(bytes);
        module.extend_from_slice(bytes);
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    Ok((module, digest.finalize().into(), sequence))
}

fn validate_acknowledgement(
    frame: &Frame,
    request_id: u64,
    module_len: usize,
    module_digest: &[u8; 32],
) -> Result<(), ProtocolError> {
    if frame.kind != KIND_UPLOAD_ACCEPTED
        || frame.request_id != request_id
        || frame.sequence != 0
        || frame.payload.len() != 40
        || usize_from_u64(read_u64(&frame.payload[0..8])?)? != module_len
        || !bool::from(frame.payload[8..40].ct_eq(module_digest))
    {
        return Err(ProtocolError::InvalidFrame(
            "invalid upload acknowledgement",
        ));
    }
    Ok(())
}

fn write_acknowledgement<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    request_id: u64,
    module_len: usize,
    module_digest: &[u8; 32],
) -> Result<(), ProtocolError>
where
    S: Write,
{
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&(module_len as u64).to_le_bytes());
    payload.extend_from_slice(module_digest);
    write_frame(
        stream,
        keys,
        session_id,
        KIND_UPLOAD_ACCEPTED,
        request_id,
        0,
        &payload,
    )
}

fn encode_metadata(metadata: ModuleMetadata, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&metadata.application_id());
    bytes.extend_from_slice(&metadata.build_id());
    let (abi_major, abi_minor) = metadata.abi_version();
    bytes.extend_from_slice(&abi_major.to_le_bytes());
    bytes.extend_from_slice(&abi_minor.to_le_bytes());
    bytes.extend_from_slice(&metadata.capability_manifest_digest());
}

fn decode_metadata(bytes: &[u8]) -> Result<ModuleMetadata, ProtocolError> {
    if bytes.len() != 68 {
        return Err(ProtocolError::InvalidFrame(
            "invalid module metadata length",
        ));
    }
    let mut application_id = [0_u8; 16];
    application_id.copy_from_slice(&bytes[0..16]);
    let mut build_id = [0_u8; 16];
    build_id.copy_from_slice(&bytes[16..32]);
    let abi_major = read_u16(&bytes[32..34])?;
    let abi_minor = read_u16(&bytes[34..36])?;
    let mut capability_manifest_digest = [0_u8; 32];
    capability_manifest_digest.copy_from_slice(&bytes[36..68]);
    Ok(ModuleMetadata::new(
        application_id,
        build_id,
        abi_major,
        abi_minor,
        capability_manifest_digest,
    ))
}

fn random_array() -> Result<[u8; 32], ProtocolError> {
    let mut bytes = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| ProtocolError::Cryptography)?;
    Ok(bytes)
}

fn read_u16(bytes: &[u8]) -> Result<u16, ProtocolError> {
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("truncated u16"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u64(bytes: &[u8]) -> Result<u64, ProtocolError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("truncated u64"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn usize_from_u64(value: u64) -> Result<usize, ProtocolError> {
    value
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("length does not fit this target"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn module_transfer_accepts_every_chunk_boundary_without_buffering_one_module_frame() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1)).max_chunk_bytes(3);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_credentials = credentials.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            receive_module_and_acknowledge(&mut stream, &server_credentials, limits, |module| {
                assert_eq!(module, b"0123456789");
                Ok(())
            })
            .unwrap()
        });
        let mut stream = TcpStream::connect(address).unwrap();

        let acknowledgement =
            send_module(&mut stream, &credentials, limits, 7, b"0123456789").unwrap();

        assert_eq!(acknowledgement.module_len, 10);
        assert_eq!(server.join().unwrap(), acknowledgement);
    }

    #[test]
    fn authenticated_frame_has_a_stable_golden_vector() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
        let keys = derive_keys(&credentials, &transcript).unwrap();
        let mut bytes = Vec::new();

        write_frame(
            &mut bytes,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_END,
            0x0102_0304_0506_0708,
            9,
            b"golden",
        )
        .unwrap();

        assert_eq!(
            hex::encode(bytes),
            "414d524c01000000030000005800000016000000000000001111111111111111111111111111111108070605040302010900000000000000f337a33a670f7f15e10cb873eebd2dbbf317092ae625c252b18fcbf7db09c45b3df6e3e86c01071b83f4be4c1702f466469b8279b760"
        );
    }

    #[test]
    fn every_frame_truncation_and_authenticated_header_mutation_is_rejected() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
        let keys = derive_keys(&credentials, &transcript).unwrap();
        let mut valid = Vec::new();
        write_frame(
            &mut valid,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_END,
            7,
            2,
            b"frame",
        )
        .unwrap();

        for end in 0..valid.len() {
            assert!(
                read_frame(
                    &mut Cursor::new(&valid[..end]),
                    &keys.client_to_server,
                    credentials.session_id(),
                    5,
                )
                .is_err()
            );
        }
        for index in [0, 4, 6, 10, 12, 14, 24, 56, valid.len() - 1] {
            let mut malformed = valid.clone();
            malformed[index] ^= 1;
            assert!(
                read_frame(
                    &mut Cursor::new(malformed),
                    &keys.client_to_server,
                    credentials.session_id(),
                    5,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn client_rejects_an_incompatible_handshake_minor_before_authentication() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let mut challenge = Vec::new();
        challenge.extend_from_slice(HANDSHAKE_MAGIC);
        challenge.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
        challenge.extend_from_slice(&(PROTOCOL_MINOR + 1).to_le_bytes());
        challenge.extend_from_slice(&[0x22; 32]);
        let mut stream = Cursor::new(challenge);

        assert!(matches!(
            client_handshake(&mut stream, &credentials),
            Err(ProtocolError::Authentication)
        ));
    }

    #[test]
    fn authenticated_chunk_duplicates_gaps_and_overlaps_are_rejected() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(6, Duration::from_secs(1)).max_chunk_bytes(3);

        for (second_sequence, second_offset) in [(1_u64, 3_u64), (3, 3), (2, 2), (2, 4)] {
            let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
            let keys = derive_keys(&credentials, &transcript).unwrap();
            let mut frames = Vec::new();
            write_frame(
                &mut frames,
                &keys.client_to_server,
                credentials.session_id(),
                KIND_MODULE_CHUNK,
                7,
                1,
                &[&0_u64.to_le_bytes()[..], b"abc"].concat(),
            )
            .unwrap();
            write_frame(
                &mut frames,
                &keys.client_to_server,
                credentials.session_id(),
                KIND_MODULE_CHUNK,
                7,
                second_sequence,
                &[&second_offset.to_le_bytes()[..], b"def"].concat(),
            )
            .unwrap();

            assert!(
                read_module_chunks(
                    &mut Cursor::new(frames),
                    &keys.client_to_server,
                    credentials.session_id(),
                    limits,
                    7,
                    6,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn reconnect_transcripts_derive_separate_keys_and_reject_replayed_frames() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let first_transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
        let second_transcript = transcript(credentials.session_id(), &[0x44; 32], &[0x55; 32]);
        let first_keys = derive_keys(&credentials, &first_transcript).unwrap();
        let second_keys = derive_keys(&credentials, &second_transcript).unwrap();
        let mut first_frame = Vec::new();
        let mut second_frame = Vec::new();
        write_frame(
            &mut first_frame,
            &first_keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_END,
            7,
            1,
            b"same",
        )
        .unwrap();
        write_frame(
            &mut second_frame,
            &second_keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_END,
            7,
            1,
            b"same",
        )
        .unwrap();

        assert_ne!(first_frame, second_frame);
        assert!(matches!(
            read_frame(
                &mut Cursor::new(first_frame),
                &second_keys.client_to_server,
                credentials.session_id(),
                4,
            ),
            Err(ProtocolError::Authentication)
        ));
    }
}
