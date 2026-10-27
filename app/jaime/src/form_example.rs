//! A live showcase for `aimer_form`'s controlled text fields, validation,
//! dirty/touched state, submit/reset, and focus-on-first-error behavior.
//!
//! Every field is a real, editable [`TextField`]. Typing marks a field
//! touched and dirty immediately; leaving the field (or submitting) runs its
//! explicit validators and shows the first error, if any. Submitting a form
//! with an invalid field moves keyboard focus to that field's real
//! [`FocusNode`] instead of only reporting the failure.
//!
//! W17 registers [`start_form_example`] in the shared showcase and exposes
//! the `aimer_form` model through the umbrella crate.

use aimer::input::TextEditingController;
use aimer::input::button::Button;
use aimer::input::input::{InputType, TextField};
use aimer::style::{
    BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, LayoutSpacing,
    Spacing, TextAlign, TextStyle, Theme, ThemeData,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Color, Column, Container, Dimension, FocusNode, Row, State,
    StateUpdater, StatefulWidget, Text, Widget,
};

use aimer::form::{Form, FormField, InputHint, SubmitResult, email, min_length, number, required};

use crate::theme;

/// Builds the form showcase without starting an application.
pub fn form_example() -> impl Widget {
    FormExample::new()
}

/// Starts the form showcase application.
pub fn start_form_example() {
    AimerApp::start(theme::provide(form_example()));
}

/// A live page exercising text input types, validation, submit/reset,
/// dirty/touched state, and focus-on-error.
#[derive(StatefulWidget)]
pub struct FormExample {}

impl FormExample {
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for FormExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FormExampleState {
    form: Form,
    email_controller: TextEditingController,
    password_controller: TextEditingController,
    amount_controller: TextEditingController,
    email_focus: FocusNode,
    password_focus: FocusNode,
    amount_focus: FocusNode,
    last_submit: Option<String>,
    updater: StateUpdater<Self>,
}

impl FormExampleState {
    fn initial() -> Self {
        let mut form = Form::new();
        form.add_field(
            FormField::new("email", "")
                .input_hint(InputHint::Email)
                .focus_target("email")
                .validator(required())
                .validator(email()),
        )
        .expect("the example registers a unique email field");
        form.add_field(
            FormField::new("password", "")
                .input_hint(InputHint::Password)
                .focus_target("password")
                .validator(required())
                .validator(min_length(8)),
        )
        .expect("the example registers a unique password field");
        form.add_field(
            FormField::new("amount", "")
                .input_hint(InputHint::Number)
                .focus_target("amount")
                .validator(required())
                .validator(number()),
        )
        .expect("the example registers a unique amount field");

        Self {
            form,
            email_controller: TextEditingController::new(),
            password_controller: TextEditingController::new(),
            amount_controller: TextEditingController::new(),
            email_focus: FocusNode::new(),
            password_focus: FocusNode::new(),
            amount_focus: FocusNode::new(),
            last_submit: None,
            updater: StateUpdater::empty(),
        }
    }

    fn focus_for(&self, target: &str) -> Option<&FocusNode> {
        match target {
            "email" => Some(&self.email_focus),
            "password" => Some(&self.password_focus),
            "amount" => Some(&self.amount_focus),
            _ => None,
        }
    }

    /// Marks every field touched, runs every validator, and — on rejection —
    /// moves real keyboard focus to the first invalid field's own
    /// `FocusNode` rather than only reporting which field failed.
    fn submit(&mut self) {
        match self.form.submit() {
            SubmitResult::Accepted => {
                self.last_submit = Some("Submitted — every field is valid.".to_owned());
            }
            SubmitResult::Rejected { first_error } => {
                self.last_submit = Some(format!(
                    "Rejected — fix \"{first_error}\" (focus moved there)."
                ));
                let target = first_error
                    .as_str()
                    .to_owned();
                if let Some(focus) = self.focus_for(&target) {
                    focus.request_focus();
                }
            }
        }
    }

    /// Restores every field's initial value and clears touched/dirty/error
    /// state, including the visible text each `TextField` displays — the
    /// controller is separate state from the form's own value, so both must
    /// be reset together.
    fn reset(&mut self) {
        self.form.reset();
        self.email_controller
            .set_text(String::new());
        self.password_controller
            .set_text(String::new());
        self.amount_controller
            .set_text(String::new());
        self.last_submit = None;
    }

    fn field_section(
        &self,
        id: &'static str,
        label: &'static str,
        controller: TextEditingController,
        focus: FocusNode,
        input_type: InputType,
        app_theme: ThemeData,
    ) -> AnyWidget {
        let field = self.form.field(id);
        let dirty = field.is_some_and(FormField::dirty);
        let error = field
            .filter(|field| field.touched())
            .and_then(|field| {
                field
                    .errors()
                    .first()
            })
            .map(|error| {
                error
                    .message()
                    .to_owned()
            });
        let error_color = app_theme
            .primary_color
            .lighten(0.18);
        let (status, status_color) = match &error {
            Some(message) => (message.clone(), error_color),
            None if dirty => ("Edited".to_owned(), app_theme.primary_color),
            None => (String::new(), theme::muted_text(&app_theme)),
        };
        let border_color = if error.is_some() {
            error_color
        } else {
            theme::divider(&app_theme)
        };
        let field_decoration = BoxDecoration::new()
            .background_color(theme::recessed_surface(&app_theme))
            .border(BoxBorder::all(
                BorderSlice::new()
                    .style(BorderStyle::Solid)
                    .stroke(1.0)
                    .color(border_color),
            ))
            .border_radius(
                BorderRadius::new()
                    .top_left(6.0)
                    .top_right(6.0)
                    .bottom_right(6.0)
                    .bottom_left(6.0),
            );
        let focus_decoration = BoxDecoration::new()
            .background_color(theme::recessed_surface(&app_theme))
            .border(BoxBorder::all(
                BorderSlice::new()
                    .style(BorderStyle::Solid)
                    .stroke(2.0)
                    .color(if error.is_some() {
                        error_color
                    } else {
                        app_theme.primary_color
                    }),
            ))
            .border_radius(6);
        let hover_decoration = BoxDecoration::new()
            .background_color(theme::raised_surface(&app_theme))
            .border(BoxBorder::all(
                BorderSlice::new()
                    .style(BorderStyle::Solid)
                    .stroke(1.0)
                    .color(
                        app_theme
                            .primary_color
                            .with_alpha(0.62),
                    ),
            ))
            .border_radius(6);

        let changed_updater = self.updater;
        let blurred_updater = self.updater;
        let submitted_updater = self.updater;

        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(4)))
            .children([
                Text::new(label)
                    .text_style(
                        TextStyle::new()
                            .font_size(13)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                TextField::new()
                    .key(id)
                    .controller(controller)
                    .focus_node(focus)
                    .input_type(input_type)
                    .hint(label)
                    .text_style(
                        TextStyle::new()
                            .font_size(15)
                            .color(app_theme.on_surface_color),
                    )
                    .hint_style(
                        TextStyle::new()
                            .font_size(15)
                            .color(theme::muted_text(&app_theme)),
                    )
                    .decoration(field_decoration)
                    .hover_decoration(hover_decoration)
                    .focus_decoration(focus_decoration)
                    .selection_color(
                        app_theme
                            .primary_color
                            .with_alpha(0.28),
                    )
                    .padding(LayoutSpacing::all(Spacing::Px(10)))
                    .on_changed(move |value: String| {
                        changed_updater.set_state(move |state| {
                            let _ = state
                                .form
                                .set_user_value(id, value);
                        });
                    })
                    .on_blur(move |_| {
                        blurred_updater.set_state(move |state| {
                            let _ = state
                                .form
                                .mark_touched(id);
                            let _ = state
                                .form
                                .validate_field(id);
                        });
                    })
                    .on_submitted(move |_| {
                        submitted_updater.set_state(|state| state.submit());
                    })
                    .boxed(),
                Text::new(status)
                    .text_style(
                        TextStyle::new()
                            .font_size(12)
                            .color(status_color),
                    )
                    .boxed(),
            ])
            .boxed()
    }
}

impl StatefulWidget for FormExample {
    type State = FormExampleState;

    fn create_state(self) -> Self::State {
        FormExampleState::initial()
    }
}

impl State<FormExample> for FormExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let email_field = self.field_section(
            "email",
            "Email",
            self.email_controller
                .clone(),
            self.email_focus
                .clone(),
            InputType::Email,
            app_theme,
        );
        let password_field = self.field_section(
            "password",
            "Password (min 8 characters)",
            self.password_controller
                .clone(),
            self.password_focus
                .clone(),
            InputType::Password,
            app_theme,
        );
        let amount_field = self.field_section(
            "amount",
            "Amount",
            self.amount_controller
                .clone(),
            self.amount_focus
                .clone(),
            InputType::Number,
            app_theme,
        );

        let submit_updater = self.updater;
        let reset_updater = self.updater;
        let buttons = Row::new()
            .gaps(LayoutSpacing::all(Spacing::Px(12)))
            .children([
                action_button("Submit", app_theme, true, move || {
                    submit_updater.set_state(|state| state.submit());
                }),
                action_button("Reset", app_theme, false, move || {
                    reset_updater.set_state(|state| state.reset());
                }),
            ])
            .boxed();

        let status = self
            .last_submit
            .clone();
        let status_widget = status.map_or_else(
            || Text::new(String::new()).boxed(),
            |status| {
                let is_rejected = status.starts_with("Rejected");
                Container::new()
                    .padding(LayoutSpacing::all(Spacing::Px(12)))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(theme::recessed_surface(&app_theme))
                            .border(BoxBorder::all(
                                BorderSlice::new()
                                    .style(BorderStyle::Solid)
                                    .stroke(1.0)
                                    .color(if is_rejected {
                                        app_theme.primary_color
                                    } else {
                                        theme::divider(&app_theme)
                                    }),
                            ))
                            .border_radius(8),
                    )
                    .child(
                        Text::new(status).text_style(
                            TextStyle::new()
                                .font_size(14)
                                .font_weight(FontWeight::Bold)
                                .color(if is_rejected {
                                    app_theme
                                        .primary_color
                                        .lighten(0.18)
                                } else {
                                    app_theme.primary_color
                                }),
                        ),
                    )
                    .boxed()
            },
        );

        Container::new()
            .box_decoration(
                BoxDecoration::new()
                    .background_color(app_theme.surface_color)
                    .border_radius(16),
            )
            .padding(LayoutSpacing::all(Spacing::Px(24)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children([
                        Text::new("Forms and validation")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_surface_color),
                            )
                            .boxed(),
                        Text::new(
                            "A compact controlled form with explicit validation and predictable \
                             focus. Leave a field to validate it, or submit to jump to the first \
                             invalid value.",
                        )
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .wrapped()
                        .boxed(),
                        email_field,
                        password_field,
                        amount_field,
                        buttons,
                        status_widget,
                    ]),
            )
    }
}

fn action_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let background = if primary {
        app_theme.primary_color
    } else {
        Color::Transparent
    };
    let foreground = if primary {
        app_theme.on_primary_color
    } else {
        app_theme.primary_color
    };
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border(if primary {
                    BoxBorder::default()
                } else {
                    BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(1.0)
                            .color(app_theme.primary_color),
                    )
                })
                .border_radius(8),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(if primary {
                    app_theme
                        .primary_color
                        .lighten(0.08)
                } else {
                    theme::raised_surface(&app_theme)
                })
                .border_radius(8),
        )
        .press_decoration(
            BoxDecoration::new()
                .background_color(if primary {
                    app_theme
                        .primary_color
                        .darken(0.08)
                } else {
                    theme::recessed_surface(&app_theme)
                })
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(
                    LayoutSpacing::new()
                        .left(14)
                        .right(14),
                )
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(form_example());
    }

    #[test]
    fn submit_rejects_an_invalid_form_and_moves_focus_to_the_first_failure() {
        let mut state = FormExampleState::initial();
        state
            .form
            .set_user_value("email", "not-an-email")
            .unwrap();
        state
            .form
            .set_user_value("password", "short")
            .unwrap();
        state
            .form
            .set_user_value("amount", "12.5")
            .unwrap();

        state.submit();

        assert!(
            state
                .last_submit
                .as_deref()
                .is_some_and(|status| status.contains("email"))
        );
        // `request_focus` only records a pending request; a `FocusManager`
        // resolves it into `has_focus() == true` on the next real frame, which
        // this state-level unit test does not simulate. Checking the pending
        // request is what proves `submit` actually asked for focus rather than
        // only reporting which field failed.
        assert!(
            state
                .email_focus
                .request()
                .is_some()
        );
        assert!(
            state
                .password_focus
                .request()
                .is_none()
        );
        assert!(
            state
                .amount_focus
                .request()
                .is_none()
        );
    }

    #[test]
    fn submit_accepts_a_fully_valid_form() {
        let mut state = FormExampleState::initial();
        state
            .form
            .set_user_value("email", "ada@example.com")
            .unwrap();
        state
            .form
            .set_user_value("password", "correct-horse")
            .unwrap();
        state
            .form
            .set_user_value("amount", "12.5")
            .unwrap();

        state.submit();

        assert_eq!(
            state
                .last_submit
                .as_deref(),
            Some("Submitted — every field is valid.")
        );
    }

    #[test]
    fn number_input_hint_never_validates_a_non_numeric_amount() {
        // The amount field asks for a numeric keyboard, but only the
        // explicit `number()` validator decides whether the typed value is
        // accepted — matching the crate's separation of hints from
        // validation.
        let mut state = FormExampleState::initial();
        state
            .form
            .set_user_value("amount", "not-a-number")
            .unwrap();
        let _ = state
            .form
            .mark_touched("amount");
        let _ = state
            .form
            .validate_field("amount");

        assert!(
            !state
                .form
                .field("amount")
                .unwrap()
                .is_valid()
        );
    }

    #[test]
    fn reset_clears_the_form_and_every_field_controller() {
        let mut state = FormExampleState::initial();
        state
            .form
            .set_user_value("email", "ada@example.com")
            .unwrap();
        state
            .email_controller
            .set_text("ada@example.com");
        state.submit();

        state.reset();

        assert!(
            !state
                .form
                .field("email")
                .unwrap()
                .dirty()
        );
        assert!(
            !state
                .form
                .field("email")
                .unwrap()
                .touched()
        );
        assert_eq!(
            state
                .email_controller
                .value()
                .text(),
            ""
        );
        assert_eq!(state.last_submit, None);
    }
}
