//! Platform-neutral color values and keyboard-driven color selection.

use core::fmt;

use super::{
    CancelReason, FocusRestorer, FocusTarget, OverlayConsumer, OverlayRequest, PickerOutcome,
    PickerOverlay, PickerSession, PickerSessionError,
};

/// An RGBA color with byte channels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl Rgba {
    /// Creates an RGBA value from byte channels.
    #[inline]
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Returns the red channel.
    #[inline]
    pub const fn red(self) -> u8 {
        self.red
    }

    /// Returns the green channel.
    #[inline]
    pub const fn green(self) -> u8 {
        self.green
    }

    /// Returns the blue channel.
    #[inline]
    pub const fn blue(self) -> u8 {
        self.blue
    }

    /// Returns the alpha channel.
    #[inline]
    pub const fn alpha(self) -> u8 {
        self.alpha
    }
}

/// A validated hue/saturation/value/alpha color.
///
/// Hue is expressed in degrees in `0..=360`; saturation, value, and alpha are
/// integer percentages in `0..=100`. The inclusive hue endpoint is retained
/// as a useful keyboard boundary even though it converts to the same RGB hue
/// as zero.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Hsva {
    hue: u16,
    saturation: u8,
    value: u8,
    alpha: u8,
}

impl Hsva {
    /// Creates a validated HSVA value.
    pub fn try_new(hue: u16, saturation: u16, value: u16, alpha: u16) -> Result<Self, ColorError> {
        if hue > 360 {
            return Err(ColorError::InvalidHue(hue));
        }
        let saturation =
            u8::try_from(saturation).map_err(|_| ColorError::InvalidSaturation(saturation))?;
        let value = u8::try_from(value).map_err(|_| ColorError::InvalidValue(value))?;
        let alpha = u8::try_from(alpha).map_err(|_| ColorError::InvalidAlpha(alpha))?;
        if saturation > 100 {
            return Err(ColorError::InvalidSaturation(u16::from(saturation)));
        }
        if value > 100 {
            return Err(ColorError::InvalidValue(u16::from(value)));
        }
        if alpha > 100 {
            return Err(ColorError::InvalidAlpha(u16::from(alpha)));
        }
        Ok(Self {
            hue,
            saturation,
            value,
            alpha,
        })
    }

    /// Returns the hue in degrees.
    #[inline]
    pub const fn hue(self) -> u16 {
        self.hue
    }

    /// Returns saturation as a percentage.
    #[inline]
    pub const fn saturation(self) -> u8 {
        self.saturation
    }

    /// Returns value/brightness as a percentage.
    #[inline]
    pub const fn value(self) -> u8 {
        self.value
    }

    /// Returns alpha as a percentage.
    #[inline]
    pub const fn alpha(self) -> u8 {
        self.alpha
    }

    /// Converts this HSV value to byte-channel RGBA using deterministic
    /// nearest-integer rounding.
    pub fn to_rgba(self) -> Rgba {
        // Keep the normalized channels over a common integer denominator so
        // midpoint values round consistently across platforms and toolchains.
        let saturation = u64::from(self.saturation);
        let value = u64::from(self.value);
        let chroma = saturation * value;
        let minimum = value * (100 - saturation) * 60;
        let hue = self.hue % 360;
        let sector = hue / 60;
        let offset = u64::from(hue % 60);
        let x_factor = match sector {
            0 | 2 | 4 => offset,
            _ => 60 - offset,
        };
        let x = chroma * x_factor;
        let chroma = chroma * 60;
        let (red, green, blue) = match sector {
            0 => (chroma, x, 0),
            1 => (x, chroma, 0),
            2 => (0, chroma, x),
            3 => (0, x, chroma),
            4 => (x, 0, chroma),
            _ => (chroma, 0, x),
        };
        let to_byte = |channel: u64| ((channel + minimum) * 255 + 300_000) / 600_000;
        Rgba::new(
            to_byte(red) as u8,
            to_byte(green) as u8,
            to_byte(blue) as u8,
            ((u64::from(self.alpha) * 255 + 50) / 100) as u8,
        )
    }
}

/// The independently keyboard-adjustable color axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorChannel {
    /// Hue in degrees.
    Hue,
    /// Saturation percentage.
    Saturation,
    /// Value/brightness percentage.
    Value,
    /// Alpha/transparency percentage.
    Alpha,
}

/// Keyboard operations supported by a color axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorKey {
    /// Increase by the configured channel step.
    Increase,
    /// Decrease by the configured channel step.
    Decrease,
    /// Move to the channel's minimum.
    Home,
    /// Move to the channel's maximum.
    End,
}

/// Stable identifier for a caller-provided color swatch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SwatchId(u64);

impl SwatchId {
    /// Creates an application-owned swatch identifier.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the application-owned identifier.
    #[inline]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// A selectable or disabled color swatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Swatch {
    id: SwatchId,
    color: Hsva,
    disabled: bool,
}

impl Swatch {
    /// Creates a swatch with an explicit disabled state.
    pub const fn new(id: SwatchId, color: Hsva, disabled: bool) -> Self {
        Self {
            id,
            color,
            disabled,
        }
    }

    /// Returns the stable swatch identifier.
    #[inline]
    pub const fn id(self) -> SwatchId {
        self.id
    }

    /// Returns the swatch color.
    #[inline]
    pub const fn color(self) -> Hsva {
        self.color
    }

    /// Returns whether the swatch is disabled.
    #[inline]
    pub const fn is_disabled(self) -> bool {
        self.disabled
    }
}

/// Errors returned by color construction and interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorError {
    /// Hue exceeded 360 degrees.
    InvalidHue(u16),
    /// Saturation exceeded 100 percent.
    InvalidSaturation(u16),
    /// Value exceeded 100 percent.
    InvalidValue(u16),
    /// Alpha exceeded 100 percent.
    InvalidAlpha(u16),
    /// A keyboard step must be non-zero.
    InvalidStep,
    /// Alpha interaction was requested for a picker that does not support it.
    AlphaDisabled,
    /// A swatch was requested but is not registered.
    UnknownSwatch(SwatchId),
    /// A registered swatch cannot be selected.
    DisabledSwatch(SwatchId),
    /// A swatch identifier is already registered.
    DuplicateSwatch(SwatchId),
    /// The picker must be opened before it can be edited.
    Closed,
    /// No overlay host is installed for the presentation request.
    MissingHost,
    /// An installed overlay host cannot represent the presentation request.
    UnsupportedHost,
    /// The overlay policy does not allow this user-driven dismissal reason.
    DismissalNotAllowed(CancelReason),
}

impl fmt::Display for ColorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHue(value) => write!(formatter, "hue {value} is outside 0..=360"),
            Self::InvalidSaturation(value) => {
                write!(formatter, "saturation {value} is outside 0..=100")
            }
            Self::InvalidValue(value) => write!(formatter, "value {value} is outside 0..=100"),
            Self::InvalidAlpha(value) => write!(formatter, "alpha {value} is outside 0..=100"),
            Self::InvalidStep => formatter.write_str("color keyboard steps must be non-zero"),
            Self::AlphaDisabled => formatter.write_str("alpha interaction is disabled"),
            Self::UnknownSwatch(id) => write!(formatter, "swatch {} is not registered", id.id()),
            Self::DisabledSwatch(id) => write!(formatter, "swatch {} is disabled", id.id()),
            Self::DuplicateSwatch(id) => {
                write!(formatter, "swatch {} is already registered", id.id())
            }
            Self::Closed => formatter.write_str("color picker is closed"),
            Self::MissingHost => formatter.write_str("picker overlay host is missing"),
            Self::UnsupportedHost => formatter.write_str("picker overlay host is unsupported"),
            Self::DismissalNotAllowed(reason) => {
                write!(
                    formatter,
                    "picker overlay dismissal is disabled for {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for ColorError {}

/// A transactional color picker model with keyboard and swatch seams.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorPicker {
    session: PickerSession<Hsva>,
    alpha_enabled: bool,
    hue_step: u16,
    channel_step: u8,
    swatches: Vec<Swatch>,
}

impl ColorPicker {
    /// Creates a closed picker with one-degree and one-percent keyboard steps.
    pub fn new(initial: Hsva, alpha_enabled: bool) -> Self {
        Self {
            session: PickerSession::new(initial),
            alpha_enabled,
            hue_step: 1,
            channel_step: 1,
            swatches: Vec::new(),
        }
    }

    /// Opens the picker and resets its draft to the last confirmed color.
    pub fn open(&mut self) {
        self.session.open();
    }

    /// Presents the picker through a checked caller-owned overlay host and
    /// opens its transactional draft.
    pub fn open_with_overlay<C: OverlayConsumer>(
        &mut self,
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<PickerOverlay<C::Handle>, ColorError> {
        self.session
            .open_with_overlay(consumer, request, restore_focus)
            .map_err(ColorError::from)
    }

    /// Returns whether the picker is open.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.session.is_open()
    }

    /// Returns whether the alpha channel accepts keyboard interaction.
    #[inline]
    pub const fn alpha_enabled(&self) -> bool {
        self.alpha_enabled
    }

    /// Returns the last confirmed color.
    #[inline]
    pub const fn value(&self) -> Hsva {
        *self.session.committed()
    }

    /// Returns the current draft color.
    #[inline]
    pub const fn draft(&self) -> Hsva {
        *self.session.draft()
    }

    /// Builds a platform-neutral accessibility snapshot of this picker.
    #[inline]
    pub fn semantics(&self) -> super::ColorPickerSemantics {
        super::ColorPickerSemantics::from_picker(self)
    }

    /// Returns the current confirmed color as RGBA.
    #[inline]
    pub fn rgba(&self) -> Rgba {
        self.value().to_rgba()
    }

    /// Sets non-zero hue and channel keyboard increments.
    pub fn set_steps(&mut self, hue_step: u16, channel_step: u8) -> Result<(), ColorError> {
        if hue_step == 0 || channel_step == 0 {
            return Err(ColorError::InvalidStep);
        }
        self.hue_step = hue_step;
        self.channel_step = channel_step;
        Ok(())
    }

    /// Registers a caller-owned swatch, rejecting duplicate identifiers.
    pub fn add_swatch(&mut self, swatch: Swatch) -> Result<(), ColorError> {
        if self
            .swatches
            .iter()
            .any(|existing| existing.id() == swatch.id())
        {
            return Err(ColorError::DuplicateSwatch(swatch.id()));
        }
        self.swatches.push(swatch);
        Ok(())
    }

    /// Returns the registered swatches in insertion order.
    #[inline]
    pub fn swatches(&self) -> &[Swatch] {
        &self.swatches
    }

    /// Selects one enabled swatch into the draft.
    pub fn select_swatch(&mut self, id: SwatchId) -> Result<(), ColorError> {
        if !self.is_open() {
            return Err(ColorError::Closed);
        }
        let Some(swatch) = self.swatches.iter().find(|swatch| swatch.id() == id) else {
            return Err(ColorError::UnknownSwatch(id));
        };
        if swatch.is_disabled() {
            return Err(ColorError::DisabledSwatch(id));
        }
        self.session
            .set_draft(swatch.color())
            .map_err(|_| ColorError::Closed)
    }

    /// Sets one channel on the open draft from a pointer or other continuous
    /// input source.
    pub fn set_channel_value(
        &mut self,
        channel: ColorChannel,
        value: u16,
    ) -> Result<(), ColorError> {
        if !self.is_open() {
            return Err(ColorError::Closed);
        }
        if channel == ColorChannel::Alpha && !self.alpha_enabled {
            return Err(ColorError::AlphaDisabled);
        }
        match channel {
            ColorChannel::Hue if value > 360 => return Err(ColorError::InvalidHue(value)),
            ColorChannel::Saturation if value > 100 => {
                return Err(ColorError::InvalidSaturation(value));
            }
            ColorChannel::Value if value > 100 => return Err(ColorError::InvalidValue(value)),
            ColorChannel::Alpha if value > 100 => return Err(ColorError::InvalidAlpha(value)),
            _ => {}
        }
        let current = self.draft();
        let next = match channel {
            ColorChannel::Hue => Hsva::try_new(
                value,
                u16::from(current.saturation()),
                u16::from(current.value()),
                u16::from(current.alpha()),
            ),
            ColorChannel::Saturation => Hsva::try_new(
                current.hue(),
                value,
                u16::from(current.value()),
                u16::from(current.alpha()),
            ),
            ColorChannel::Value => Hsva::try_new(
                current.hue(),
                u16::from(current.saturation()),
                value,
                u16::from(current.alpha()),
            ),
            ColorChannel::Alpha => Hsva::try_new(
                current.hue(),
                u16::from(current.saturation()),
                u16::from(current.value()),
                value,
            ),
        }
        .expect("validated color channel values stay within HSVA bounds");
        self.session.set_draft(next).map_err(|_| ColorError::Closed)
    }

    /// Applies one keyboard operation to a color channel.
    pub fn handle_key(&mut self, channel: ColorChannel, key: ColorKey) -> Result<(), ColorError> {
        if !self.is_open() {
            return Err(ColorError::Closed);
        }
        if channel == ColorChannel::Alpha && !self.alpha_enabled {
            return Err(ColorError::AlphaDisabled);
        }
        let current = self.draft();
        let (minimum, maximum, step) = match channel {
            ColorChannel::Hue => (0_i32, 360_i32, i32::from(self.hue_step)),
            ColorChannel::Saturation | ColorChannel::Value | ColorChannel::Alpha => {
                (0, 100, i32::from(self.channel_step))
            }
        };
        let current = match channel {
            ColorChannel::Hue => i32::from(current.hue()),
            ColorChannel::Saturation => i32::from(current.saturation()),
            ColorChannel::Value => i32::from(current.value()),
            ColorChannel::Alpha => i32::from(current.alpha()),
        };
        let next = match key {
            ColorKey::Increase => current.saturating_add(step).min(maximum),
            ColorKey::Decrease => current.saturating_sub(step).max(minimum),
            ColorKey::Home => minimum,
            ColorKey::End => maximum,
        };
        self.set_channel_value(
            channel,
            u16::try_from(next).expect("color channel bounds are non-negative"),
        )
    }

    /// Confirms the current draft.
    pub fn confirm(&mut self) -> Result<PickerOutcome<Hsva>, ColorError> {
        self.session.confirm().map_err(|_| ColorError::Closed)
    }

    /// Cancels the current draft and keeps the last confirmed color.
    pub fn cancel(&mut self, reason: CancelReason) -> Result<PickerOutcome<Hsva>, ColorError> {
        self.session.cancel(reason).map_err(|_| ColorError::Closed)
    }

    /// Confirms the draft, dismisses its overlay, and restores focus.
    pub fn confirm_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Hsva>, ColorError> {
        self.session
            .confirm_with_overlay(overlay, consumer, focus)
            .map_err(ColorError::from)
    }

    /// Cancels the draft according to the overlay policy, dismisses the
    /// overlay, and restores focus.
    pub fn cancel_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Hsva>, ColorError> {
        self.session
            .cancel_with_overlay(overlay, reason, consumer, focus)
            .map_err(ColorError::from)
    }

    /// Closes the picker as a programmatic cancellation through its overlay.
    pub fn close_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Hsva>, ColorError> {
        self.cancel_with_overlay(overlay, CancelReason::Programmatic, consumer, focus)
    }
}

impl From<PickerSessionError> for ColorError {
    fn from(error: PickerSessionError) -> Self {
        match error {
            PickerSessionError::Closed => Self::Closed,
            PickerSessionError::MissingHost => Self::MissingHost,
            PickerSessionError::UnsupportedHost => Self::UnsupportedHost,
            PickerSessionError::DismissalNotAllowed(reason) => Self::DismissalNotAllowed(reason),
        }
    }
}
