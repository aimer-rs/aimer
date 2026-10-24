//! Passkeys through Apple's AuthenticationServices, on macOS and iOS.
//!
//! A ceremony is an `ASAuthorizationController` driven from the main thread. It
//! reports through a delegate, which completes a Venus [`Completion`] that the
//! returned future awaits, so the result is applied in the frame phase of the
//! awaiting task. The future owns the controller and the delegate: the controller
//! only holds its delegate weakly, so they live exactly as long as someone is
//! waiting, and dropping the future cancels the system sheet.
//!
//! The presentation anchor is the application's key window, found when the
//! ceremony starts, so the application never has to hand a window down.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use aimer_venus::{Completer, Completion, completion};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_protocol, msg_send, sel,
};
use objc2_authentication_services::{
    ASAuthorization, ASAuthorizationController, ASAuthorizationControllerDelegate,
    ASAuthorizationPlatformPublicKeyCredentialAssertion,
    ASAuthorizationPlatformPublicKeyCredentialDescriptor,
    ASAuthorizationPlatformPublicKeyCredentialProvider,
    ASAuthorizationPlatformPublicKeyCredentialRegistration, ASAuthorizationPublicKeyCredentialAssertion,
    ASAuthorizationPublicKeyCredentialAssertionRequest,
    ASAuthorizationPublicKeyCredentialAttestationKindDirect,
    ASAuthorizationPublicKeyCredentialAttestationKindEnterprise,
    ASAuthorizationPublicKeyCredentialAttestationKindIndirect,
    ASAuthorizationPublicKeyCredentialAttestationKindNone,
    ASAuthorizationPublicKeyCredentialRegistration,
    ASAuthorizationPublicKeyCredentialRegistrationRequest,
    ASAuthorizationPublicKeyCredentialUserVerificationPreferenceDiscouraged,
    ASAuthorizationPublicKeyCredentialUserVerificationPreferencePreferred,
    ASAuthorizationPublicKeyCredentialUserVerificationPreferenceRequired, ASAuthorizationRequest,
    ASPublicKeyCredential,
};
use objc2_foundation::{
    NSArray, NSData, NSError, NSOperatingSystemVersion, NSProcessInfo, NSString,
};

use crate::error::{PasskeyCapability, PasskeyError, PasskeyResult};
use crate::passkeys::{PasskeyFuture, Passkeys, ready};
use crate::request::{Attestation, CreateRequest, CredentialId, GetRequest, UserVerification};
use crate::response::{Assertion, Registration};
use crate::status::from_apple;

/// The first release with passkeys: macOS 13 Ventura, iOS 16.
#[cfg(target_os = "macos")]
const MINIMUM_MAJOR_VERSION: isize = 13;
#[cfg(target_os = "ios")]
const MINIMUM_MAJOR_VERSION: isize = 16;

/// Passkeys through AuthenticationServices: iCloud Keychain and any other
/// passkey provider the user enabled.
///
/// Apple only creates ES256 passkeys, so a request's algorithm list and timeout
/// are ignored. The application must be signed with the Associated Domains
/// entitlement (`webcredentials:<relying party id>`) and listed in the domain's
/// `apple-app-site-association`; otherwise a ceremony fails with
/// [`PasskeyError::NotAssociated`] or [`PasskeyError::MissingEntitlement`].
///
/// Ceremonies can only be started from the main thread, which is where Venus
/// runs tasks.
#[derive(Clone, Copy, Debug, Default)]
pub struct AuthenticationServicesPasskeys;

impl AuthenticationServicesPasskeys {
    /// Creates the adapter.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Passkeys for AuthenticationServicesPasskeys {
    fn is_available(&self) -> PasskeyFuture<bool> {
        let version = NSOperatingSystemVersion {
            majorVersion: MINIMUM_MAJOR_VERSION,
            minorVersion: 0,
            patchVersion: 0,
        };
        ready(Ok(
            NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(version)
        ))
    }

    fn create(&self, request: CreateRequest) -> PasskeyFuture<Registration> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        match begin(
            &request.relying_party.id,
            |mtm, provider| registration_request(mtm, provider, &request),
            Outcome::registration,
        ) {
            Ok(ceremony) => Box::pin(ceremony),
            Err(error) => ready(Err(error)),
        }
    }

    fn get(&self, request: GetRequest) -> PasskeyFuture<Assertion> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        match begin(
            &request.rp_id,
            |mtm, provider| assertion_request(mtm, provider, &request),
            Outcome::assertion,
        ) {
            Ok(ceremony) => Box::pin(ceremony),
            Err(error) => ready(Err(error)),
        }
    }
}

/// What a finished authorization carried.
enum Outcome {
    Registration(Registration),
    Assertion(Assertion),
}

impl Outcome {
    fn registration(self) -> PasskeyResult<Registration> {
        match self {
            Self::Registration(registration) => Ok(registration),
            Self::Assertion(_) => Err(PasskeyError::Failed),
        }
    }

    fn assertion(self) -> PasskeyResult<Assertion> {
        match self {
            Self::Assertion(assertion) => Ok(assertion),
            Self::Registration(_) => Err(PasskeyError::Failed),
        }
    }
}

struct Ivars {
    /// Taken by the first callback; a second callback finds nothing to complete.
    completer: RefCell<Option<Completer<PasskeyResult<Outcome>>>>,
    /// The window the system sheet is presented over.
    anchor: Retained<NSObject>,
}

extern_protocol!(
    /// The presentation half of `ASAuthorizationController`'s delegate pair.
    ///
    /// The bindings declare this protocol's one method on macOS only, because the
    /// anchor type differs per platform, but iOS asks for it too and
    /// `define_class!` only counts a method written in the protocol's own impl.
    /// Redeclaring the protocol under its real name, with the anchor as the plain
    /// `NSObject` both platforms' windows are, lets one definition serve both.
    // SAFETY (for implementors): mirrors the framework's protocol, so the method
    // must return a window object.
    #[allow(clippy::missing_safety_doc)]
    #[name = "ASAuthorizationControllerPresentationContextProviding"]
    unsafe trait PresentationContextProviding: NSObjectProtocol + MainThreadOnly {
        #[unsafe(method(presentationAnchorForAuthorizationController:))]
        #[unsafe(method_family = none)]
        unsafe fn presentation_anchor(
            &self,
            controller: &ASAuthorizationController,
        ) -> Retained<NSObject>;
    }
);

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AimerPasskeyDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl ASAuthorizationControllerDelegate for Delegate {
        #[unsafe(method(authorizationController:didCompleteWithAuthorization:))]
        fn did_complete_with_authorization(
            &self,
            _controller: &ASAuthorizationController,
            authorization: &ASAuthorization,
        ) {
            self.finish(outcome_of(authorization));
        }

        #[unsafe(method(authorizationController:didCompleteWithError:))]
        fn did_complete_with_error(&self, _controller: &ASAuthorizationController, error: &NSError) {
            self.finish(Err(error_of(error)));
        }
    }

    unsafe impl PresentationContextProviding for Delegate {
        #[unsafe(method_id(presentationAnchorForAuthorizationController:))]
        unsafe fn presentation_anchor(
            &self,
            _controller: &ASAuthorizationController,
        ) -> Retained<NSObject> {
            self.ivars().anchor.clone()
        }
    }
);

impl Delegate {
    fn new(
        mtm: MainThreadMarker,
        anchor: Retained<NSObject>,
        completer: Completer<PasskeyResult<Outcome>>,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars {
            completer: RefCell::new(Some(completer)),
            anchor,
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, result: PasskeyResult<Outcome>) {
        if let Some(completer) = self.ivars().completer.borrow_mut().take() {
            completer.complete(result);
        }
    }
}

/// Reads the credential an authorization produced.
fn outcome_of(authorization: &ASAuthorization) -> PasskeyResult<Outcome> {
    // SAFETY: a plain getter on a completed authorization.
    let credential = unsafe { authorization.credential() };
    let credential: &AnyObject = credential.as_ref();

    if let Some(registration) =
        credential.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialRegistration>()
    {
        // SAFETY: plain getters on a completed registration.
        return Ok(Outcome::Registration(unsafe {
            Registration {
                credential_id: registration.credentialID().to_vec(),
                client_data_json: registration.rawClientDataJSON().to_vec(),
                attestation_object: registration
                    .rawAttestationObject()
                    .map(|object| object.to_vec())
                    .unwrap_or_default(),
            }
        }));
    }
    if let Some(assertion) =
        credential.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialAssertion>()
    {
        // SAFETY: plain getters on a completed assertion.
        return Ok(Outcome::Assertion(unsafe {
            Assertion {
                credential_id: assertion.credentialID().to_vec(),
                client_data_json: assertion.rawClientDataJSON().to_vec(),
                authenticator_data: assertion.rawAuthenticatorData().to_vec(),
                signature: assertion.signature().to_vec(),
                user_handle: Some(assertion.userID().to_vec()).filter(|handle| !handle.is_empty()),
            }
        }));
    }
    // A password or Sign in with Apple credential cannot answer a passkey request.
    Err(PasskeyError::Failed)
}

fn error_of(error: &NSError) -> PasskeyError {
    // The specific reason ("not associated with domain ...") is in the full
    // description, not in `localizedDescription`.
    let message = format!("{error:?}");
    from_apple(&error.domain().to_string(), error.code() as i64, &message)
}

/// An authorization in flight. Dropping it before it finishes cancels the sheet.
struct Ceremony<T> {
    /// `None` once the result has been taken.
    session: Option<Session>,
    completion: Completion<PasskeyResult<Outcome>>,
    convert: fn(Outcome) -> PasskeyResult<T>,
}

/// What must stay alive while the system sheet is up.
struct Session {
    controller: Retained<ASAuthorizationController>,
    _delegate: Retained<Delegate>,
}

impl<T> Future for Ceremony<T> {
    type Output = PasskeyResult<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let ceremony = &mut *self;
        match Pin::new(&mut ceremony.completion).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                // Finished: nothing is left to cancel.
                ceremony.session = None;
                Poll::Ready(match result {
                    Ok(Ok(outcome)) => (ceremony.convert)(outcome),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(PasskeyError::Interrupted),
                })
            }
        }
    }
}

impl<T> Drop for Ceremony<T> {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            // SAFETY: `cancel` is documented as callable at any time; the main
            // thread is where the session was created and is dropped, because the
            // future is not `Send`.
            unsafe { session.controller.cancel() };
        }
    }
}

/// Starts a ceremony for the request `build` makes and returns the future that
/// resolves with its outcome.
fn begin<T>(
    rp_id: &str,
    build: impl FnOnce(
        MainThreadMarker,
        &ASAuthorizationPlatformPublicKeyCredentialProvider,
    ) -> Retained<ASAuthorizationRequest>,
    convert: fn(Outcome) -> PasskeyResult<T>,
) -> PasskeyResult<Ceremony<T>> {
    let mtm = MainThreadMarker::new().ok_or(PasskeyError::Unavailable {
        capability: PasskeyCapability::MainThread,
    })?;
    let anchor = anchor(mtm).ok_or(PasskeyError::Unavailable {
        capability: PasskeyCapability::Window,
    })?;

    let (completer, completion) = completion();
    let delegate = Delegate::new(mtm, anchor, completer);

    // SAFETY: `init` with a relying party identifier is the provider's designated
    // initializer.
    let provider = unsafe {
        ASAuthorizationPlatformPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
            mtm.alloc::<ASAuthorizationPlatformPublicKeyCredentialProvider>(),
            &NSString::from_str(rp_id),
        )
    };
    let requests = NSArray::from_retained_slice(&[build(mtm, &provider)]);

    // SAFETY: the requests are fully configured, and the delegate outlives the
    // controller's use of it because the returned `Ceremony` keeps both.
    let controller = unsafe {
        let controller = ASAuthorizationController::initWithAuthorizationRequests(
            mtm.alloc::<ASAuthorizationController>(),
            &requests,
        );
        controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // Through `msg_send!`: the setter's typed parameter is the bindings' copy of the
        // protocol, which is not the one `Delegate` adopts above.
        let _: () = msg_send![&*controller, setPresentationContextProvider: &*delegate];
        controller.performRequests();
        controller
    };

    Ok(Ceremony {
        session: Some(Session {
            controller,
            _delegate: delegate,
        }),
        completion,
        convert,
    })
}

/// The framework's constants are weakly linked, so each may be missing on an OS
/// that predates it; a missing one leaves the system default in place.
fn user_verification(preference: UserVerification) -> Option<&'static NSString> {
    // SAFETY: reads a framework-provided constant.
    unsafe {
        match preference {
            UserVerification::Required => {
                ASAuthorizationPublicKeyCredentialUserVerificationPreferenceRequired
            }
            UserVerification::Preferred => {
                ASAuthorizationPublicKeyCredentialUserVerificationPreferencePreferred
            }
            UserVerification::Discouraged => {
                ASAuthorizationPublicKeyCredentialUserVerificationPreferenceDiscouraged
            }
        }
    }
}

fn attestation(preference: Attestation) -> Option<&'static NSString> {
    // SAFETY: reads a framework-provided constant.
    unsafe {
        match preference {
            Attestation::None => ASAuthorizationPublicKeyCredentialAttestationKindNone,
            Attestation::Indirect => ASAuthorizationPublicKeyCredentialAttestationKindIndirect,
            Attestation::Direct => ASAuthorizationPublicKeyCredentialAttestationKindDirect,
            Attestation::Enterprise => ASAuthorizationPublicKeyCredentialAttestationKindEnterprise,
        }
    }
}

fn data(bytes: &[u8]) -> Retained<NSData> {
    NSData::with_bytes(bytes)
}

fn descriptors(
    mtm: MainThreadMarker,
    ids: &[CredentialId],
) -> Retained<NSArray<ASAuthorizationPlatformPublicKeyCredentialDescriptor>> {
    let descriptors: Vec<_> = ids
        .iter()
        .map(|id| {
            // SAFETY: the designated initializer, given a credential id.
            unsafe {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor::initWithCredentialID(
                    mtm.alloc::<ASAuthorizationPlatformPublicKeyCredentialDescriptor>(),
                    &data(id.as_bytes()),
                )
            }
        })
        .collect();
    NSArray::from_retained_slice(&descriptors)
}

fn registration_request(
    mtm: MainThreadMarker,
    provider: &ASAuthorizationPlatformPublicKeyCredentialProvider,
    request: &CreateRequest,
) -> Retained<ASAuthorizationRequest> {
    // SAFETY: the statics are framework-provided constants, and every setter takes
    // an object created here or one of those constants.
    unsafe {
        let registration = provider.createCredentialRegistrationRequestWithChallenge_name_userID(
            &data(&request.challenge),
            &NSString::from_str(&request.user.name),
            &data(&request.user.id),
        );
        registration.setDisplayName(Some(&NSString::from_str(&request.user.display_name)));
        if let Some(preference) = user_verification(request.user_verification) {
            registration.setUserVerificationPreference(preference);
        }
        if let Some(kind) = attestation(request.attestation) {
            registration.setAttestationPreference(kind);
        }

        // `excludedCredentials` arrived after the first passkey release (iOS 17.4,
        // macOS 14.4) and the bindings do not declare it on this class, so it is set
        // only where the running system answers to it.
        let setter = sel!(setExcludedCredentials:);
        if !request.exclude.is_empty() && registration.respondsToSelector(setter) {
            let excluded = descriptors(mtm, &request.exclude);
            let _: () = msg_send![&*registration, setExcludedCredentials: &*excluded];
        }
        registration.into_super()
    }
}

fn assertion_request(
    mtm: MainThreadMarker,
    provider: &ASAuthorizationPlatformPublicKeyCredentialProvider,
    request: &GetRequest,
) -> Retained<ASAuthorizationRequest> {
    // SAFETY: as for the registration request.
    unsafe {
        let assertion = provider.createCredentialAssertionRequestWithChallenge(&data(&request.challenge));
        if !request.allow.is_empty() {
            assertion.setAllowedCredentials(&descriptors(mtm, &request.allow));
        }
        if let Some(preference) = user_verification(request.user_verification) {
            assertion.setUserVerificationPreference(preference);
        }
        assertion.into_super()
    }
}

/// The key window of the application: what the system sheet slides out of.
#[cfg(target_os = "macos")]
fn anchor(mtm: MainThreadMarker) -> Option<Retained<NSObject>> {
    use objc2_app_kit::NSApplication;

    let app = NSApplication::sharedApplication(mtm);
    let window = app
        .keyWindow()
        .or_else(|| app.mainWindow())
        .or_else(|| app.windows().firstObject())?;
    Some(window.into_super().into_super())
}

/// The key window of the foreground scene.
#[cfg(target_os = "ios")]
fn anchor(mtm: MainThreadMarker) -> Option<Retained<NSObject>> {
    use objc2_ui_kit::{UIApplication, UISceneActivationState, UIWindowScene};

    let app = UIApplication::sharedApplication(mtm);
    let scenes = app.connectedScenes();
    let mut fallback = None;
    for scene in scenes.iter() {
        let Ok(scene) = scene.downcast::<UIWindowScene>() else {
            continue;
        };
        let window = scene.keyWindow().or_else(|| scene.windows().firstObject());
        if scene.activationState() == UISceneActivationState::ForegroundActive {
            if let Some(window) = window {
                return Some(window.into_super().into_super().into_super());
            }
        } else if fallback.is_none() {
            fallback = window;
        }
    }
    fallback.map(|window| window.into_super().into_super().into_super())
}
