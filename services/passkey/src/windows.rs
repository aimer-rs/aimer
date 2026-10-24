//! Windows Hello passkeys through `webauthn.dll`.
//!
//! Both ceremonies are blocking calls that own a modal sheet until the user
//! finishes, so they run on Venus's offload pool through one ordered lane. The
//! window the sheet belongs to is found on the calling (UI) thread, where
//! `GetActiveWindow` is meaningful, and handed to the worker as a number.
//!
//! `webauthn.dll` does not build the client data, so this adapter does, with the
//! origin `https://<relying party id>`. Windows also does not check that the
//! application is associated with the domain: the server is the only judge of
//! the origin.

use std::ffi::c_void;
use std::future::Future;
use std::marker::PhantomData;
use std::mem;
use std::pin::Pin;
use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use aimer_venus::Lane;
use windows::Win32::Foundation::HWND;
use windows::Win32::Networking::WindowsWebServices::{
    WEBAUTHN_API_VERSION_4, WEBAUTHN_ASSERTION, WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT,
    WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
    WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
    WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
    WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_4,
    WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
    WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_3, WEBAUTHN_CLIENT_DATA,
    WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETER,
    WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETERS,
    WEBAUTHN_CREDENTIAL_ATTESTATION, WEBAUTHN_CREDENTIAL_EX,
    WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION, WEBAUTHN_CREDENTIAL_LIST,
    WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY, WEBAUTHN_CTAP_TRANSPORT_FLAGS_MASK,
    WEBAUTHN_HASH_ALGORITHM_SHA_256, WEBAUTHN_RP_ENTITY_INFORMATION,
    WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION, WEBAUTHN_USER_ENTITY_INFORMATION,
    WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED, WebAuthNAuthenticatorGetAssertion,
    WebAuthNAuthenticatorMakeCredential, WebAuthNCancelCurrentOperation, WebAuthNFreeAssertion,
    WebAuthNFreeCredentialAttestation, WebAuthNGetApiVersionNumber, WebAuthNGetCancellationId,
    WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{GUID, PCWSTR};

use crate::client_data::{self, Ceremony};
use crate::error::{PasskeyCapability, PasskeyError, PasskeyResult};
use crate::passkeys::{PasskeyFuture, Passkeys, ready};
use crate::request::{Attestation, CreateRequest, CredentialId, GetRequest, UserVerification};
use crate::response::{Assertion, Registration};
use crate::status::from_hresult;

/// Passkeys through Windows Hello.
///
/// Needs the `webauthn.dll` of a Windows 10 or 11 build that implements WebAuthn
/// API version 4; older builds report themselves unavailable. The system sheet
/// offers whatever the machine has: Windows Hello, a phone, or a security key.
#[derive(Clone, Default)]
pub struct WindowsHelloPasskeys {
    lane: Lane,
}

impl WindowsHelloPasskeys {
    /// Creates the adapter.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `work` on the lane, translating the lane's failures.
    fn run<T, F>(&self, work: F) -> PasskeyFuture<T>
    where
        T: Send + 'static,
        F: FnOnce() -> PasskeyResult<T> + Send + 'static,
    {
        match self.lane.submit(move |_| work()) {
            Some(completion) => {
                Box::pin(async move { completion.await.unwrap_or(Err(PasskeyError::Interrupted)) })
            }
            None => ready(Err(PasskeyError::Unavailable {
                capability: PasskeyCapability::Runtime,
            })),
        }
    }
}

impl Passkeys for WindowsHelloPasskeys {
    fn is_available(&self) -> PasskeyFuture<bool> {
        self.run(|| {
            // SAFETY: neither call takes a pointer.
            let supported = unsafe { WebAuthNGetApiVersionNumber() } >= WEBAUTHN_API_VERSION_4;
            Ok(supported
                && unsafe { WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable() }
                    .is_ok_and(|available| available.as_bool()))
        })
    }

    fn create(&self, request: CreateRequest) -> PasskeyFuture<Registration> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        let pending = match Pending::begin() {
            Ok(pending) => pending,
            Err(error) => return ready(Err(error)),
        };
        let (window, cancellation, abandoned) = pending.parts();
        let work = self.run(move || {
            if abandoned.load(Ordering::Acquire) {
                return Err(PasskeyError::Cancelled);
            }
            make_credential(window, cancellation, &request)
        });
        Box::pin(pending.guard(work))
    }

    fn get(&self, request: GetRequest) -> PasskeyFuture<Assertion> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        let pending = match Pending::begin() {
            Ok(pending) => pending,
            Err(error) => return ready(Err(error)),
        };
        let (window, cancellation, abandoned) = pending.parts();
        let work = self.run(move || {
            if abandoned.load(Ordering::Acquire) {
                return Err(PasskeyError::Cancelled);
            }
            get_assertion(window, cancellation, &request)
        });
        Box::pin(pending.guard(work))
    }
}

/// What the UI thread contributes before a ceremony moves to a worker: the
/// window to parent the sheet to, and an id that can cancel it.
struct Pending {
    /// An `HWND`, as a number so it can cross to the worker.
    window: usize,
    cancellation: GUID,
    abandoned: Arc<AtomicBool>,
}

impl Pending {
    fn begin() -> PasskeyResult<Self> {
        // SAFETY: the call takes no pointer.
        if unsafe { WebAuthNGetApiVersionNumber() } < WEBAUTHN_API_VERSION_4 {
            return Err(PasskeyError::Unavailable {
                capability: PasskeyCapability::PlatformAuthenticator,
            });
        }
        // SAFETY: `begin` runs on the UI thread that owns the window.
        let window = unsafe { own_window() }.ok_or(PasskeyError::Unavailable {
            capability: PasskeyCapability::Window,
        })?;
        // SAFETY: no arguments; the id is a plain value.
        let cancellation = unsafe { WebAuthNGetCancellationId() }
            .map_err(|error| from_hresult(error.code().0))?;
        Ok(Self {
            window: window.0 as usize,
            cancellation,
            abandoned: Arc::new(AtomicBool::new(false)),
        })
    }

    /// What the worker needs.
    fn parts(&self) -> (usize, GUID, Arc<AtomicBool>) {
        (self.window, self.cancellation, Arc::clone(&self.abandoned))
    }

    fn guard<F>(self, work: F) -> Guarded<F> {
        Guarded {
            work,
            cancellation: self.cancellation,
            abandoned: self.abandoned,
            finished: false,
        }
    }
}

/// The application's window on this thread, else its foreground window.
///
/// # Safety
///
/// Must be called on the UI thread that owns the window.
unsafe fn own_window() -> Option<HWND> {
    // SAFETY: the only pointer is the out-parameter below, which points at a live
    // local.
    unsafe {
        let active = GetActiveWindow();
        if !active.is_invalid() {
            return Some(active);
        }
        // The foreground window may belong to another process; parenting the sheet
        // to it would put it in front of someone else's app.
        let foreground = GetForegroundWindow();
        let mut process = 0;
        GetWindowThreadProcessId(foreground, Some(&mut process));
        (!foreground.is_invalid() && process == GetCurrentProcessId()).then_some(foreground)
    }
}

/// A ceremony future that cancels the system sheet if it is dropped early.
struct Guarded<F> {
    work: F,
    cancellation: GUID,
    abandoned: Arc<AtomicBool>,
    finished: bool,
}

impl<T, F> Future for Guarded<F>
where
    F: Future<Output = PasskeyResult<T>> + Unpin,
{
    type Output = PasskeyResult<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let poll = Pin::new(&mut self.work).poll(context);
        if poll.is_ready() {
            self.finished = true;
        }
        poll
    }
}

impl<F> Drop for Guarded<F> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // A job still waiting in the lane sees the flag and never opens a sheet; one
        // already showing it is told to close.
        self.abandoned.store(true, Ordering::Release);
        // SAFETY: the id is a live local; cancelling an id that is not in flight is
        // a no-op.
        let _ = unsafe { WebAuthNCancelCurrentOperation(&self.cancellation) };
    }
}

/// UTF-16, NUL-terminated.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn user_verification(preference: UserVerification) -> u32 {
    match preference {
        UserVerification::Required => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        UserVerification::Preferred => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
        UserVerification::Discouraged => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
    }
}

/// Windows has no enterprise tier: it is requested as direct attestation.
fn attestation(preference: Attestation) -> u32 {
    match preference {
        Attestation::None => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        Attestation::Indirect => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
        Attestation::Direct | Attestation::Enterprise => {
            WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT
        }
    }
}

/// Credential ids laid out the way `WEBAUTHN_CREDENTIAL_LIST` wants them.
///
/// The entries point into the borrowed ids and the list points into the entries,
/// so the list is only valid while this value is alive. Moving the value is fine:
/// the pointers are to the vectors' heap buffers, which neither move nor grow.
struct CredentialList<'a> {
    entries: Vec<WEBAUTHN_CREDENTIAL_EX>,
    pointers: Vec<*mut WEBAUTHN_CREDENTIAL_EX>,
    ids: PhantomData<&'a [CredentialId]>,
}

impl<'a> CredentialList<'a> {
    fn new(ids: &'a [CredentialId]) -> Self {
        let mut entries: Vec<WEBAUTHN_CREDENTIAL_EX> = ids
            .iter()
            .map(|id| WEBAUTHN_CREDENTIAL_EX {
                dwVersion: WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION,
                cbId: id.as_bytes().len() as u32,
                // The API only reads the id.
                pbId: id.as_bytes().as_ptr().cast_mut(),
                pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
                dwTransports: WEBAUTHN_CTAP_TRANSPORT_FLAGS_MASK,
            })
            .collect();
        let pointers = entries.iter_mut().map(|entry| entry as *mut _).collect();
        Self {
            entries,
            pointers,
            ids: PhantomData,
        }
    }

    fn raw(&mut self) -> WEBAUTHN_CREDENTIAL_LIST {
        debug_assert_eq!(self.entries.len(), self.pointers.len());
        WEBAUTHN_CREDENTIAL_LIST {
            cCredentials: self.pointers.len() as u32,
            ppCredentials: self.pointers.as_mut_ptr(),
        }
    }
}

fn make_credential(
    window: usize,
    mut cancellation: GUID,
    request: &CreateRequest,
) -> PasskeyResult<Registration> {
    let client_data_json = client_data::build(
        Ceremony::Create,
        &request.challenge,
        &request.relying_party.id,
    );
    let rp_id = wide(&request.relying_party.id);
    let rp_name = wide(&request.relying_party.name);
    let user_name = wide(&request.user.name);
    let user_display_name = wide(&request.user.display_name);

    let rp = WEBAUTHN_RP_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
        pwszId: PCWSTR(rp_id.as_ptr()),
        pwszName: PCWSTR(rp_name.as_ptr()),
        pwszIcon: PCWSTR::null(),
    };
    let user = WEBAUTHN_USER_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
        cbId: request.user.id.len() as u32,
        pbId: request.user.id.as_ptr().cast_mut(),
        pwszName: PCWSTR(user_name.as_ptr()),
        pwszIcon: PCWSTR::null(),
        pwszDisplayName: PCWSTR(user_display_name.as_ptr()),
    };
    let mut algorithms: Vec<WEBAUTHN_COSE_CREDENTIAL_PARAMETER> = request
        .algorithms
        .iter()
        .map(|algorithm| WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: algorithm.cose_id(),
        })
        .collect();
    let parameters = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
        cCredentialParameters: algorithms.len() as u32,
        pCredentialParameters: algorithms.as_mut_ptr(),
    };
    let client_data = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data_json.len() as u32,
        pbClientDataJSON: client_data_json.as_ptr().cast_mut(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };

    let mut exclude = CredentialList::new(&request.exclude);
    let mut exclude_list = exclude.raw();

    // SAFETY: every field is an integer, a pointer, or a struct of them, for which
    // all-zero is the documented "not set".
    let mut options: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS = unsafe { mem::zeroed() };
    options.dwVersion = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_3;
    options.dwTimeoutMilliseconds = request.timeout_millis();
    options.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY;
    // A passkey is a discoverable credential.
    options.bRequireResidentKey = true.into();
    options.dwUserVerificationRequirement = user_verification(request.user_verification);
    options.dwAttestationConveyancePreference = attestation(request.attestation);
    options.pCancellationId = &mut cancellation;
    if !request.exclude.is_empty() {
        options.pExcludeCredentialList = &mut exclude_list;
    }

    // SAFETY: every pointer refers to a local that lives until the call returns,
    // and the options version selects only fields initialized above.
    let attestation = unsafe {
        WebAuthNAuthenticatorMakeCredential(
            HWND(window as *mut c_void),
            &rp,
            &user,
            &parameters,
            &client_data,
            Some(&options),
        )
    }
    .map_err(|error| from_hresult(error.code().0))?;

    let attestation = Owned::<WEBAUTHN_CREDENTIAL_ATTESTATION>::new(attestation, |pointer| {
        // SAFETY: `pointer` came from `WebAuthNAuthenticatorMakeCredential` and is
        // freed exactly once, by the owner's drop.
        unsafe { WebAuthNFreeCredentialAttestation(Some(pointer.cast_const())) }
    });
    let attestation = attestation.get();
    // SAFETY: each pointer/length pair describes memory the attestation owns, which
    // stays valid until the owner drops.
    Ok(unsafe {
        Registration {
            credential_id: bytes(attestation.pbCredentialId, attestation.cbCredentialId),
            client_data_json,
            attestation_object: bytes(
                attestation.pbAttestationObject,
                attestation.cbAttestationObject,
            ),
        }
    })
}

fn get_assertion(
    window: usize,
    mut cancellation: GUID,
    request: &GetRequest,
) -> PasskeyResult<Assertion> {
    let client_data_json = client_data::build(Ceremony::Get, &request.challenge, &request.rp_id);
    let rp_id = wide(&request.rp_id);
    let client_data = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data_json.len() as u32,
        pbClientDataJSON: client_data_json.as_ptr().cast_mut(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };

    let mut allow = CredentialList::new(&request.allow);
    let mut allow_list = allow.raw();

    // SAFETY: as for the make-credential options, all-zero means "not set".
    let mut options: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS = unsafe { mem::zeroed() };
    options.dwVersion = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_4;
    options.dwTimeoutMilliseconds = request.timeout_millis();
    options.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY;
    options.dwUserVerificationRequirement = user_verification(request.user_verification);
    options.pCancellationId = &mut cancellation;
    if !request.allow.is_empty() {
        options.pAllowCredentialList = &mut allow_list;
    }

    // SAFETY: every pointer refers to a local that lives until the call returns,
    // and the options version selects only fields initialized above.
    let assertion = unsafe {
        WebAuthNAuthenticatorGetAssertion(
            HWND(window as *mut c_void),
            PCWSTR(rp_id.as_ptr()),
            &client_data,
            Some(&options),
        )
    }
    .map_err(|error| from_hresult(error.code().0))?;

    let assertion = Owned::<WEBAUTHN_ASSERTION>::new(assertion, |pointer| {
        // SAFETY: `pointer` came from `WebAuthNAuthenticatorGetAssertion` and is
        // freed exactly once, by the owner's drop.
        unsafe { WebAuthNFreeAssertion(pointer.cast_const()) }
    });
    let assertion = assertion.get();
    // SAFETY: as for the attestation above.
    Ok(unsafe {
        Assertion {
            credential_id: bytes(assertion.Credential.pbId, assertion.Credential.cbId),
            client_data_json,
            authenticator_data: bytes(assertion.pbAuthenticatorData, assertion.cbAuthenticatorData),
            signature: bytes(assertion.pbSignature, assertion.cbSignature),
            // Windows reports a missing handle as an empty one.
            user_handle: Some(bytes(assertion.pbUserId, assertion.cbUserId))
                .filter(|handle| !handle.is_empty()),
        }
    })
}

/// A structure the WebAuthn API allocated, freed with the API's own function.
struct Owned<T> {
    pointer: *mut T,
    free: fn(*mut T),
}

impl<T> Owned<T> {
    /// Takes ownership of `pointer`, which must be non-null.
    fn new(pointer: *mut T, free: fn(*mut T)) -> Self {
        debug_assert!(!pointer.is_null());
        Self { pointer, free }
    }

    fn get(&self) -> &T {
        // SAFETY: non-null by construction and valid until `free` runs in drop.
        unsafe { &*self.pointer }
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        (self.free)(self.pointer);
    }
}

/// Copies `len` bytes at `pointer`; a null pointer or zero length is empty.
///
/// # Safety
///
/// A non-null `pointer` must be valid for reads of `len` bytes.
unsafe fn bytes(pointer: *const u8, len: u32) -> Vec<u8> {
    if pointer.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: guaranteed by the caller.
    unsafe { slice::from_raw_parts(pointer, len as usize) }.to_vec()
}
