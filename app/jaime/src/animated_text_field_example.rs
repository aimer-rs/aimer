//! Text fields whose frame is animated from their own state.
//!
//! Both fields use [`TextField::builder`]: the closure returns the whole look
//! of the field and places the live editing surface with
//! [`TextFieldVisualState::input`]. An [`ImplicitAnimatedBuilder`] inside the
//! closure eases a single progress value toward focus, so the label glides, the
//! border changes color and the underline sweeps, while the editor underneath
//! keeps its caret, selection and composition.

use std::time::Duration;

use aimer::animation::{Animatable, ImplicitAnimatedBuilder, curves};
use aimer::input::TextEditingController;
use aimer::input::input::{InputType, TextField, TextFieldVisualState};
use aimer::style::*;
use aimer::*;

use crate::theme;

const FADE: Duration = Duration::from_millis(180);
const FIELD_HEIGHT: u32 = 58;

/// Starts the animated text-field showcase.
pub fn start_animated_text_field_example() {
    AimerApp::start(theme::provide(AnimatedTextFieldExample::new().boxed()))
}

/// Demonstrates a floating-label field and an underline field, both driven by
/// the state a field's builder receives.
pub struct AnimatedTextFieldExample {
    name: TextEditingController,
    email: TextEditingController,
}

impl AnimatedTextFieldExample {
    /// Creates the showcase with one empty field and one that starts filled.
    #[inline]
    pub fn new() -> Self {
        Self {
            name: TextEditingController::new(),
            email: TextEditingController::with_text("hello@aimer.dev"),
        }
    }
}

impl Default for AnimatedTextFieldExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for AnimatedTextFieldExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let theme = ThemeData::copied(ctx);
        let palette = theme_for(&theme);
        let text_style = TextStyle::new().font_size(16).color(theme.on_surface_color);

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(20)))
                    .children(vec![
                        Text::new("Animated TextField")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        Text::new(
                            "The builder receives hover, focus and emptiness; the frame eases \
                             between them while the editor stays mounted.",
                        )
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .color(theme::muted_text(&theme)),
                        )
                        .boxed(),
                        TextField::new()
                            .controller(self.name)
                            .input_type(InputType::Text)
                            .text_style(text_style)
                            .cursor_color(theme::input_cursor_color(&theme))
                            .selection_color(theme.primary_color.with_alpha(0.28))
                            .padding(
                                LayoutSpacing::new()
                                    .left(14)
                                    .right(14)
                                    .top(24)
                                    .bottom(8),
                            )
                            .builder(move |state| floating_label(&palette, "Full name", state))
                            .boxed(),
                        TextField::new()
                            .controller(self.email)
                            .input_type(InputType::Email)
                            .text_style(text_style)
                            .cursor_color(theme::input_cursor_color(&theme))
                            .selection_color(theme.primary_color.with_alpha(0.28))
                            .padding(LayoutSpacing::new().top(6).bottom(6))
                            .builder(move |state| underline(&palette, "Email address", state))
                            .boxed(),
                    ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedTextFieldExample"
    }
}

impl aimer::PortableWidget for AnimatedTextFieldExample {}

/// The theme colors a builder closure needs, copied out so the closure owns them.
#[derive(Clone, Copy)]
struct Palette {
    muted: Color,
    accent: Color,
    divider: Color,
    resting: Color,
    raised: Color,
}

fn theme_for(theme: &ThemeData) -> Palette {
    Palette {
        muted: theme::muted_text(theme),
        accent: theme.primary_color,
        divider: theme::divider(theme),
        resting: theme::recessed_surface(theme),
        raised: theme::raised_surface(theme),
    }
}

/// How far the field has moved toward "active": focused, hovered, or holding text.
#[inline]
fn target(state: &TextFieldVisualState) -> (f32, f32) {
    let lifted = if state.is_focused || !state.is_empty { 1.0 } else { 0.0 };
    let engaged = if state.is_focused {
        1.0
    } else if state.is_hovered {
        0.5
    } else {
        0.0
    };
    (lifted, engaged)
}

/// An outlined field whose label rises into the border once there is something
/// to label, and whose border and fill warm up with hover and focus.
fn floating_label(palette: &Palette, label: &'static str, state: TextFieldVisualState) -> AnyWidget {
    let palette = *palette;
    let (lifted, engaged) = target(&state);
    let input = state.input();
    ImplicitAnimatedBuilder::new(
        (lifted, engaged),
        FADE,
        curves::EASE_IN_OUT,
        move |&(lifted, engaged): &(f32, f32)| {
            let border = palette.divider.lerp(palette.accent, engaged);
            let fill = palette.resting.lerp(palette.raised, engaged);
            let label_color = palette.muted.lerp(palette.accent, engaged);
            let label_size = 16.0_f32.lerp(&12.0, lifted);
            let label_top = 18.0_f32.lerp(&7.0, lifted);
            Container::new()
                .height(FIELD_HEIGHT)
                .child(Stack::new().children([
                    Container::new()
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(fill)
                                .border(BoxBorder::all(
                                    BorderSlice::new()
                                        .style(BorderStyle::Solid)
                                        .stroke(Stroke::Px(1.5))
                                        .color(border),
                                ))
                                .border_radius(10),
                        )
                        .box_child(input.clone()),
                    Positioned::new()
                        .left(15)
                        .top(Dimension::Px(label_top))
                        .layer(1)
                        .child(
                            Text::new(label).text_style(
                                TextStyle::new()
                                    .font_size(label_size.round() as u32)
                                    .color(label_color),
                            ),
                        )
                        .boxed(),
                ]))
        },
    )
    .boxed()
}

/// A borderless field over a rule that an accent line sweeps across from the
/// middle, with a caption that follows the focus color.
fn underline(palette: &Palette, label: &'static str, state: TextFieldVisualState) -> AnyWidget {
    let palette = *palette;
    let (_, engaged) = target(&state);
    let input = state.input();
    ImplicitAnimatedBuilder::new(
        engaged,
        FADE,
        curves::EASE_IN_OUT,
        move |&engaged: &f32| {
            let caption = palette.muted.lerp(palette.accent, engaged);
            let inset = Spacing::Percent((50.0 * (1.0 - engaged)).round() as u32);
            Column::new().children([
                Text::new(label)
                    .text_style(TextStyle::new().font_size(13).color(caption))
                    .boxed(),
                input.clone().boxed(),
                // A resting rule with an accent bar over it. Padding closes in
                // from both edges, so the bar grows outward from the middle.
                Stack::new()
                    .children([
                        SizedBox::new()
                            .width(Dimension::Percent(100.0))
                            .height(2)
                            .color(palette.divider)
                            .boxed(),
                        Container::new()
                            .height(2)
                            .padding(LayoutSpacing::new().left(inset).right(inset))
                            .child(Container::new().color(palette.accent).child(ZeroSizedBox))
                            .boxed(),
                    ])
                    .boxed(),
            ])
        },
    )
    .boxed()
}
