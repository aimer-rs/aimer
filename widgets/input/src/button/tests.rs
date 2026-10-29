use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::size::ResolvedSize;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_style::BoxDecoration;
use aimer_widget::base::{Color, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, ErrorWidget, EventElement, Key, LayoutElement, PortableWidget,
    Rebuildable, State, StatefulWidget, VisitorElement, Widget, dispatch_event,
};

use super::{BuildContext, Button};

#[cfg(feature = "portable-guest")]
use aimer_anteros::PropertyValue;
#[cfg(feature = "portable-guest")]
use aimer_anteros::{
    EVENT_BUTTON_DOUBLE_PRESS, EVENT_BUTTON_LONG_PRESS, EVENT_BUTTON_PRESS,
    EVENT_BUTTON_RIGHT_PRESS, PROPERTY_BUTTON_DECORATION, Version, WIDGET_BUTTON,
    WidgetDocumentView, WidgetSchemaId,
};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{
    PortableBuildContext, PortableBuildError, PortableLimits, PortableNodeId, PortableWidgetLimits,
    SourceFingerprint, StableId128,
};

/// A child that reports how often it was asked for an element.
struct Probe {
    builds: Rc<Cell<usize>>,
}

impl Widget for Probe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.builds.set(self.builds.get() + 1);
        ErrorWidget::new("probe").to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "Probe"
    }
}

impl PortableWidget for Probe {}

struct EventProbe {
    events: Rc<Cell<usize>>,
}

impl Widget for EventProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        EventProbeElement {
            events: self.events,
        }
        .boxed()
    }
}

impl PortableWidget for EventProbe {}

struct EventProbeElement {
    events: Rc<Cell<usize>>,
}

impl VisitorElement for EventProbeElement {
    fn debug_name(&self) -> &'static str {
        "ButtonEventProbe"
    }
}
impl EventElement for EventProbeElement {
    fn on_event(&self, _event: &ElementEvent) -> aimer_widget::EventResult {
        self.events.set(self.events.get() + 1);
        aimer_widget::EventResult::ignored()
    }
}
impl LayoutElement for EventProbeElement {}
impl Drawable for EventProbeElement {
    fn update(&self, _ctx: &BuildContext<'_>) {}
}
impl Rebuildable for EventProbeElement {}

#[cfg(feature = "portable-guest")]
struct PortableChild;

#[cfg(feature = "portable-guest")]
impl Widget for PortableChild {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("portable child must not use native construction")
    }
}

#[cfg(feature = "portable-guest")]
impl PortableWidget for PortableChild {
    fn to_portable_node(
        self,
        ctx: &mut PortableBuildContext,
        source: SourceFingerprint,
    ) -> Result<PortableNodeId, PortableBuildError> {
        ctx.push_node(
            WidgetSchemaId::new(77),
            Version::new(1, 0),
            None,
            source,
            &[],
            &[],
        )
    }
}

#[cfg(feature = "portable-guest")]
fn portable_context() -> PortableBuildContext {
    PortableBuildContext::new(
        3,
        5,
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
            .with_max_blob_bytes(1_024)
            .with_max_callbacks(8),
        PortableLimits::new(8, 16, 64, 128, 1_024),
    )
    .unwrap()
}

#[cfg(feature = "portable-guest")]
fn portable_source(value: u8) -> SourceFingerprint {
    SourceFingerprint::new(StableId128::from_bytes([value; 16]))
}

#[cfg(feature = "portable-guest")]
#[test]
fn portable_button_emits_exact_schema_child_and_four_distinct_routes() {
    let calls = Rc::new(Cell::new(0_u32));
    let mut context = portable_context();
    let source = portable_source(1);
    let button = Button::new()
        .on_press({
            let calls = calls.clone();
            move || calls.set(calls.get() | 1)
        })
        .on_long_press({
            let calls = calls.clone();
            move || calls.set(calls.get() | 2)
        })
        .on_double_press({
            let calls = calls.clone();
            move || calls.set(calls.get() | 4)
        })
        .on_right_press({
            let calls = calls.clone();
            move || calls.set(calls.get() | 8)
        })
        .child(PortableChild);
    let root = button.to_portable_node(&mut context, source).unwrap();
    let document = context.finish_document(root).unwrap();
    let bytes = document.encode().unwrap();
    let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
    let node = view.node(root.index()).unwrap();
    let bindings = node.callbacks().collect::<Vec<_>>();

    assert_eq!(node.widget_type(), WIDGET_BUTTON);
    assert_eq!(node.widget_schema(), Version::new(1, 0));
    assert!(node.properties().next().is_none());
    assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding.event_kind())
            .collect::<Vec<_>>(),
        vec![
            EVENT_BUTTON_PRESS,
            EVENT_BUTTON_LONG_PRESS,
            EVENT_BUTTON_DOUBLE_PRESS,
            EVENT_BUTTON_RIGHT_PRESS,
        ]
    );
    let mut ids = bindings
        .iter()
        .map(|binding| binding.callback_id())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4);

    let registry = context.callback_registry();
    for binding in bindings {
        registry
            .dispatch(
                StableId128::from_bytes(*binding.callback_id().as_bytes()),
                &mut context,
            )
            .unwrap();
    }
    assert_eq!(calls.get(), 15);
    assert!(context.take_rebuild_request());
    assert!(!context.take_rebuild_request());
}

#[cfg(feature = "portable-guest")]
#[test]
fn portable_button_identity_uses_key_or_source_and_rejects_lossy_properties() {
    let source = portable_source(2);
    let mut first = portable_context();
    let root = Button::new()
        .key("save")
        .child(PortableChild)
        .to_portable_node(&mut first, source)
        .unwrap();
    let keyed = first.finish_document(root).unwrap();
    let keyed_bytes = keyed.encode().unwrap();
    let keyed_view = WidgetDocumentView::decode(&keyed_bytes, keyed.model_limits()).unwrap();
    let keyed_id = keyed_view.node(root.index()).unwrap().key();

    let mut second = portable_context();
    let root = Button::new()
        .key("save")
        .child(PortableChild)
        .to_portable_node(&mut second, portable_source(99))
        .unwrap();
    let same_key = second.finish_document(root).unwrap();
    let same_key_bytes = same_key.encode().unwrap();
    let same_key_view =
        WidgetDocumentView::decode(&same_key_bytes, same_key.model_limits()).unwrap();
    assert_eq!(same_key_view.node(root.index()).unwrap().key(), keyed_id);

    let mut disabled = portable_context();
    let error = Button::new()
        .disabled(true)
        .child(PortableChild)
        .to_portable_node(&mut disabled, source)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "button property `disabled` has no portable lowering"
    );

    let mut decorated = portable_context();
    let root = Button::new()
        .decoration(BoxDecoration::new().background_color(Color::BLACK))
        .child(PortableChild)
        .to_portable_node(&mut decorated, source)
        .unwrap();
    let document = decorated.finish_document(root).unwrap();
    let bytes = document.encode().unwrap();
    let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
    let property = view
        .node(root.index())
        .unwrap()
        .properties()
        .find(|property| property.property_id() == PROPERTY_BUTTON_DECORATION)
        .expect("non-default Button decoration must be lowered");
    assert!(matches!(property.value(), PropertyValue::BlobRef(_)));

    let mut asynchronous = portable_context();
    let node = Button::new()
        .on_press_async(|| async {})
        .child(PortableChild)
        .to_portable_node(&mut asynchronous, source)
        .unwrap();
    asynchronous.finish_document(node).unwrap();
    let callback_id = asynchronous.callback_id_for(None, source, EVENT_BUTTON_PRESS);
    let start = asynchronous
        .callback_registry()
        .dispatch_start(callback_id, &mut asynchronous)
        .unwrap();
    assert!(matches!(
        start,
        aimer_widget::portable::PortableCallbackStart::Started { .. }
    ));
    asynchronous.run_async_microtasks();
}

fn context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize::default(),
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

/// Rebuilding a button's configuration must reach the same retained child
/// again, preserving the child's own state rather than constructing it anew.
#[tokio::test]
async fn every_rebuild_reaches_the_same_child() {
    let builds = Rc::new(Cell::new(0));
    let state = Button::new()
        .child(Probe {
            builds: Rc::clone(&builds),
        })
        .create_state();
    let ctx = context();

    state.build(&ctx).to_element(&ctx);
    state.build(&ctx).to_element(&ctx);

    assert_eq!(
        builds.get(),
        1,
        "a hover or a press must reuse the child, not rebuild it"
    );
}

#[tokio::test]
async fn button_blocks_events_from_its_child_by_default() {
    let events = Rc::new(Cell::new(0));
    let state = Button::new()
        .child(EventProbe {
            events: events.clone(),
        })
        .create_state();
    let ctx = context();
    let element = state.build(&ctx).to_element(&ctx);

    let pointer = PointerInfo::mouse(
        aimer_widget::base::Vec2d { x: 1.0, y: 1.0 },
        PointerButton::Primary,
    );
    let _ = dispatch_event(
        element.as_ref(),
        pointer.pos,
        &ElementEvent::PointerDown(pointer),
    );

    assert_eq!(events.get(), 0);
}

/// An asynchronous press handler may keep an [`Rc`] from the element tree
/// across an `await`, and its effect lands on the runtime that owns the
/// frame.
///
/// This is the whole reason the async path moved off a thread pool: a
/// handler that cannot capture a `StateUpdater` cannot change anything a
/// user would see. The bound is easy to re-tighten by accident, so it is
/// pinned here rather than left to a compile that happens to still work.
#[tokio::test]
async fn an_async_press_handler_may_hold_tree_state_across_an_await() {
    use aimer_utils::callback::CallbackExecutor;
    use aimer_venus::Venus;

    let venus = Venus::new();
    venus.install();

    let presses = Rc::new(Cell::new(0));
    let counted = Rc::clone(&presses);
    let button = Button::new()
        .child(aimer_widget::ErrorWidget::new("button"))
        .on_press_async(move || async move {
            aimer_venus::yield_now().await;
            counted.set(counted.get() + 1);
        });

    button.on_press.execute(());
    while venus.task_count() > 0 {
        venus.run_microtasks();
    }

    assert_eq!(presses.get(), 1);
}

#[test]
fn explicit_key_sets_reconciliation_identity() {
    let button = Button::new()
        .child(aimer_widget::ErrorWidget::new("button"))
        .key("platform-button");

    assert_eq!(
        Widget::key(&button),
        Some(Key::Value("platform-button".to_owned()))
    );
}

#[test]
fn state_specific_decorations_are_transferred_to_button_state() {
    let normal = BoxDecoration::new().background_color(Color::WHITE);
    let hover = BoxDecoration::new().background_color(Color::BLUE);
    let press = BoxDecoration::new().background_color(Color::RED);
    let disabled = BoxDecoration::new().background_color(Color::GRAY);

    let state = Button::new()
        .decoration(normal.clone())
        .hover_decoration(hover.clone())
        .press_decoration(press.clone())
        .disable_decoration(disabled.clone())
        .child(aimer_widget::ErrorWidget::new("button"))
        .create_state();

    assert_eq!(state.decoration, normal);
    assert_eq!(state.hover_decoration, Some(hover));
    assert_eq!(state.press_decoration, Some(press));
    assert_eq!(state.disable_decoration, Some(disabled));
}

#[test]
fn state_specific_decorations_follow_disabled_pressed_hover_precedence() {
    let normal = BoxDecoration::new().background_color(Color::WHITE);
    let hover = BoxDecoration::new().background_color(Color::BLUE);
    let press = BoxDecoration::new().background_color(Color::RED);
    let disabled = BoxDecoration::new().background_color(Color::GRAY);
    let mut state = Button::new()
        .decoration(normal.clone())
        .hover_decoration(hover.clone())
        .press_decoration(press.clone())
        .disable_decoration(disabled.clone())
        .child(aimer_widget::ErrorWidget::new("button"))
        .create_state();

    state.is_disabled = true;
    state.is_hover.set(true);
    state.is_pressed.set(true);
    assert_eq!(state.active_decoration(), disabled);

    state.is_disabled = false;
    assert_eq!(state.active_decoration(), press);

    state.is_pressed.set(false);
    assert_eq!(state.active_decoration(), hover);

    state.is_hover.set(false);
    assert_eq!(state.active_decoration(), normal);
}

#[cfg(feature = "portable-guest")]
mod portable {
    use aimer_anteros::{
        CallbackBinding, EVENT_BUTTON_DOUBLE_PRESS, EVENT_BUTTON_LONG_PRESS, EVENT_BUTTON_PRESS,
        EVENT_BUTTON_RIGHT_PRESS, Version, WIDGET_BUTTON, WidgetDocument, WidgetNode,
        WidgetSchemaId,
    };
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableNodeId,
        PortableWidgetLimits, SourceFingerprint, StableId128,
    };

    use super::*;

    struct PortableLeaf;

    impl Widget for PortableLeaf {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("portable test child must not build natively")
        }
    }

    impl PortableWidget for PortableLeaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<PortableNodeId, PortableBuildError> {
            ctx.push_node(
                WidgetSchemaId::new(99),
                Version::new(1, 0),
                None,
                source,
                &[],
                &[],
            )
        }
    }

    fn source(value: u128) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_u128(value))
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            8,
            3,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
                .with_max_blob_bytes(1_024)
                .with_max_callbacks(8),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    #[test]
    fn generated_button_lowering_matches_the_legacy_wire_shape() {
        let source = source(2);

        let mut context = context();
        let button = Button::new().child(PortableLeaf);
        let expected_ids = [
            context.callback_id_for(None, source, EVENT_BUTTON_PRESS),
            context.callback_id_for(None, source, EVENT_BUTTON_LONG_PRESS),
            context.callback_id_for(None, source, EVENT_BUTTON_DOUBLE_PRESS),
            context.callback_id_for(None, source, EVENT_BUTTON_RIGHT_PRESS),
        ];
        let child_key = aimer_anteros::StableId128::from_bytes(
            context.slot_for(None, source.child(0)).to_bytes(),
        );
        let button_key =
            aimer_anteros::StableId128::from_bytes(context.slot_for(None, source).to_bytes());
        let root = button.to_portable_node(&mut context, source).unwrap();
        let document = context.finish_document(root).unwrap();

        let callbacks = [
            CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(expected_ids[0].to_bytes()),
            ),
            CallbackBinding::new(
                EVENT_BUTTON_LONG_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(expected_ids[1].to_bytes()),
            ),
            CallbackBinding::new(
                EVENT_BUTTON_DOUBLE_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(expected_ids[2].to_bytes()),
            ),
            CallbackBinding::new(
                EVENT_BUTTON_RIGHT_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(expected_ids[3].to_bytes()),
            ),
        ];
        let children = [0];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(99), Version::new(1, 0)).key(child_key),
            WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                .key(button_key)
                .callbacks(&callbacks)
                .children(&children),
        ];
        let expected = WidgetDocument::new(8, 3, 1, &nodes, &[], &[])
            .encode(document.model_limits())
            .unwrap();

        assert_eq!(document.encode().unwrap(), expected);
        let registry = context.callback_registry();
        for callback_id in expected_ids {
            registry.dispatch(callback_id, &mut context).unwrap();
        }
    }

    #[test]
    fn button_lowers_exact_schema_child_and_four_distinct_callback_routes() {
        let counts = Rc::new([Cell::new(0), Cell::new(0), Cell::new(0), Cell::new(0)]);
        let mut button = Button::new().key("stable-button");
        let press = counts.clone();
        button = button.on_press(move || press[0].set(press[0].get() + 1));
        let long = counts.clone();
        button = button.on_long_press(move || long[1].set(long[1].get() + 1));
        let double = counts.clone();
        button = button.on_double_press(move || double[2].set(double[2].get() + 1));
        let right = counts.clone();
        let button = button
            .on_right_press(move || right[3].set(right[3].get() + 1))
            .child(PortableLeaf);
        let widget_source = source(1);
        let mut ctx = context();
        let expected_ids = [
            ctx.callback_id_for(
                Widget::key(&button).as_ref(),
                widget_source,
                EVENT_BUTTON_PRESS,
            ),
            ctx.callback_id_for(
                Widget::key(&button).as_ref(),
                widget_source,
                EVENT_BUTTON_LONG_PRESS,
            ),
            ctx.callback_id_for(
                Widget::key(&button).as_ref(),
                widget_source,
                EVENT_BUTTON_DOUBLE_PRESS,
            ),
            ctx.callback_id_for(
                Widget::key(&button).as_ref(),
                widget_source,
                EVENT_BUTTON_RIGHT_PRESS,
            ),
        ];

        let root = button.to_portable_node(&mut ctx, widget_source).unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view =
            aimer_anteros::WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        {
            assert_eq!(view.root_node(), 1);
            let node = view.node(1).unwrap();
            assert_eq!(node.widget_type(), WIDGET_BUTTON);
            assert_eq!(node.widget_schema(), Version::new(1, 0));
            assert_eq!(node.properties().len(), 0);
            assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
            let callbacks: Vec<_> = node.callbacks().collect();
            assert_eq!(callbacks.len(), 4);
            assert_eq!(
                callbacks
                    .iter()
                    .map(|binding| binding.callback_id())
                    .collect::<Vec<_>>(),
                expected_ids.map(|id| aimer_anteros::StableId128::from_bytes(id.to_bytes())),
            );
        }
        let registry = ctx.take_callback_registry();
        for id in expected_ids {
            registry.dispatch(id, &mut ctx).unwrap();
        }
        assert_eq!(
            counts.iter().map(Cell::get).collect::<Vec<_>>(),
            vec![1, 1, 1, 1]
        );
    }

    #[test]
    fn callback_identity_prefers_explicit_key_and_otherwise_uses_source() {
        let ctx = context();
        let key = Key::Value("button".into());
        assert_eq!(
            ctx.callback_id_for(Some(&key), source(1), EVENT_BUTTON_PRESS),
            ctx.callback_id_for(Some(&key), source(2), EVENT_BUTTON_PRESS),
        );
        assert_ne!(
            ctx.callback_id_for(None, source(1), EVENT_BUTTON_PRESS),
            ctx.callback_id_for(None, source(2), EVENT_BUTTON_PRESS),
        );
        assert_ne!(
            ctx.callback_id_for(Some(&key), source(1), EVENT_BUTTON_PRESS),
            ctx.callback_id_for(Some(&key), source(1), EVENT_BUTTON_LONG_PRESS),
        );
    }

    #[test]
    fn disabled_and_state_specific_decorations_are_diagnosed_while_async_lowering_is_retained() {
        let mut ctx = context();
        let disabled = Button::new()
            .disabled(true)
            .child(PortableLeaf)
            .to_portable_node(&mut ctx, source(3))
            .unwrap_err();
        assert!(matches!(
            disabled,
            PortableBuildError::UnsupportedProperty {
                widget: "Button",
                property: "disabled",
                ..
            }
        ));

        let mut ctx = context();
        let decorated = Button::new()
            .decoration(BoxDecoration::new().background_color(Color::WHITE))
            .child(PortableLeaf)
            .to_portable_node(&mut ctx, source(4))
            .unwrap();
        ctx.finish_document(decorated).unwrap();

        let mut ctx = context();
        let asynchronous = Button::new()
            .on_press_async(|| async {})
            .child(PortableLeaf)
            .to_portable_node(&mut ctx, source(5))
            .unwrap();
        ctx.finish_document(asynchronous).unwrap();
        let callback_id = ctx.callback_id_for(None, source(5), EVENT_BUTTON_PRESS);
        assert!(matches!(
            ctx.callback_registry()
                .dispatch_start(callback_id, &mut ctx),
            Ok(aimer_widget::portable::PortableCallbackStart::Started { .. })
        ));
        ctx.run_async_microtasks();
    }
}

mod retained;
mod custom;
mod no_decor;
mod gestures;
mod semantics;
