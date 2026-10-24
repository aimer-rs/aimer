//! Shared harness for the integration tests: a headless build context and
//! helpers that drive a control through the real event dispatcher.
#![allow(dead_code)]

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_widget::base::{BuildContext, Vec2d, WindowHandle};
use aimer_widget::{AnyElement, Drawable, EventDispatcher, EventResult};

/// A 400 x 400 context whose constraint is seeded the way a host seeds it
/// before the first layout pass.
pub fn context() -> BuildContext<'static> {
    context_with_height(400.0)
}

/// Like [`context`], with a taller constraint for long lists.
pub fn context_with_height(height: f32) -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    let mut ctx = BuildContext::new(
        canvas,
        ResolvedSize {
            width: 400.0,
            height,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 400.0,
        max_height: height,
    };
    ctx
}

/// Drives one element tree with pointer and key events.
pub struct Driver<'a> {
    pub ctx: &'a BuildContext<'static>,
    pub element: AnyElement,
    dispatcher: EventDispatcher,
}

impl<'a> Driver<'a> {
    /// Lays `element` out and runs its first update.
    pub fn new(ctx: &'a BuildContext<'static>, element: AnyElement) -> Self {
        use aimer_widget::LayoutElement;
        // A windowed list has no rows until its first frame, so settle twice.
        for _ in 0..2 {
            let _ = element.layout(ctx);
            element.update(ctx);
        }
        Self {
            ctx,
            element,
            dispatcher: EventDispatcher::new(),
        }
    }

    fn pointer(x: f32, y: f32) -> PointerInfo {
        PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary)
    }

    fn settle(&self) {
        use aimer_widget::LayoutElement;
        self.element.update(self.ctx);
        let _ = self.element.layout(self.ctx);
        self.element.update(self.ctx);
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) -> EventResult {
        let pointer = Self::pointer(x, y);
        let result = self.dispatcher.dispatch(
            self.element.as_ref(),
            pointer.pos,
            &ElementEvent::PointerDown(pointer),
        );
        self.settle();
        result
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) -> EventResult {
        let pointer = Self::pointer(x, y);
        let result = self.dispatcher.dispatch(
            self.element.as_ref(),
            pointer.pos,
            &ElementEvent::PointerUp(pointer),
        );
        self.settle();
        result
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) -> EventResult {
        let pointer = Self::pointer(x, y);
        let result = self.dispatcher.dispatch(
            self.element.as_ref(),
            pointer.pos,
            &ElementEvent::PointerMove(pointer),
        );
        self.settle();
        result
    }

    /// A full click: down then up at the same point.
    pub fn click(&mut self, x: f32, y: f32) {
        let _ = self.pointer_down(x, y);
        let _ = self.pointer_up(x, y);
    }

    pub fn key(&mut self, key: NamedKey) -> EventResult {
        let result = self.dispatcher.dispatch(
            self.element.as_ref(),
            Vec2d::default(),
            &ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            },
        );
        self.settle();
        result
    }
}
