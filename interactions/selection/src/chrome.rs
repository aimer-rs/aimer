//! Layout shared by every control: the labeled row and the padded shell.

use aimer_container::Container;
use aimer_flex::{BoxAlignment, Column, Row};
use aimer_style::{LayoutSpacing, Spacing, TextStyle, ThemeTokens};
use aimer_text::Text;
use aimer_widget::{AnyWidget, Widget};

use crate::indicator::Indicator;
use crate::interaction::Flags;
use crate::theme::{label_style, surface_color};

/// The validation message under a control, if there is one.
pub(crate) fn error_text(tokens: &ThemeTokens, error: Option<&str>) -> Option<AnyWidget> {
    error.map(|error| {
        Text::new(error.to_owned())
            .text_style(
                TextStyle::new()
                    .font_size(tokens.typography.label.font_size as u32)
                    .color(tokens.colors.error),
            )
            .boxed()
    })
}

/// An indicator followed by an optional label.
pub(crate) fn labeled_row(
    tokens: &ThemeTokens,
    indicator: Indicator,
    label: Option<&str>,
    disabled: bool,
    highlighted: bool,
) -> AnyWidget {
    let mut children = Vec::with_capacity(2);
    children.push(indicator.boxed());
    if let Some(label) = label {
        children.push(
            Text::new(label.to_owned())
                .text_style(label_style(tokens, disabled, highlighted))
                .boxed(),
        );
    }
    Row::new()
        .vertical_alignment(BoxAlignment::Center)
        .gaps(LayoutSpacing::all(Spacing::Px(tokens.spacing.small as u32)))
        .children(children)
        .boxed()
}

/// The padded, state-tinted surface around a control's content and error.
pub(crate) fn control_shell(
    tokens: &ThemeTokens,
    flags: Flags,
    disabled: bool,
    error: Option<&str>,
    child: AnyWidget,
) -> AnyWidget {
    // No explicit height: `Dimension::Auto` grows to fit a multi-row control,
    // while the indicator already meets the density minimum target per row.
    let mut children = Vec::with_capacity(2);
    children.push(child);
    children.extend(error_text(tokens, error));
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(
            tokens.spacing.x_small as u32,
        )))
        .color(surface_color(tokens, flags, disabled))
        .child(
            Column::new()
                .horizontal_alignment(BoxAlignment::Start)
                .gaps(LayoutSpacing::all(Spacing::Px(
                    tokens.spacing.x_small as u32,
                )))
                .children(children),
        )
        .boxed()
}
