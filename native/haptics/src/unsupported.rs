//! Targets without a haptic backend (Windows, Linux, the browser): every call
//! is a no-op, so widgets can fire haptics without a `cfg` at the call site.

use crate::{HapticPattern, ImpactStyle, NotificationStyle};

pub(crate) fn prepare_impact(_style: ImpactStyle) {}

pub(crate) fn impact(_style: ImpactStyle) {}

pub(crate) fn selection() {}

pub(crate) fn notification(_style: NotificationStyle) {}

pub(crate) fn play_pattern(_pattern: &HapticPattern) {}
