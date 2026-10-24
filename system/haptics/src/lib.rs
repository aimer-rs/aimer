//! Portable haptic-feedback capability contracts.
//!
//! Application code uses [`HapticFeedback`] and [`HapticKind`] in both native
//! AOT and interpreted builds. The generated guest bridge carries only a
//! canonical fixed-width integer; native enum layout never crosses the ABI.

use aimer_anteros::{CapabilityError, CapabilityResult, CapabilityTransport};

/// A portable haptic feedback effect supported by the Aimer contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum HapticKind {
    /// A light selection tick.
    Selection = 0,
    /// A light physical impact.
    LightImpact = 1,
    /// A medium physical impact.
    MediumImpact = 2,
    /// A heavy physical impact.
    HeavyImpact = 3,
    /// Positive task-completion feedback.
    Success = 4,
    /// Warning feedback.
    Warning = 5,
    /// Error feedback.
    Error = 6,
}

impl HapticKind {
    /// Returns the stable fixed-width wire discriminant.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

impl TryFrom<u32> for HapticKind {
    type Error = CapabilityError;

    fn try_from(value: u32) -> Result<Self, CapabilityError> {
        match value {
            0 => Ok(Self::Selection),
            1 => Ok(Self::LightImpact),
            2 => Ok(Self::MediumImpact),
            3 => Ok(Self::HeavyImpact),
            4 => Ok(Self::Success),
            5 => Ok(Self::Warning),
            6 => Ok(Self::Error),
            _ => Err(CapabilityError::InvalidRequest),
        }
    }
}

/// The one portable haptics API used by native providers and WASM guests.
pub trait HapticFeedback {
    /// Triggers `kind` or returns an explicit host capability failure.
    fn trigger(&self, kind: HapticKind) -> CapabilityResult<()>;
}

#[aimer_macro::capability(name = "haptics", id = "dev.aimer.haptics", abi = 1, since = "0.1.0")]
pub trait HapticsContract {
    /// Dispatches one validated fixed-width haptic discriminant.
    fn trigger(&self, kind: u32) -> CapabilityResult<()>;
}

/// Adapts a typed native haptics implementation to generated host dispatch.
pub struct HapticsProvider<P> {
    provider: P,
}

impl<P> HapticsProvider<P> {
    /// Wraps one permanent-host haptics implementation.
    #[inline]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> HapticsContract for HapticsProvider<P>
where
    P: HapticFeedback,
{
    fn trigger(&self, kind: u32) -> CapabilityResult<()> {
        self.provider.trigger(HapticKind::try_from(kind)?)
    }
}

impl<T> HapticFeedback for HapticsContractGuest<T>
where
    T: CapabilityTransport,
{
    fn trigger(&self, kind: HapticKind) -> CapabilityResult<()> {
        <HapticsContractGuest<T> as HapticsContract>::trigger(self, kind.as_u32())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aimer_anteros::{
        AbiVersion, ApplicationManifest, CALLBACK_EVENT_FORMAT_VERSION, CapabilityCall,
        CapabilityError, CapabilityLimits, CapabilityPolicy, CapabilityRegistry,
        CapabilityRegistryError, CapabilityResult, CapabilityStagingClass, CapabilityTransport,
        GenerationId, ManifestView, ModelLimits, STATE_FORMAT_VERSION, StableId128,
        WIDGET_IR_FORMAT_VERSION,
    };

    use crate::{
        HapticFeedback, HapticKind, HapticsContractCapability, HapticsContractGuest,
        HapticsContractHost, HapticsProvider,
    };

    const LIMITS: CapabilityLimits = CapabilityLimits::new(4, 0);
    const MODEL_LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
    const SELECTION_REQUEST: [u8; 4] = [0; 4];

    #[test]
    fn native_and_guest_haptics_use_the_same_typed_api() {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let native = RecordingHaptics(observed.clone());
        native.trigger(HapticKind::Success).unwrap();
        let host = HapticsContractHost::new(HapticsProvider::new(native), LIMITS);
        let guest = HapticsContractGuest::new(HostTransport(host), LIMITS);

        guest.trigger(HapticKind::Warning).unwrap();

        assert_eq!(
            observed.borrow().as_slice(),
            &[HapticKind::Success, HapticKind::Warning]
        );
    }

    #[test]
    fn generated_host_rejects_an_unknown_haptic_kind_before_native_dispatch() {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let host = HapticsContractHost::new(
            HapticsProvider::new(RecordingHaptics(observed.clone())),
            LIMITS,
        );

        let error = host.dispatch(0, &u32::MAX.to_le_bytes(), 0).unwrap_err();

        assert_eq!(error, CapabilityError::InvalidRequest);
        assert!(observed.borrow().is_empty());
    }

    #[test]
    fn required_and_optional_haptics_report_unavailable_providers() {
        let registry = CapabilityRegistry::new(0);
        let required = manifest(CapabilityPolicy::Required);
        let optional = manifest(CapabilityPolicy::Optional);

        assert_eq!(
            registry
                .negotiate_generation(&required, GenerationId::new(41))
                .unwrap_err(),
            CapabilityRegistryError::MissingRequiredProvider {
                capability_id: HapticsContractCapability::ID,
            }
        );
        let bindings = registry
            .negotiate_generation(&optional, GenerationId::new(41))
            .unwrap();
        assert_eq!(bindings.invoke(call()), Err(CapabilityError::Unsupported));
    }

    #[test]
    fn haptic_effects_are_rejected_for_candidates_and_after_retirement() {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let host = HapticsContractHost::new(
            HapticsProvider::new(RecordingHaptics(observed.clone())),
            LIMITS,
        );
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(host, CapabilityStagingClass::IrreversibleEffect)
            .unwrap();
        let bindings = registry
            .negotiate_generation(&manifest(CapabilityPolicy::Required), GenerationId::new(41))
            .unwrap();

        assert_eq!(bindings.invoke(call()), Err(CapabilityError::NotActive));
        assert!(observed.borrow().is_empty());
        bindings.activate();
        assert_eq!(bindings.invoke(call()), Ok(Vec::new()));
        assert_eq!(observed.borrow().as_slice(), &[HapticKind::Selection]);
        bindings.retire();
        assert_eq!(
            bindings.invoke(call()),
            Err(CapabilityError::RetiredGeneration)
        );
    }

    fn call() -> CapabilityCall<'static> {
        CapabilityCall::new(
            HapticsContractCapability::ID,
            HapticsContractCapability::ABI_MAJOR,
            0,
            &SELECTION_REQUEST,
            0,
        )
    }

    fn manifest(policy: CapabilityPolicy) -> ManifestView<'static> {
        let requirements = [HapticsContractCapability::requirement(policy)];
        let bytes = ApplicationManifest::new(
            AbiVersion::new(1, 0),
            AbiVersion::new(1, 0),
            WIDGET_IR_FORMAT_VERSION,
            CALLBACK_EVENT_FORMAT_VERSION,
            STATE_FORMAT_VERSION,
            StableId128::from_bytes([0x10; 16]),
            &requirements,
        )
        .encode(MODEL_LIMITS)
        .unwrap()
        .into_boxed_slice();
        ManifestView::decode(Box::leak(bytes), MODEL_LIMITS).unwrap()
    }

    struct RecordingHaptics(Rc<RefCell<Vec<HapticKind>>>);

    impl HapticFeedback for RecordingHaptics {
        fn trigger(&self, kind: HapticKind) -> CapabilityResult<()> {
            self.0.borrow_mut().push(kind);
            Ok(())
        }
    }

    struct HostTransport(HapticsContractHost<HapticsProvider<RecordingHaptics>>);

    impl CapabilityTransport for HostTransport {
        fn invoke(&self, call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
            assert_eq!(call.capability_id(), HapticsContractCapability::ID);
            self.0
                .dispatch(call.method_id(), call.request(), call.response_limit())
        }
    }
}
