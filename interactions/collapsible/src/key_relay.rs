use std::rc::Rc;

use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_widget::base::{BuildContext, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, EventTreeRole, LayoutElement,
    PortableWidget, Rebuildable, RequiredChild, VisitorElement, Widget,
};

/// A small focused-key seam used by [`crate::CollapsibleList`].
pub(crate) struct KeyRelay<W = RequiredChild> {
    child: W,
    on_activate: Rc<dyn Fn() -> bool>,
}

impl KeyRelay {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            child: RequiredChild,
            on_activate: Rc::new(|| false),
        }
    }

    #[inline]
    pub(crate) fn on_activate(mut self, on_activate: impl Fn() -> bool + 'static) -> Self {
        self.on_activate = Rc::new(on_activate);
        self
    }

    #[inline]
    pub(crate) fn child<C: Widget>(self, child: C) -> KeyRelay<C> {
        KeyRelay {
            child,
            on_activate: self.on_activate,
        }
    }
}

impl<W: Widget + 'static> PortableWidget for KeyRelay<W> {}

impl<W: Widget + 'static> Widget for KeyRelay<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawKeyRelay {
            child: self.child.to_element(ctx),
            on_activate: self.on_activate,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "CollapsibleListKeyRelay"
    }
}

struct RawKeyRelay {
    child: AnyElement,
    on_activate: Rc<dyn Fn() -> bool>,
}

impl VisitorElement for RawKeyRelay {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "CollapsibleListKeyRelay"
    }
}

impl Rebuildable for RawKeyRelay {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn is_carry_state(&self) -> bool {
        self.child.is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.child.with_rebuild_context(ctx, callback);
    }
}

impl Drawable for RawKeyRelay {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

impl LayoutElement for RawKeyRelay {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.event_tree_bounds()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl EventElement for RawKeyRelay {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if is_activation_key(event) && (self.on_activate)() {
            EventResult::consumed()
        } else {
            EventResult::ignored()
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

fn is_activation_key(event: &ElementEvent) -> bool {
    match event {
        ElementEvent::KeyInput {
            key: NamedKey::Enter,
            action: KeyAction::Pressed,
            ..
        } => true,
        ElementEvent::KeyInput {
            key: NamedKey::Other(name),
            action: KeyAction::Pressed,
            ..
        } => name.eq_ignore_ascii_case("space") || name == " ",
        ElementEvent::CharInput {
            ch: ' ',
            action: KeyAction::Pressed,
            ..
        } => true,
        ElementEvent::TextInput {
            text,
            action: KeyAction::Pressed,
            ..
        } => text == " ",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::is_activation_key;
    use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};

    use super::RawKeyRelay;
    use aimer_widget::base::BuildContext;
    use aimer_widget::{
        Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
    };

    struct Child;

    impl VisitorElement for Child {
        fn debug_name(&self) -> &'static str {
            "KeyRelayTestChild"
        }
    }

    impl EventElement for Child {}

    impl LayoutElement for Child {}

    impl Drawable for Child {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for Child {}

    #[test]
    fn key_relay_is_an_indexed_target() {
        let relay = RawKeyRelay {
            child: Child.boxed(),
            on_activate: Rc::new(|| false),
        };

        assert_eq!(
            relay.event_tree_role(),
            aimer_widget::EventTreeRole::IndexedTarget
        );
        assert_eq!(relay.event_tree_bounds(), None);
    }

    #[test]
    fn activation_accepts_enter_and_space_press_only() {
        let modifiers = Modifiers::default();
        assert!(is_activation_key(&ElementEvent::KeyInput {
            key: NamedKey::Enter,
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        }));
        assert!(is_activation_key(&ElementEvent::KeyInput {
            key: NamedKey::Other("Space".into()),
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        }));
        assert!(is_activation_key(&ElementEvent::CharInput {
            ch: ' ',
            action: KeyAction::Pressed,
            modifiers,
        }));
        assert!(!is_activation_key(&ElementEvent::KeyInput {
            key: NamedKey::Enter,
            action: KeyAction::Repeat,
            modifiers: Modifiers::default(),
        }));
        assert!(!is_activation_key(&ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        }));
    }
}
