/// Above this many logical pixels a height is a placeholder for "as much as
/// you like" rather than an offer, which is what a flex measuring pass hands a
/// child along its main axis.
const UNBOUNDED_HEIGHT: f32 = 1_000_000.0;

impl RawTextField {
    fn compute_dimensions(&self, ctx: &BuildContext) -> (f32, f32) {
        let constraint = ctx.box_constraint;
        // A field that fills its parent can only do so when the parent offers
        // a height it actually has. Answering an unbounded measuring pass with
        // that unbounded value claims a box no screen has room for, and
        // everything drawn relative to it — the caret above all — lands
        // outside the field. Such a field falls through and reports the
        // intrinsic height of its lines instead.
        if (self.max_lines == Some(1)
            || matches!(self.expand, ExpandDirection::Vertical | ExpandDirection::Both))
            && constraint.max_height < UNBOUNDED_HEIGHT
        {
            return (constraint.max_width, constraint.max_height);
        }

        let scale = ctx.scale;
        let width = constraint.max_width;
        // Padding given as a percentage of an unbounded height resolves to
        // nothing rather than to a share of infinity.
        let height_reference = if constraint.max_height < UNBOUNDED_HEIGHT {
            constraint.max_height
        } else {
            0.0
        };
        let pad_top = self.padding.top.value(height_reference, scale);
        let pad_bottom = self.padding.bottom.value(height_reference, scale);
        let pad_left = self.padding.left.value(width, scale);
        let pad_right = self.padding.right.value(width, scale);
        let content_width = (width - pad_left - pad_right).max(1.0);
        let font_size = self.scaled_font_size(&self.text_style, scale);
        let geometry = self.editable_geometry(ctx, content_width);
        let min_lines = self.min_lines.unwrap_or(1).max(1);
        let line_count = geometry
            .visual_lines
            .len()
            .max(min_lines)
            .min(self.max_lines.unwrap_or(usize::MAX).max(min_lines));
        let style = self.field_text_style();
        let line_height = geometry
            .interaction
            .as_ref()
            .map(|layout| layout.metrics.line_height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or_else(|| {
                ctx.canvas
                    .measure_text_metrics_styled(
                        "",
                        font_size,
                        0.0,
                        style.font_family,
                        style.font_style,
                        style.font_weight.numeric(),
                    )
                    .line_height
            });
        let desired_height = line_count as f32 * line_height + pad_top + pad_bottom;
        (width, desired_height.min(constraint.max_height))
    }
}

#[cfg(test)]
mod dimensions_tests {
    //! The box a field asks for.
    //!
    //! A single-line field fills the height it is offered, which is what makes
    //! `SizedBox::new().height(48)` around a field work. An offered height is
    //! not always a real one though: a column measures its children against an
    //! unbounded main axis, and a field that answers with that unbounded value
    //! claims a box no screen has room for.

    use aimer_widget::LayoutElement;

    use super::test_support::{dummy_build_context, focused_single_line_field};
    use crate::input_field::controller::TextFieldController;

    #[test]
    fn a_single_line_field_fills_the_height_it_is_offered() {
        let field = focused_single_line_field(TextFieldController::with_initial("hello"));
        let ctx = dummy_build_context(400.0, 48.0);

        assert_eq!(field.computed_size(&ctx).height, 48.0);
    }

    #[test]
    fn a_single_line_field_asks_for_one_line_when_the_height_is_unbounded() {
        let field = focused_single_line_field(TextFieldController::with_initial("hello"));
        let mut ctx = dummy_build_context(400.0, 600.0);
        ctx.box_constraint.max_height = f32::MAX;
        let font_size = field.scaled_font_size(&field.text_style, ctx.scale);
        let line_height = ctx
            .canvas
            .measure_text_metrics("", font_size, 0.0)
            .line_height;

        let height = field.computed_size(&ctx).height;

        // Four logical pixels of padding on each side, and the outline of the
        // default decoration adds nothing.
        assert!(
            (height - (line_height + 8.0)).abs() <= 1.0,
            "field asked for {height}, expected one {line_height} line plus padding",
        );
    }
}
