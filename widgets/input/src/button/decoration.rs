use super::*;

// Preserve the existing public BoxDecoration fields and portable wire schema.
// Move their contents out without cloning shadows or allocating. Custom
// decorations are erased once during configuration, never during painting.
pub(super) fn split<D: Decoration + 'static>(
    mut decoration: D,
) -> (Option<BoxDecoration>, Option<Rc<dyn Decoration>>) {
    if let Some(decoration) = (&mut decoration as &mut dyn std::any::Any)
        .downcast_mut::<BoxDecoration>()
    {
        (Some(std::mem::take(decoration)), None)
    } else {
        (None, Some(Rc::new(decoration)))
    }
}

impl<W: Widget + 'static> ButtonState<W> {
    fn custom_for(&self, hover: bool, pressed: bool) -> Option<Rc<dyn Decoration>> {
        let slot = if self.is_disabled {
            Some(3)
        } else if pressed && (self.custom_decorations[2].is_some() || self.press_decoration.is_some()) {
            Some(2)
        } else if hover && (self.custom_decorations[1].is_some() || self.hover_decoration.is_some()) {
            Some(1)
        } else {
            None
        };
        if let Some(slot) = slot {
            if let Some(custom) = &self.custom_decorations[slot] {
                return Some(custom.clone());
            }
            let explicit_box = match slot {
                1 => self.hover_decoration.is_some(),
                2 => self.press_decoration.is_some(),
                _ => self.disable_decoration.is_some(),
            };
            if explicit_box {
                return None;
            }
        }
        self.custom_decorations[0].clone()
    }

    pub(super) fn build_custom_decorations(&self) -> AnyWidget {
        let child = self.child.clone();
        if self.is_disabled {
            self.is_pressed.set(false);
            let decoration = self.custom_for(false, false)
                .unwrap_or_else(|| Rc::new(self.decoration_for(false, false)));
            return Container::new().decoration(decoration).child(child).boxed();
        }
        if self.custom_decorations[1..].iter().all(Option::is_none)
            && self.hover_decoration.is_none() && self.press_decoration.is_none()
        {
            let decoration = self.custom_decorations[0].as_ref().unwrap().clone();
            let child = Container::new().decoration(decoration).child(child);
            return self.wrap_input(child, visual::Feedback::Fixed {
                hover: self.is_hover.clone(), pressed: self.is_pressed.clone(),
            });
        }
        let decorations: [Rc<dyn Decoration>; 4] = std::array::from_fn(|index| {
            let hover = index & 1 != 0;
            let pressed = index & 2 != 0;
            self.custom_for(hover, pressed)
                .unwrap_or_else(|| Rc::new(self.decoration_for(hover, pressed)))
        });
        let builder = Rc::new(move |state: ButtonVisualState| {
            let index = usize::from(state.is_hovered) | (usize::from(state.is_pressed) << 1);
            Container::new().decoration(decorations[index].clone()).child(child.clone()).boxed()
        });
        let (child, feedback) = visual::Feedback::visual(self, builder);
        self.wrap_input(child, feedback)
    }
}

pub(super) struct ButtonDecoration {
    hover: Rc<Cell<bool>>,
    pressed: Rc<Cell<bool>>,
    // Resolve fallback lightening/darkening once per configuration build.
    // Selecting a pointer state does not clone decorations or allocate.
    decorations: [BoxDecoration; 4],
}

impl ButtonDecoration {
    pub(super) fn new<W: Widget + 'static>(state: &ButtonState<W>) -> Self {
        Self {
            hover: state.is_hover.clone(),
            pressed: state.is_pressed.clone(),
            decorations: [
                state.decoration_for(false, false),
                state.decoration_for(true, false),
                state.decoration_for(false, true),
                state.decoration_for(true, true),
            ],
        }
    }

    #[inline]
    fn selected(&self) -> usize {
        usize::from(self.hover.get()) | (usize::from(self.pressed.get()) << 1)
    }

    pub(super) fn set_hover(&self, hover: bool) {
        self.set_flag(&self.hover, hover);
    }

    pub(super) fn set_pressed(&self, pressed: bool) {
        self.set_flag(&self.pressed, pressed);
    }

    fn set_flag(&self, flag: &Cell<bool>, value: bool) {
        if flag.get() == value {
            return;
        }
        let before = &self.decorations[self.selected()];
        flag.set(value);
        let after = &self.decorations[self.selected()];
        if before == after {
            return;
        }
        let before_widths = [
            before.border.top.stroke,
            before.border.right.stroke,
            before.border.bottom.stroke,
            before.border.left.stroke,
        ];
        let after_widths = [
            after.border.top.stroke,
            after.border.right.stroke,
            after.border.bottom.stroke,
            after.border.left.stroke,
        ];
        if before_widths != after_widths {
            // Border insets can alter ancestor measurements and event bounds.
            // The mounted elements stay in place; refresh their cached geometry.
            aimer_widget::notify_element_tree_changed();
        } else if before.border_radius != after.border_radius
            || before.outline != after.outline
            || before.box_shadow != after.box_shadow
        {
            aimer_widget::notify_retained_render_structure_changed();
        }
        aimer_events::window::request_animation_frame();
    }
}

impl aimer_container::RetainedBoxDecoration for ButtonDecoration {
    #[inline]
    fn decoration(&self) -> &BoxDecoration {
        &self.decorations[self.selected()]
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.selected() as u64
    }
}
