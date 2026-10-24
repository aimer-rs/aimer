use crossbeam::queue::SegQueue;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use aimer_anteros::{GenerationLimits, ModelLimits, RuntimeConfig, StateTransferCoordinator};
use aimer_reload_protocol::{
    DevelopmentHostConfig, MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES, ProtocolLimits,
    SessionCredentials,
};
use aimer_reload_server::ListenerSecurity;

use super::{LiveReloadConfig, ReloadCandidateLimits};

const SESSION_ID_VARIABLE: &str = "AIMER_RELOAD_SESSION_ID";
const SESSION_TOKEN_VARIABLE: &str = "AIMER_RELOAD_SESSION_TOKEN";
const LISTENER_PORT_VARIABLE: &str = "AIMER_RELOAD_LISTENER_PORT";
const HOST_CONFIG_VARIABLE: &str = "AIMER_RELOAD_HOST_CONFIG";
#[cfg(target_os = "ios")]
const IOS_SERVICE_NAME_VARIABLE: &str = "AIMER_RELOAD_SERVICE_NAME";
const SESSION_ID_HEX_BYTES: usize = 32;
const SESSION_TOKEN_HEX_BYTES: usize = 64;

const BOOTSTRAP_EMPTY: u8 = 0;
const BOOTSTRAP_INITIALIZING: u8 = 1;
const BOOTSTRAP_STAGED: u8 = 2;
const BOOTSTRAP_CONSUMED: u8 = 3;

struct HotReloadBootstrap {
    // The queue owns the staged value; the atomic publishes which transition
    // is currently visible to callers without borrowing shared mutable state.
    state: AtomicU8,
    staged: SegQueue<StagedHostConfig>,
}

static HOT_RELOAD_CONFIG: OnceLock<HotReloadBootstrap> = OnceLock::new();

fn hot_reload_config() -> &'static HotReloadBootstrap {
    HOT_RELOAD_CONFIG.get_or_init(|| HotReloadBootstrap {
        state: AtomicU8::new(BOOTSTRAP_EMPTY),
        staged: SegQueue::new(),
    })
}

struct StagedHostConfig {
    address: SocketAddr,
    credentials: SessionCredentials,
    development: DevelopmentHostConfig,
}

/// Reads and stages the private development-host launch configuration.
///
/// A normal native launch carries none of the four reload values and succeeds
/// without staging a host. A reload launch must provide all four values in
/// their strict canonical forms. Any partial, malformed, unknown, or oversized
/// input fails closed and emits only a generic diagnostic that cannot disclose
/// credentials or configuration text.
///
/// Desktop and Apple targets consume private environment variables. Android
/// consumes `files/aimer_reload_session` beneath the application's private data
/// directory after the generated entry point has stored `ANDROID_APP`. Launch
/// input is removed from its source immediately after the bounded read.
///
/// The synchronization boundary permits at most one configuration to be staged
/// and consumed in a process, while keeping the non-`Send` live host policy out
/// of process-global storage.
pub fn initialize_hot_reload_host() -> bool {
    let initialized = initialize_hot_reload_host_inner();
    if initialized.is_err() {
        aimer_utils::error!("Aimer hot reload bootstrap rejected its private launch configuration");
    }
    initialized.is_ok()
}

fn initialize_hot_reload_host_inner() -> Result<(), ()> {
    let bootstrap = hot_reload_config();
    if bootstrap
        .state
        .compare_exchange(
            BOOTSTRAP_EMPTY,
            BOOTSTRAP_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return Ok(());
    }

    match read_launch_config() {
        Ok(Some(config)) => {
            bootstrap.staged.push(config);
            bootstrap.state.store(BOOTSTRAP_STAGED, Ordering::Release);
            Ok(())
        }
        Ok(None) => {
            bootstrap.state.store(BOOTSTRAP_EMPTY, Ordering::Release);
            Ok(())
        }
        Err(()) => {
            bootstrap.state.store(BOOTSTRAP_EMPTY, Ordering::Release);
            Err(())
        }
    }
}

/// Takes the staged launch configuration for the first `AimerApp` consumer.
///
/// The returned address is always target-local loopback. The live policy is
/// constructed only on the consuming application thread because it contains
/// local capability state that is intentionally not `Send`. Calling this more
/// than once returns `None`, including when the first call observed a normal
/// launch with no staged reload host.
pub fn take_hot_reload_config() -> Option<(SocketAddr, SessionCredentials, LiveReloadConfig)> {
    let bootstrap = hot_reload_config();
    let state = bootstrap.state.load(Ordering::Acquire);
    let staged = match state {
        BOOTSTRAP_STAGED
            if bootstrap
                .state
                .compare_exchange(
                    BOOTSTRAP_STAGED,
                    BOOTSTRAP_CONSUMED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok() =>
        {
            bootstrap.staged.pop()?
        }
        BOOTSTRAP_EMPTY
            if bootstrap
                .state
                .compare_exchange(
                    BOOTSTRAP_EMPTY,
                    BOOTSTRAP_CONSUMED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok() =>
        {
            return None;
        }
        BOOTSTRAP_CONSUMED | BOOTSTRAP_INITIALIZING | BOOTSTRAP_EMPTY | BOOTSTRAP_STAGED => {
            return None;
        }
        _ => return None,
    };
    Some(staged.into_live_reload_config())
}

impl StagedHostConfig {
    fn into_live_reload_config(self) -> (SocketAddr, SessionCredentials, LiveReloadConfig) {
        (
            self.address,
            self.credentials,
            HostPolicy::from(self.development).into_live_reload_config(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HostPolicy {
    runtime_fuel_per_call: u64,
    module_bytes: usize,
    runtime_memory_pages: u32,
    runtime_table_elements: usize,
    runtime_call_depth: usize,
    protocol_chunk_bytes: usize,
    protocol_diagnostic_bytes: usize,
    protocol_terminal_results: usize,
    protocol_io_timeout_ms: u64,
    listener_credential_ttl_ms: u64,
    listener_authentication_failures: u32,
    listener_authentication_backoff_ms: u64,
    model_document_bytes: u32,
    model_collection_entries: u32,
    model_widget_depth: u32,
    callback_bindings: u32,
    state_transfer_migration_fuel: u64,
    retained_generations: usize,
    reload_command_queue_capacity: usize,
    event_queue_capacity: usize,
    widget_ir_diagnostics: bool,
}

impl From<DevelopmentHostConfig> for HostPolicy {
    fn from(config: DevelopmentHostConfig) -> Self {
        let model_collection_entries = config
            .model_state_entry_limit()
            .min(config.model_widget_node_limit())
            .min(config.model_property_limit());
        Self {
            runtime_fuel_per_call: config.runtime_fuel_per_call_limit(),
            module_bytes: config.module_bytes_limit(),
            runtime_memory_pages: config.runtime_memory_pages_limit(),
            runtime_table_elements: config.runtime_table_elements_limit() as usize,
            runtime_call_depth: config.runtime_call_depth_limit() as usize,
            protocol_chunk_bytes: config.protocol_chunk_bytes_limit(),
            protocol_diagnostic_bytes: config.protocol_diagnostic_bytes_limit(),
            protocol_terminal_results: config.protocol_terminal_result_limit(),
            protocol_io_timeout_ms: config.protocol_io_timeout_ms_limit(),
            listener_credential_ttl_ms: config.listener_credential_ttl_ms_limit(),
            listener_authentication_failures: config.listener_authentication_failure_limit(),
            listener_authentication_backoff_ms: config.listener_authentication_backoff_ms_limit(),
            model_document_bytes: config.model_document_bytes_limit() as u32,
            model_collection_entries,
            model_widget_depth: config.model_widget_depth_limit(),
            callback_bindings: config.callback_binding_limit(),
            state_transfer_migration_fuel: config.state_transfer_migration_fuel_limit(),
            // The host drops every retired generation after commit. Zero is a
            // stricter upper bound than every validated profile, whose minimum
            // accepted retained-generation ceiling is one.
            retained_generations: 0,
            // The authenticated command bridge is fixed at one item. One is no
            // looser than every validated profile's positive queue ceiling.
            reload_command_queue_capacity: 1,
            event_queue_capacity: config.event_queue_capacity_limit(),
            widget_ir_diagnostics: config.widget_ir_diagnostics_enabled(),
        }
    }
}

impl HostPolicy {
    fn into_live_reload_config(self) -> LiveReloadConfig {
        let runtime = RuntimeConfig::new()
            .fuel_per_call(self.runtime_fuel_per_call)
            .max_module_bytes(self.module_bytes)
            .max_memory_pages(self.runtime_memory_pages)
            .max_table_elements(self.runtime_table_elements)
            .max_call_depth(self.runtime_call_depth);
        let protocol = ProtocolLimits::new(
            self.module_bytes,
            Duration::from_millis(self.protocol_io_timeout_ms),
        )
        .max_chunk_bytes(self.protocol_chunk_bytes)
        .max_diagnostic_bytes(self.protocol_diagnostic_bytes)
        .max_terminal_results(self.protocol_terminal_results);
        let security = ListenerSecurity::new(
            Duration::from_millis(self.listener_credential_ttl_ms),
            self.listener_authentication_failures,
            Duration::from_millis(self.listener_authentication_backoff_ms),
        );

        // `ModelLimits` has one collection ceiling shared by state entries,
        // widget nodes, and properties. Applying the minimum of the three host
        // ceilings therefore enforces every upper bound and can only be stricter
        // for the two larger collections. Strings and blobs are bounded by the
        // complete model document ceiling, so neither can exceed that allocation.
        let model = ModelLimits::new(
            self.model_document_bytes,
            self.model_collection_entries,
            self.model_document_bytes,
            self.model_document_bytes,
        )
        .max_widget_depth(self.model_widget_depth);
        // DevelopmentHostConfig currently defines no generation-owned resource
        // allowance. A zero per-kind limit is the fail-closed configuration and
        // is independent of the retired-generation ceiling enforced above.
        let generation = GenerationLimits::new(0);
        let candidate = ReloadCandidateLimits::new(model, generation, self.callback_bindings);
        let state_transfer = StateTransferCoordinator::new()
            .model_limits(model)
            .migration_fuel(self.state_transfer_migration_fuel);

        debug_assert_eq!(self.retained_generations, 0);
        debug_assert_eq!(self.reload_command_queue_capacity, 1);
        LiveReloadConfig::new(runtime, protocol, security, candidate)
            .state_transfer(state_transfer)
            .max_queued_events(self.event_queue_capacity)
            .widget_ir_diagnostics(self.widget_ir_diagnostics)
    }
}

#[cfg(not(target_os = "android"))]
fn read_launch_config() -> Result<Option<StagedHostConfig>, ()> {
    let values = [
        std::env::var_os(SESSION_ID_VARIABLE),
        std::env::var_os(SESSION_TOKEN_VARIABLE),
        std::env::var_os(LISTENER_PORT_VARIABLE),
        std::env::var_os(HOST_CONFIG_VARIABLE),
    ];

    // SAFETY: the generated native entry point calls this boundary before user
    // application code or Aimer starts any thread. No concurrent environment
    // access exists at this process-startup point.
    unsafe {
        std::env::remove_var(SESSION_ID_VARIABLE);
        std::env::remove_var(SESSION_TOKEN_VARIABLE);
        std::env::remove_var(LISTENER_PORT_VARIABLE);
        std::env::remove_var(HOST_CONFIG_VARIABLE);
    }

    let values = values
        .map(|value| value.map(|value| value.into_string()).transpose())
        .map(|value| value.map_err(|_| ()))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let staged = parse_launch_values([
        values[0].as_deref(),
        values[1].as_deref(),
        values[2].as_deref(),
        values[3].as_deref(),
    ])?;
    #[cfg(target_os = "ios")]
    {
        let mut staged = staged;
        let service_name = std::env::var_os(IOS_SERVICE_NAME_VARIABLE);
        // SAFETY: this remains inside the process-startup boundary documented
        // above, before Aimer or application code starts any thread.
        unsafe {
            std::env::remove_var(IOS_SERVICE_NAME_VARIABLE);
        }
        let service_name = service_name
            .map(|value| value.into_string().map_err(|_| ()))
            .transpose()?;
        if let Some(config) = staged.as_mut() {
            config
                .address
                .set_ip(ios_listener_ip(service_name.as_deref())?.into());
        } else if service_name.is_some() {
            return Err(());
        }
        return Ok(staged);
    }
    #[cfg(not(target_os = "ios"))]
    Ok(staged)
}

#[cfg(any(target_os = "ios", test))]
fn ios_listener_ip(service_name: Option<&str>) -> Result<Ipv4Addr, ()> {
    let Some(service_name) = service_name else {
        return Ok(Ipv4Addr::LOCALHOST);
    };
    if service_name.is_empty()
        || service_name.len() > 128
        || !service_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(());
    }
    Ok(Ipv4Addr::UNSPECIFIED)
}

#[cfg(target_os = "android")]
fn read_launch_config() -> Result<Option<StagedHostConfig>, ()> {
    use std::fs::{self, File};
    use std::io::{self, Read};

    const SESSION_FILE: &str = "files/aimer_reload_session";
    const MAX_SESSION_FILE_BYTES: usize = SESSION_ID_HEX_BYTES
        + 1
        + SESSION_TOKEN_HEX_BYTES
        + 1
        + 5
        + 1
        + MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES;

    let data_path = aimer_platform::ANDROID_APP
        .get()
        .and_then(|app| app.internal_data_path())
        .ok_or(())?;
    let session_path = data_path.join(SESSION_FILE);
    let file = match File::open(&session_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let mut bytes = Vec::with_capacity(MAX_SESSION_FILE_BYTES + 1);
    let read_result = file
        .take((MAX_SESSION_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes);
    let remove_result = fs::remove_file(&session_path);
    read_result.map_err(|_| ())?;
    remove_result.map_err(|_| ())?;
    if bytes.len() > MAX_SESSION_FILE_BYTES {
        return Err(());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| ())?;
    parse_android_session(text)
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn parse_android_session(text: &str) -> Result<Option<StagedHostConfig>, ()> {
    let mut fields = text.splitn(4, '\n');
    parse_launch_values([fields.next(), fields.next(), fields.next(), fields.next()])
}

fn parse_launch_values(values: [Option<&str>; 4]) -> Result<Option<StagedHostConfig>, ()> {
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let [Some(session_id), Some(token), Some(port), Some(host_config)] = values else {
        return Err(());
    };
    if host_config.len() > MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES {
        return Err(());
    }

    let session_id = decode_fixed_hex::<16>(session_id, SESSION_ID_HEX_BYTES)?;
    let token = decode_fixed_hex::<32>(token, SESSION_TOKEN_HEX_BYTES)?;
    let port = parse_listener_port(port)?;
    let development = DevelopmentHostConfig::from_text(host_config).map_err(|_| ())?;
    Ok(Some(StagedHostConfig {
        address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        credentials: SessionCredentials::from_parts(session_id, token),
        development,
    }))
}

fn decode_fixed_hex<const N: usize>(text: &str, encoded_len: usize) -> Result<[u8; N], ()> {
    if text.len() != encoded_len {
        return Err(());
    }
    let mut decoded = [0_u8; N];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (decode_hex_nibble(pair[0])? << 4) | decode_hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn decode_hex_nibble(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

fn parse_listener_port(text: &str) -> Result<u16, ()> {
    if text.is_empty()
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(());
    }
    let port = text.parse::<u16>().map_err(|_| ())?;
    if port == 0 {
        return Err(());
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION_ID: &str = "11111111111111111111111111111111";
    const TOKEN: &str = concat!(
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
    );

    fn host_config() -> String {
        DevelopmentHostConfig::cli_safe_profile().to_text().unwrap()
    }

    #[test]
    fn absent_launch_values_are_a_normal_launch() {
        assert!(
            parse_launch_values([None, None, None, None])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn only_a_valid_physical_ios_service_selects_a_non_loopback_bind() {
        assert_eq!(ios_listener_ip(None), Ok(Ipv4Addr::LOCALHOST));
        assert_eq!(
            ios_listener_ip(Some("aimer-reload-0123456789abcdef")),
            Ok(Ipv4Addr::UNSPECIFIED)
        );
        assert!(ios_listener_ip(Some("")).is_err());
        assert!(ios_listener_ip(Some("invalid.service")).is_err());
    }

    #[test]
    fn launch_values_are_all_or_nothing() {
        let config = host_config();
        for missing in 0..4 {
            let mut values = [
                Some(SESSION_ID),
                Some(TOKEN),
                Some("37654"),
                Some(config.as_str()),
            ];
            values[missing] = None;
            assert!(parse_launch_values(values).is_err());
        }
    }

    #[test]
    fn complete_launch_values_decode_strict_credentials_port_and_config() {
        let config = host_config();
        assert!(decode_fixed_hex::<16>(SESSION_ID, SESSION_ID_HEX_BYTES).is_ok());
        assert!(decode_fixed_hex::<32>(TOKEN, SESSION_TOKEN_HEX_BYTES).is_ok());
        assert_eq!(parse_listener_port("37654"), Ok(37654));
        assert!(DevelopmentHostConfig::from_text(&config).is_ok());
        let staged = parse_launch_values([
            Some(SESSION_ID),
            Some(TOKEN),
            Some("37654"),
            Some(config.as_str()),
        ])
        .unwrap()
        .unwrap();
        let (session_id, token) = staged.credentials.launch_environment_hex();

        assert_eq!(staged.address, "127.0.0.1:37654".parse().unwrap());
        assert_eq!(session_id.as_str(), SESSION_ID);
        assert_eq!(token.as_str(), TOKEN);
        assert_eq!(
            staged.development,
            DevelopmentHostConfig::cli_safe_profile()
        );
    }

    #[test]
    fn malformed_credentials_and_ports_fail_closed() {
        let config = host_config();
        for values in [
            [
                Some("11"),
                Some(TOKEN),
                Some("37654"),
                Some(config.as_str()),
            ],
            [
                Some("gggggggggggggggggggggggggggggggg"),
                Some(TOKEN),
                Some("37654"),
                Some(config.as_str()),
            ],
            [
                Some(SESSION_ID),
                Some("a5"),
                Some("37654"),
                Some(config.as_str()),
            ],
            [
                Some(SESSION_ID),
                Some(TOKEN),
                Some("0"),
                Some(config.as_str()),
            ],
            [
                Some(SESSION_ID),
                Some(TOKEN),
                Some("037654"),
                Some(config.as_str()),
            ],
            [
                Some(SESSION_ID),
                Some(TOKEN),
                Some("65536"),
                Some(config.as_str()),
            ],
        ] {
            assert!(parse_launch_values(values).is_err());
        }
    }

    #[test]
    fn malformed_unknown_and_oversized_host_configs_fail_closed() {
        let malformed = "version=1\n";
        let unknown = format!("{}unknown=1\n", host_config());
        let oversized = "x".repeat(MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES + 1);
        for config in [malformed, unknown.as_str(), oversized.as_str()] {
            assert!(
                parse_launch_values([Some(SESSION_ID), Some(TOKEN), Some("37654"), Some(config),])
                    .is_err()
            );
        }
    }

    #[test]
    fn android_session_uses_the_same_four_value_boundary() {
        let text = format!("{SESSION_ID}\n{TOKEN}\n37654\n{}", host_config());
        let staged = parse_android_session(&text).unwrap().unwrap();

        assert_eq!(staged.address, "127.0.0.1:37654".parse().unwrap());
        assert!(parse_android_session("only-one-line").is_err());
    }

    #[test]
    fn policy_maps_every_ceiling_without_becoming_looser() {
        let config = DevelopmentHostConfig::cli_safe_profile()
            .model_max_state_entries(23)
            .model_max_widget_nodes(17)
            .model_max_widget_depth(17)
            .model_max_properties(29)
            .generation_max_retired(9)
            .reload_command_queue_capacity(7)
            .validate()
            .unwrap();
        let policy = HostPolicy::from(config);

        assert_eq!(
            policy.runtime_fuel_per_call,
            config.runtime_fuel_per_call_limit()
        );
        assert_eq!(policy.module_bytes, config.module_bytes_limit());
        assert_eq!(
            policy.runtime_memory_pages,
            config.runtime_memory_pages_limit()
        );
        assert_eq!(
            policy.runtime_table_elements,
            config.runtime_table_elements_limit() as usize
        );
        assert_eq!(
            policy.runtime_call_depth,
            config.runtime_call_depth_limit() as usize
        );
        assert_eq!(
            policy.protocol_chunk_bytes,
            config.protocol_chunk_bytes_limit()
        );
        assert_eq!(
            policy.protocol_diagnostic_bytes,
            config.protocol_diagnostic_bytes_limit()
        );
        assert_eq!(
            policy.protocol_terminal_results,
            config.protocol_terminal_result_limit()
        );
        assert_eq!(
            policy.protocol_io_timeout_ms,
            config.protocol_io_timeout_ms_limit()
        );
        assert_eq!(
            policy.listener_credential_ttl_ms,
            config.listener_credential_ttl_ms_limit()
        );
        assert_eq!(
            policy.listener_authentication_failures,
            config.listener_authentication_failure_limit()
        );
        assert_eq!(
            policy.listener_authentication_backoff_ms,
            config.listener_authentication_backoff_ms_limit()
        );
        assert_eq!(
            policy.model_document_bytes,
            config.model_document_bytes_limit() as u32
        );
        assert_eq!(policy.model_collection_entries, 17);
        assert_eq!(policy.model_widget_depth, config.model_widget_depth_limit());
        assert_eq!(policy.callback_bindings, config.callback_binding_limit());
        assert_eq!(
            policy.state_transfer_migration_fuel,
            config.state_transfer_migration_fuel_limit()
        );
        assert!(policy.retained_generations <= config.retired_generation_limit());
        assert!(
            policy.reload_command_queue_capacity <= config.reload_command_queue_capacity_limit()
        );
        assert_eq!(
            policy.event_queue_capacity,
            config.event_queue_capacity_limit()
        );
    }
}
