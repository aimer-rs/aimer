use aimer_picker::{
    CancelReason, ColorChannel, ColorError, ColorKey, ColorPicker, Hsva, PickerOutcome, Rgba,
    Swatch, SwatchId,
};

#[test]
fn hsva_accepts_inclusive_boundaries_and_rejects_invalid_channels() {
    let minimum = Hsva::try_new(0, 0, 0, 0).unwrap();
    assert_eq!(minimum.hue(), 0);
    assert_eq!(minimum.saturation(), 0);
    assert_eq!(minimum.value(), 0);
    assert_eq!(minimum.alpha(), 0);

    let maximum = Hsva::try_new(360, 100, 100, 100).unwrap();
    assert_eq!(maximum.hue(), 360);
    assert_eq!(maximum.saturation(), 100);
    assert_eq!(maximum.value(), 100);
    assert_eq!(maximum.alpha(), 100);

    assert_eq!(
        Hsva::try_new(361, 0, 0, 0),
        Err(ColorError::InvalidHue(361))
    );
    assert_eq!(
        Hsva::try_new(0, 101, 0, 0),
        Err(ColorError::InvalidSaturation(101))
    );
    assert_eq!(
        Hsva::try_new(0, 0, 101, 0),
        Err(ColorError::InvalidValue(101))
    );
    assert_eq!(
        Hsva::try_new(0, 0, 0, 101),
        Err(ColorError::InvalidAlpha(101))
    );
}

#[test]
fn hsva_to_rgba_uses_standard_hues_and_nearest_byte_rounding() {
    assert_eq!(
        Hsva::try_new(0, 100, 100, 100).unwrap().to_rgba(),
        Rgba::new(255, 0, 0, 255)
    );
    assert_eq!(
        Hsva::try_new(120, 100, 100, 50).unwrap().to_rgba(),
        Rgba::new(0, 255, 0, 128)
    );
    assert_eq!(
        Hsva::try_new(240, 100, 100, 0).unwrap().to_rgba(),
        Rgba::new(0, 0, 255, 0)
    );
    assert_eq!(
        Hsva::try_new(30, 100, 100, 100).unwrap().to_rgba(),
        Rgba::new(255, 128, 0, 255)
    );
    assert_eq!(
        Hsva::try_new(210, 80, 90, 100).unwrap().to_rgba(),
        Rgba::new(46, 138, 230, 255)
    );
    assert_eq!(
        Hsva::try_new(360, 100, 100, 100).unwrap().to_rgba(),
        Rgba::new(255, 0, 0, 255)
    );
}

#[test]
fn color_picker_keyboard_steps_clamp_each_axis_at_home_and_end() {
    let initial = Hsva::try_new(120, 40, 50, 60).unwrap();
    let mut picker = ColorPicker::new(initial, true);
    picker.open();
    picker.set_steps(30, 10).unwrap();

    picker
        .handle_key(ColorChannel::Hue, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().hue(), 150);
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Decrease)
        .unwrap();
    assert_eq!(picker.draft().hue(), 120);
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Home)
        .unwrap();
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Decrease)
        .unwrap();
    assert_eq!(picker.draft().hue(), 0);
    picker.handle_key(ColorChannel::Hue, ColorKey::End).unwrap();
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().hue(), 360);

    picker
        .handle_key(ColorChannel::Saturation, ColorKey::Home)
        .unwrap();
    picker
        .handle_key(ColorChannel::Saturation, ColorKey::Decrease)
        .unwrap();
    assert_eq!(picker.draft().saturation(), 0);
    picker
        .handle_key(ColorChannel::Saturation, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().saturation(), 10);
    picker
        .handle_key(ColorChannel::Saturation, ColorKey::End)
        .unwrap();
    picker
        .handle_key(ColorChannel::Saturation, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().saturation(), 100);

    picker
        .handle_key(ColorChannel::Value, ColorKey::Home)
        .unwrap();
    picker
        .handle_key(ColorChannel::Value, ColorKey::Decrease)
        .unwrap();
    assert_eq!(picker.draft().value(), 0);
    picker
        .handle_key(ColorChannel::Value, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().value(), 10);
    picker
        .handle_key(ColorChannel::Value, ColorKey::End)
        .unwrap();
    picker
        .handle_key(ColorChannel::Value, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().value(), 100);

    picker
        .handle_key(ColorChannel::Alpha, ColorKey::Home)
        .unwrap();
    picker
        .handle_key(ColorChannel::Alpha, ColorKey::Decrease)
        .unwrap();
    assert_eq!(picker.draft().alpha(), 0);
    picker
        .handle_key(ColorChannel::Alpha, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().alpha(), 10);
    picker
        .handle_key(ColorChannel::Alpha, ColorKey::End)
        .unwrap();
    picker
        .handle_key(ColorChannel::Alpha, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft().alpha(), 100);

    assert_eq!(picker.set_steps(0, 10), Err(ColorError::InvalidStep));
    assert_eq!(picker.set_steps(30, 0), Err(ColorError::InvalidStep));
}

#[test]
fn color_picker_disables_alpha_keyboard_input_without_mutating_the_draft() {
    let initial = Hsva::try_new(210, 80, 90, 35).unwrap();
    let mut picker = ColorPicker::new(initial, false);
    picker.open();

    assert_eq!(
        picker.handle_key(ColorChannel::Alpha, ColorKey::End),
        Err(ColorError::AlphaDisabled)
    );
    assert_eq!(picker.draft(), initial);
}

#[test]
fn color_picker_swatches_preserve_order_and_reject_duplicate_disabled_or_unknown_selection() {
    let initial = Hsva::try_new(0, 100, 100, 100).unwrap();
    let disabled_id = SwatchId::new(7);
    let enabled_id = SwatchId::new(8);
    let mut picker = ColorPicker::new(initial, true);
    let disabled = Swatch::new(
        disabled_id,
        Hsva::try_new(120, 100, 100, 100).unwrap(),
        true,
    );
    let enabled = Swatch::new(
        enabled_id,
        Hsva::try_new(240, 100, 100, 100).unwrap(),
        false,
    );

    picker.add_swatch(disabled).unwrap();
    picker.add_swatch(enabled).unwrap();
    assert_eq!(picker.swatches(), &[disabled, enabled]);
    assert_eq!(
        picker.add_swatch(Swatch::new(disabled_id, initial, false)),
        Err(ColorError::DuplicateSwatch(disabled_id))
    );
    assert_eq!(picker.swatches(), &[disabled, enabled]);

    assert_eq!(picker.select_swatch(enabled_id), Err(ColorError::Closed));
    picker.open();
    assert_eq!(
        picker.select_swatch(SwatchId::new(99)),
        Err(ColorError::UnknownSwatch(SwatchId::new(99)))
    );
    assert_eq!(picker.draft(), initial);
    assert_eq!(
        picker.select_swatch(disabled_id),
        Err(ColorError::DisabledSwatch(disabled_id))
    );
    assert_eq!(picker.draft(), initial);
    picker.select_swatch(enabled_id).unwrap();
    assert_eq!(picker.draft(), enabled.color());
}

#[test]
fn color_picker_transactions_reset_drafts_and_commit_only_on_confirmation() {
    let initial = Hsva::try_new(20, 30, 40, 50).unwrap();
    let mut picker = ColorPicker::new(initial, true);

    assert_eq!(picker.confirm(), Err(ColorError::Closed));
    assert_eq!(picker.cancel(CancelReason::Escape), Err(ColorError::Closed));

    picker.open();
    picker.handle_key(ColorChannel::Hue, ColorKey::End).unwrap();
    assert_ne!(picker.draft(), picker.value());
    assert_eq!(
        picker.cancel(CancelReason::OutsideClick),
        Ok(PickerOutcome::Cancelled {
            reason: CancelReason::OutsideClick,
            value: initial,
        })
    );
    assert!(!picker.is_open());
    assert_eq!(picker.value(), initial);
    assert_eq!(picker.draft(), initial);

    picker.open();
    assert_eq!(picker.draft(), initial);
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Home)
        .unwrap();
    picker
        .handle_key(ColorChannel::Saturation, ColorKey::End)
        .unwrap();
    picker
        .handle_key(ColorChannel::Value, ColorKey::End)
        .unwrap();
    picker
        .handle_key(ColorChannel::Alpha, ColorKey::End)
        .unwrap();
    assert_eq!(
        picker.confirm(),
        Ok(PickerOutcome::Confirmed(
            Hsva::try_new(0, 100, 100, 100).unwrap()
        ))
    );
    assert!(!picker.is_open());

    let committed = picker.value();
    assert_eq!(committed, Hsva::try_new(0, 100, 100, 100).unwrap());
    picker.open();
    picker
        .handle_key(ColorChannel::Hue, ColorKey::Increase)
        .unwrap();
    assert_eq!(picker.draft(), Hsva::try_new(1, 100, 100, 100).unwrap());
    assert_eq!(
        picker.cancel(CancelReason::Programmatic),
        Ok(PickerOutcome::Cancelled {
            reason: CancelReason::Programmatic,
            value: committed,
        })
    );
    assert_eq!(picker.value(), committed);
    assert_eq!(picker.draft(), committed);
}
