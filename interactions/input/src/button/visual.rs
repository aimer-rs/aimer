use super::*;

/// Read-only interaction flags supplied to [`Button::build`].
///
/// Disabled buttons report neither hover nor press. A cancelled gesture clears
/// `is_pressed` without invoking the primary callback.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ButtonVisualState {
    /// Whether the pointer is over the enabled button.
    pub is_hovered: bool,
    /// Whether an enabled button has an active pointer press.
    pub is_pressed: bool,
    /// Whether all button gesture callbacks are suppressed.
    pub is_disabled: bool,
}

pub(super) type VisualBuilder = Rc<dyn Fn(ButtonVisualState) -> AnyWidget>;
type VisualUpdater = Rc<Cell<Option<StateUpdater<VisualState>>>>;

// Keep the stateful boundary inside the input wrappers: rebuilding arbitrary
// user visuals must not replace recognizers or lose a captured press.
struct Visual {
    builder: VisualBuilder,
    hover: Rc<Cell<bool>>,
    pressed: Rc<Cell<bool>>,
    disabled: bool,
    updater: VisualUpdater,
}

pub(super) struct VisualState(Visual);

impl aimer_widget::PortableWidget for Visual {}

impl Widget for Visual {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ButtonVisual", None).0.boxed()
    }
}

impl StatefulWidget for Visual {
    type State = VisualState;

    fn create_state(self) -> Self::State {
        VisualState(self)
    }
}

impl State<Visual> for VisualState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.0.updater.set(Some(updater));
    }

    fn adopt_config_from(&mut self, new: Self) {
        new.0.updater.set(self.0.updater.get());
        self.0 = new.0;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        (self.0.builder)(ButtonVisualState {
            is_hovered: !self.0.disabled && self.0.hover.get(),
            is_pressed: !self.0.disabled && self.0.pressed.get(),
            is_disabled: self.0.disabled,
        })
    }
}

#[derive(Clone)]
pub(super) enum Feedback {
    Decoration(Rc<decoration::ButtonDecoration>),
    Fixed {
        hover: Rc<Cell<bool>>,
        pressed: Rc<Cell<bool>>,
    },
    Builder {
        hover: Rc<Cell<bool>>,
        pressed: Rc<Cell<bool>>,
        updater: VisualUpdater,
    },
}

impl Feedback {
    pub(super) fn visual<W: Widget + 'static>(
        state: &ButtonState<W>, builder: VisualBuilder,
    ) -> (AnyWidget, Self) {
        let updater = Rc::new(Cell::new(None));
        let child = Visual {
            builder,
            hover: state.is_hover.clone(),
            pressed: state.is_pressed.clone(),
            disabled: state.is_disabled,
            updater: updater.clone(),
        }.boxed();
        let feedback = Self::Builder {
            hover: state.is_hover.clone(),
            pressed: state.is_pressed.clone(),
            updater,
        };
        (child, feedback)
    }

    pub(super) fn set_hover(&self, value: bool) {
        match self {
            Self::Decoration(decoration) => decoration.set_hover(value),
            Self::Fixed { hover, .. } => hover.set(value),
            Self::Builder { hover, updater, .. } => Self::set_flag(hover, updater, value),
        }
    }

    pub(super) fn set_pressed(&self, value: bool) {
        match self {
            Self::Decoration(decoration) => decoration.set_pressed(value),
            Self::Fixed { pressed, .. } => pressed.set(value),
            Self::Builder { pressed, updater, .. } => Self::set_flag(pressed, updater, value),
        }
    }

    fn set_flag(flag: &Cell<bool>, updater: &VisualUpdater, value: bool) {
        if flag.replace(value) != value {
            if let Some(updater) = updater.get() {
                updater.set_state(|_| {});
            }
        }
    }
}
