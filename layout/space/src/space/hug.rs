use aimer_attribute::dimension::Dimension;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::LayoutElement;
use aimer_widget::base::BuildContext;

/// Measures a child that its parent places by alignment or edge offsets.
///
/// An element whose size is [`Dimension::Auto`] fills whatever constraint it is
/// given, which is right for a child that is laid out, but it leaves a placed
/// child nothing to be placed *within*: an `Align` would see a child as large as
/// itself and a `Positioned` would measure `bottom` from the wrong edge. So on
/// each axis where the child did not ask for a size of its own, this reports the
/// size its content declares through [`LayoutElement::get_size_from_child`] when
/// that is a pixel size, capped at what the constraint allows. Every other axis
/// keeps the child's own [`LayoutElement::computed_size`].
///
/// Content that declares nothing (text, a `Row`, a `Column`) leaves the axis
/// filled, which is what it did before. So does a declared size of zero: that
/// is how content says it has no intrinsic extent, and collapsing the child to
/// nothing would hide it.
pub(crate) fn hugged_size<E: LayoutElement + ?Sized>(
    child: &E,
    ctx: &BuildContext,
) -> ResolvedSize {
    let measured = child.computed_size(ctx);
    let own = child.size();
    let width_is_free = own.is_none_or(|size| size.width == Dimension::Auto);
    let height_is_free = own.is_none_or(|size| size.height == Dimension::Auto);
    if !width_is_free && !height_is_free {
        return measured;
    }
    let Some(content) = child.get_size_from_child() else {
        return measured;
    };

    let hug = |free: bool, declared: Dimension, measured: f32| match declared {
        Dimension::Px(logical) if free && logical.is_finite() && logical > 0.0 => {
            (logical * ctx.scale).min(measured)
        }
        _ => measured,
    };
    ResolvedSize {
        width: hug(width_is_free, content.width, measured.width),
        height: hug(height_is_free, content.height, measured.height),
    }
}
