extern crate self as aimer;

pub use aimer_accessibility as accessibility;
pub use aimer_anteros as anteros;
#[cfg(target_arch = "wasm32")]
pub use aimer_anteros::WasmCapabilityTransport;
pub use aimer_assets::img_widget::image_widget::Image;
pub use aimer_assets::img_widget::source::ImageSource;
pub use aimer_assets::{
    FontError, FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight, *,
};
pub use aimer_attribute::dimension::{Dimension, NumberExt};
pub use aimer_attribute::position::Vec2d;
pub use aimer_attribute::size::{ResolvedSize, Size};
pub use aimer_canvas as canvas;
pub use aimer_collapsible as collapsible;
pub use aimer_collapsible::{CollapsibleList, ListBody, ListHeader};
pub use aimer_color::prelude::*;
pub use aimer_container::*;
pub use aimer_ctxmenu::{
    ContextMenu, ContextMenuDismiss, ContextMenuItem, ContextMenuRows, ContextMenuShape,
    ContextMenuStyle,
};

pub use aimer_grid as grid;

#[cfg(feature = "cupid")]
pub use aimer_cupid as cupid;
pub use aimer_data_view as data_view;
pub use aimer_decoration as decoration;
pub use aimer_dnd as dnd;
pub use aimer_dnd::{
    DragAxis, DragOverlay, DragPayload, DragSession, DragStartMode, DragTarget, DragTargetState,
    Draggable, DropZone, FileDrop,
};
pub use aimer_events as events;
pub use aimer_events::element::ElementEvent;
pub use aimer_device as device;
pub use aimer_feedback as feedback;
pub use aimer_flex::*;
pub use aimer_focus as focus;
pub use aimer_focus::{
    FocusBehavior, FocusCallback, FocusCandidate, FocusManager, FocusNode, FocusTransition,
    FocusTrap, FocusTrapId, active_focus_trap,
};
pub use aimer_form as form;
pub use aimer_haptics as haptics;
pub use aimer_haptics::{HapticFeedback, HapticKind};
pub use aimer_i18n as i18n;
pub use aimer_indicator as indicator;
pub use aimer_input as input;
pub use aimer_macro::{
    PortableValue, Router, StatefulWidget, StatelessWidget, Theme, assets, capability, declare_assets, key, main,
};
#[cfg(feature = "markdown")]
pub use aimer_markdown::{
    Alignment as MarkdownAlignment, Block as MarkdownBlock, BlockRule as MarkdownBlockRule,
    BlockSyntax as MarkdownBlockSyntax, CustomBlock as MarkdownCustomBlock,
    CustomBlockBuilder as MarkdownCustomBlockBuilder, CustomBlockData as MarkdownCustomBlockData,
    CustomBlockInput as MarkdownCustomBlockInput, CustomInline as MarkdownCustomInline,
    CustomInlineBuilder as MarkdownCustomInlineBuilder,
    CustomInlineData as MarkdownCustomInlineData, Document as MarkdownDocument,
    ImageResolver as MarkdownImageResolver, Inline as MarkdownInline,
    InlineRule as MarkdownInlineRule, InlineSyntax as MarkdownInlineSyntax,
    LinkHandler as MarkdownLinkHandler, MarkdownError, MarkdownImage, MarkdownTheme,
    MarkdownViewer,
};
pub use aimer_media as media;
pub use aimer_navigation as navigation;
pub use aimer_persist as persist;
pub use aimer_picker as picker;
pub use aimer_range as range;
pub use aimer_selection as selection;
pub use aimer_shape as shape;
pub use aimer_storage as storage;
pub mod modal {
    pub use aimer_modal::{
        Anchor, AnchorHandle, Floating, FloatingAlign, FloatingPlacement, FloatingSide, Modal,
        ModalAnimation, ModalController, ModalHandle, ModalHost, ModalId, OverflowPolicy,
        OverlayLayer, OverlayLayerHandle, PlacementSpec, resolve_placement,
    };
}
#[cfg(feature = "provider")]
pub use aimer_provider::{
    NotifierProvider, Provider, ProviderContext, ProviderHandle, StoreProvider,
};
#[cfg(not(aimer_portable_guest))]
pub use aimer_quiver as quiver;
#[cfg(not(aimer_portable_guest))]
pub use aimer_quiver::frame_stats;
#[cfg(not(aimer_portable_guest))]
pub use aimer_quiver::{
    self, AimerApp, ExitPolicy, FIRST_FRAME_RENDERED_EVENT, HeadlessAimerApp, HeadlessOptions,
    NewWindow, RenderMode, Window, WindowAttr, WindowId, set_first_frame_rendered_callback,
};

#[cfg(feature = "vault")]
pub use aimer_vault as vault;
#[cfg(feature = "passkey")]
pub use aimer_passkey as passkey;
#[cfg(feature = "icons")]
pub use aimer_lucide as icons;
#[cfg(feature = "icons")]
pub use aimer_lucide::Icons;
pub use aimer_screen as screen;
pub use aimer_custom_paint::{CanvasPainter, CustomPaint};
pub use aimer_scroll::*;
pub use aimer_space::*;
#[cfg(feature = "svg")]
pub use aimer_svg as svg;
#[cfg(feature = "svg")]
pub use aimer_svg::{
    RawSvg, StaticSvg, Svg, SvgAspectAlign, SvgAspectMode, SvgCallback, SvgColor, SvgDiagnostic,
    SvgDocument, SvgError, SvgFillRule, SvgFitError, SvgFitPolicy, SvgGradient, SvgGradientStop,
    SvgGradientUnits, SvgHit, SvgLimits, SvgLoadState, SvgLoader, SvgNodeId, SvgNodeMetadata,
    SvgNodePaint, SvgPaint, SvgPath, SvgPreserveAspectRatio, SvgSelector, SvgSource,
    SvgSpreadMethod, SvgStyle, SvgTransform, SvgViewBox,
};
pub use aimer_text::{RichText, SelectionArea, SpanStyle, Text, TextButton, TextSpan};
pub use aimer_text::{
    TextAccessibilityCaret, TextAccessibilityCluster, TextAccessibilityLine,
    TextAccessibilitySelectionRect, TextAccessibilitySnapshot,
};
pub use aimer_venus as venus;
pub use aimer_venus::{TaskScope, Venus, yield_if_over_budget, yield_now};
#[cfg(feature = "portable-guest")]
#[doc(hidden)]
pub use aimer_wasm_guest;
pub use aimer_widget::base::BuildContext;
pub use aimer_widget::{self, Key, State, StatefulWidget, StatelessWidget, Widget, *};

pub use aimer_native as native;

pub mod widget {
    pub use aimer_widget::base::BuildContext;
    pub use aimer_widget::{State, StatefulWidget, StatelessWidget, Widget, *};
}

pub use aimer_animation as animation;

#[cfg(test)]
mod public_api_tests {
    fn assert_reachable<T>() {}

    #[test]
    fn completion_namespaces_are_reachable_from_the_umbrella_crate() {
        assert_reachable::<crate::dnd::AutoScroller>();
        assert_reachable::<crate::dnd::FileDropPolicy>();
        assert_reachable::<crate::dnd::ReorderableList<u64>>();
        assert_reachable::<crate::collapsible::ListHeader>();
        assert_reachable::<crate::CollapsibleList>();
    }

    #[cfg(feature = "svg")]
    #[test]
    fn svg_completion_namespace_exposes_fit_and_paint_models() {
        assert_reachable::<crate::svg::SvgFitPolicy>();
        assert_reachable::<crate::svg::SvgGradient>();
        assert_reachable::<crate::svg::SvgNodePaint>();
        assert_reachable::<crate::svg::SvgViewBox>();
    }
}

// Macro re-export
pub mod macros {
    pub use aimer_macro::{
        PortableValue, Router, StatefulWidget, StatelessWidget, Theme, assets, capability, declare_assets,
        key, main, widget,
    };
}

// Styling re-export
pub mod style {
    pub use aimer_animation::primitives::curve::{AnyCurve, Curve, curves};
    pub use aimer_animation::{AnimationEffect, AnimationStatus};
    pub use aimer_color::prelude::{Color, Colors};
    pub use aimer_flex::{BoxAlignment, FlexDirection, OverflowBehavior};
    pub use aimer_style::*;
}

// utils re-export
pub mod console {
    pub use aimer_utils::*;
}

// wasm dependencies
pub use aimer_provider as provider;
pub use aimer_router as router;
#[doc(hidden)]
pub use wasm_bindgen;

pub use aimer_std::case;
pub mod share {
    pub use aimer_rubick::{RcProjectExt, ShareRef, Shared, Weak};
}

pub mod rubick {
    pub use aimer_rubick::{ErasedFrom, Rubick};
}

#[cfg(test)]
mod tests {
    mod async_scroll_headless {
        //! Regression coverage for a page whose scroll view is rebuilt by an
        //! asynchronous request.
        //!
        //! `website/src/screen/blog_detail.rs` puts the `AsyncBuilder` at the root of
        //! the page, so the `Scrollable` and every container below it is rebuilt when
        //! the post arrives. The scroll range is derived from what those containers
        //! report, so a measurement taken while the loading indicator was on screen
        //! leaves the page unscrollable with its content already painted.

        use std::thread::sleep;
        use std::time::Duration;

        use aimer::{
            AnyWidget, AsyncBuilder, AsyncSnapshot, BoxAlignment, Column, Container, Key,
            ScrollAxis, ScrollController, Scrollable, SizedBox, Widget,
        };
        use aimer_quiver::AimerApp;

        /// Height of the loaded post: far taller than the headless viewport.
        const CONTENT_HEIGHT: u32 = 4_000;

        /// How long the stand-in request takes to answer.
        ///
        /// The point of these tests is what happens when the loading state is *replaced*,
        /// so the loading state has to be on screen for the first frame. An immediately
        /// ready future does not guarantee that: the runtime is free to complete it
        /// before the frame is drawn, and under a loaded machine it does, which turns
        /// the first assertion into a coin toss. A request that takes a moment makes the
        /// ordering the test is about an actual fact rather than a race.
        const REQUEST_LATENCY: Duration = Duration::from_millis(20);

        /// Builds the page for one snapshot, mirroring the vertical branch of the blog
        /// detail screen: the `Scrollable` is part of what the request rebuilds, so the
        /// loading state and the post are measured by two different container elements
        /// standing in the same place.
        fn detail_page(
            snapshot: &AsyncSnapshot<u32, String>,
            controller: &ScrollController,
        ) -> AnyWidget {
            let (content, key) = match snapshot {
                AsyncSnapshot::Waiting => {
                    (SizedBox::new().height(40).boxed(), Key::from("first-post"))
                }
                AsyncSnapshot::Error(_) => (SizedBox::new().height(40).boxed(), Key::unique()),
                AsyncSnapshot::Data(height) => (
                    SizedBox::new().height(*height).boxed(),
                    Key::from("first-post"),
                ),
            };

            Container::new()
                .box_child(
                    Scrollable::new()
                        .key(key)
                        .controller(controller.clone())
                        .axis(ScrollAxis::Vertical)
                        .child(
                            Container::new().child(
                                Column::new()
                                    .horizontal_alignment(BoxAlignment::Start)
                                    .children([
                                        Column::new()
                                            .horizontal_alignment(BoxAlignment::Start)
                                            .children([
                                                SizedBox::new().height(28).boxed(),
                                                SizedBox::new().height(32).boxed(),
                                                content,
                                            ])
                                            .boxed(),
                                        SizedBox::new().height(48).boxed(),
                                    ]),
                            ),
                        ),
                )
                .boxed()
        }

        #[test]
        fn a_page_rebuilt_by_a_completed_request_can_be_scrolled() {
            let controller = ScrollController::new();
            let attached = controller.clone();
            let page = AsyncBuilder::new()
                .request_key("first-post".to_owned())
                .future(|| async {
                    sleep(REQUEST_LATENCY);
                    Ok::<_, String>(CONTENT_HEIGHT)
                })
                .child(move |snapshot| detail_page(snapshot, &attached));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();
            assert_eq!(
                controller.max_extent().y,
                0.0,
                "the loading state is shorter than the viewport"
            );

            // Give the request a chance to complete, then draw the frame that swaps the
            // loading state for the post.
            sleep(Duration::from_millis(100));
            app.render_frame();
            app.render_frame();

            assert!(
                controller.max_extent().y > 0.0,
                "the loaded post is {CONTENT_HEIGHT}px tall but the page reports no scroll range"
            );
        }

        /// Control: the shape of `website/src/screen/blog.rs`, where the `Scrollable`
        /// lives above the `AsyncBuilder` and is never rebuilt.
        #[test]
        fn a_stable_scroll_view_over_a_completed_request_can_be_scrolled() {
            let controller = ScrollController::new();
            let content = AsyncBuilder::new()
                .request_key("first-post".to_owned())
                .future(|| async { Ok::<_, String>(CONTENT_HEIGHT) })
                .child(|snapshot| match snapshot {
                    AsyncSnapshot::Data(height) => SizedBox::new().height(*height).boxed(),
                    _ => SizedBox::new().height(40).boxed(),
                })
                .boxed();

            let page = Container::new().box_child(
                Scrollable::new()
                    .controller(controller.clone())
                    .axis(ScrollAxis::Vertical)
                    .child(
                        Container::new().child(
                            Column::new()
                                .horizontal_alignment(BoxAlignment::Start)
                                .children([
                                    SizedBox::new().height(28).boxed(),
                                    content,
                                    SizedBox::new().height(48).boxed(),
                                ]),
                        ),
                    ),
            );

            let mut app = AimerApp::start_headless(page);
            app.render_frame();
            sleep(Duration::from_millis(100));
            app.render_frame();
            app.render_frame();

            assert!(controller.max_extent().y > 0.0);
        }
    }

    mod box_child {
        use aimer::input::button::Button;
        use aimer::input::gesture::gesture_detector::GestureDetector;
        use aimer::input::mouse_region::MouseRegion;
        use aimer::style::AnimatedTheme;
        use aimer::{
            Align, AnyWidget, AspectRatio, Container, Expanded, Opacity, Positioned, Scrollable,
            SizedBox, ZeroSizedBox,
        };

        #[cfg(feature = "provider")]
        use aimer::{Provider, StoreProvider};

        fn assert_any_widget(_: AnyWidget) {}

        #[test]
        fn box_child_erases_every_single_child_widget() {
            assert_any_widget(Expanded::new().box_child(ZeroSizedBox));
            assert_any_widget(Scrollable::new().box_child(ZeroSizedBox));
            assert_any_widget(AspectRatio::new().box_child(ZeroSizedBox));
            assert_any_widget(Container::new().box_child(ZeroSizedBox));
            assert_any_widget(Opacity::new().box_child(ZeroSizedBox));
            assert_any_widget(SizedBox::new().box_child(ZeroSizedBox));
            assert_any_widget(Align::new().box_child(ZeroSizedBox));
            assert_any_widget(Positioned::new().box_child(ZeroSizedBox));
            assert_any_widget(Button::new().box_child(ZeroSizedBox));
            assert_any_widget(GestureDetector::new().dyn_child(ZeroSizedBox));
            assert_any_widget(MouseRegion::new().box_child(ZeroSizedBox));
            #[cfg(feature = "provider")]
            {
                assert_any_widget(Provider::new().create(|| 0_u8).box_child(ZeroSizedBox));
                assert_any_widget(
                    StoreProvider::<u8, u8>::new()
                        .create(|| 0)
                        .reducer(|state, action| *state = action)
                        .box_child(ZeroSizedBox),
                );
            }
            assert_any_widget(AnimatedTheme::new().box_child(ZeroSizedBox));
        }
    }

    mod capability_macro {
        use aimer::anteros::CapabilityResult;
        use aimer::capability;

        #[capability(name = "haptics", id = "com.example.haptics", abi = 1, since = "1.0.0")]
        trait Haptics {
            fn trigger(&self, kind: u32) -> CapabilityResult<bool>;
        }

        struct NativeHaptics;

        impl Haptics for NativeHaptics {
            fn trigger(&self, kind: u32) -> CapabilityResult<bool> {
                Ok(kind == 7)
            }
        }

        #[test]
        fn umbrella_capability_macro_preserves_the_native_provider_api() {
            assert!(NativeHaptics.trigger(7).unwrap());
            assert_eq!(HapticsCapability::CANONICAL_ID, "com.example.haptics");
            assert_eq!(HapticsCapability::ABI_MAJOR, 1);
        }
    }

    mod container_crate_split {
        use std::any::TypeId;

        use aimer_container::{AspectRatio, Container, Opacity, SizedBox, ZeroSizedBox};
        use aimer_flex::{
            BoxAlignment, Column, Expanded, Flex, FlexDirection, OverflowBehavior, Row,
        };
        use aimer_grid::{
            Grid, GridAlignment, GridError, GridItem, GridOverflow, GridPlacement, GridTrack,
        };
        use aimer_scroll::{
            DragMode, ScrollAxis, ScrollBar, ScrollBehavior, ScrollController, Scrollable,
        };
        use aimer_space::{Align, Alignment, Positioned, Stack};

        #[test]
        fn split_crates_expose_their_widget_families() {
            let _ = TypeId::of::<AspectRatio>();
            let _ = TypeId::of::<Container>();
            let _ = TypeId::of::<Opacity>();
            let _ = TypeId::of::<SizedBox>();
            let _ = TypeId::of::<ZeroSizedBox>();

            let _ = TypeId::of::<BoxAlignment>();
            let _ = TypeId::of::<Column>();
            let _ = TypeId::of::<Expanded>();
            let _ = TypeId::of::<Flex>();
            let _ = TypeId::of::<FlexDirection>();
            let _ = TypeId::of::<OverflowBehavior>();
            let _ = TypeId::of::<Row>();

            let _ = TypeId::of::<Grid>();
            let _ = TypeId::of::<GridAlignment>();
            let _ = TypeId::of::<GridError>();
            let _ = TypeId::of::<GridItem<ZeroSizedBox>>();
            let _ = TypeId::of::<GridOverflow>();
            let _ = TypeId::of::<GridPlacement>();
            let _ = TypeId::of::<GridTrack>();

            let _ = TypeId::of::<DragMode>();
            let _ = TypeId::of::<ScrollAxis>();
            let _ = TypeId::of::<ScrollBar>();
            let _ = TypeId::of::<ScrollBehavior>();
            let _ = TypeId::of::<ScrollController>();
            let _ = TypeId::of::<Scrollable>();

            let _ = TypeId::of::<Align>();
            let _ = TypeId::of::<Alignment>();
            let _ = TypeId::of::<Positioned>();
            let _ = TypeId::of::<Stack>();
        }

        #[test]
        fn split_crates_preserve_public_submodule_paths() {
            let _ = TypeId::of::<aimer_flex::row_column::Row>();
            let _ = TypeId::of::<aimer_scroll::controller::ScrollController>();
            let _ = TypeId::of::<aimer_space::align::Alignment>();
        }
    }

    mod drag_and_drop_headless {
        //! Dragging one widget onto another, through the real element tree.
        //!
        //! Everything here is driven by window events, the same ones a mouse produces:
        //! nothing reaches into the drag session directly. That is deliberate — the
        //! interesting claims are about *routing* (does the drop find the topmost
        //! target that wants it?) and routing is exactly what a unit test of the
        //! session cannot check.

        use std::cell::RefCell;
        use std::rc::Rc;

        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
        use aimer::{AimerApp, AnyWidget, Container, Positioned, Row, SizedBox, Stack, Widget};
        use aimer_dnd::{DragTarget, DragTargetState, Draggable};

        /// The value carried by a drag in these tests.
        #[derive(Clone, Debug, PartialEq)]
        struct CardId(u32);

        /// A payload no target here understands.
        #[derive(Clone, Debug, PartialEq)]
        struct Unrelated(u32);

        /// Records what a target accepted.
        type Accepted = Rc<RefCell<Vec<CardId>>>;

        fn tile(size: u32) -> AnyWidget {
            SizedBox::new().width(size).height(size).boxed()
        }

        /// A draggable 100x100 card carrying `id`.
        fn card(id: CardId) -> AnyWidget {
            Draggable::new()
                .data(id)
                .feedback(|| tile(100))
                .child(tile(100))
                .boxed()
        }

        /// A 100x100 target that records everything it accepts.
        fn column(accepted: Accepted) -> AnyWidget {
            DragTarget::<CardId>::new()
                .on_accept(move |id: CardId| accepted.borrow_mut().push(id))
                .child(|_state: DragTargetState| tile(100))
                .boxed()
        }

        /// Moves the cursor and presses, drags to `to`, and releases there.
        fn drag<W: Widget + 'static>(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
            from: (f64, f64),
            to: (f64, f64),
        ) {
            let device_id = DeviceId::dummy();

            app.send_window_event(WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(from.0, from.1),
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.send_window_event(WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(to.0, to.1),
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button: MouseButton::Left,
            });
            // The drop is settled on the frame the release asked for: whether a target
            // took the payload is only knowable after the routed drop pass.
            app.render_frame();
        }

        /// A card at `0..100` and a target at `100..200`, side by side.
        #[test]
        fn a_card_dropped_on_a_target_is_accepted_once() {
            let accepted: Accepted = Rc::new(RefCell::new(Vec::new()));
            let page = Container::new()
                .child(Row::new().children([card(CardId(1)), column(accepted.clone())]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            drag(&mut app, (50.0, 50.0), (150.0, 50.0));

            assert_eq!(*accepted.borrow(), vec![CardId(1)]);
        }

        /// A press that never travels past the tap slop is a tap, not a drag.
        #[test]
        fn a_press_that_does_not_travel_drops_nothing() {
            let accepted: Accepted = Rc::new(RefCell::new(Vec::new()));
            let page = Container::new()
                .child(Row::new().children([card(CardId(1)), column(accepted.clone())]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            drag(&mut app, (50.0, 50.0), (54.0, 52.0));

            assert!(accepted.borrow().is_empty(), "a tap must not drop anything");
        }

        /// A target bound to one payload type never sees a drag carrying another.
        #[test]
        fn a_payload_of_another_type_never_reaches_the_target() {
            let accepted: Accepted = Rc::new(RefCell::new(Vec::new()));
            let unrelated = Draggable::new()
                .data(Unrelated(1))
                .feedback(|| tile(100))
                .child(tile(100))
                .boxed();
            let page =
                Container::new().child(Row::new().children([unrelated, column(accepted.clone())]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            drag(&mut app, (50.0, 50.0), (150.0, 50.0));

            assert!(accepted.borrow().is_empty());
        }

        /// A predicate that says no keeps the payload, and the target stays silent.
        #[test]
        fn a_refused_drop_never_reaches_on_accept() {
            let accepted: Accepted = Rc::new(RefCell::new(Vec::new()));
            let recorder = accepted.clone();
            let locked = DragTarget::<CardId>::new()
                .will_accept(|id: &CardId| id.0 != 1)
                .on_accept(move |id: CardId| recorder.borrow_mut().push(id))
                .child(|_state: DragTargetState| tile(100))
                .boxed();
            let page = Container::new().child(Row::new().children([card(CardId(1)), locked]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            drag(&mut app, (50.0, 50.0), (150.0, 50.0));

            assert!(
                accepted.borrow().is_empty(),
                "a target that refused the payload must not receive it"
            );
        }

        /// Two targets in the same place: the drop belongs to the one on top.
        #[test]
        fn the_topmost_target_takes_the_drop() {
            let below: Accepted = Rc::new(RefCell::new(Vec::new()));
            let above: Accepted = Rc::new(RefCell::new(Vec::new()));

            let page = Container::new().child(
                Stack::new().children([
                    Positioned::new()
                        .left(0.0)
                        .top(0.0)
                        .child(card(CardId(3)))
                        .boxed(),
                    Positioned::new()
                        .left(150.0)
                        .top(0.0)
                        .child(column(below.clone()))
                        .boxed(),
                    Positioned::new()
                        .left(150.0)
                        .top(0.0)
                        .child(column(above.clone()))
                        .boxed(),
                ]),
            );

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            drag(&mut app, (50.0, 50.0), (200.0, 50.0));

            assert!(below.borrow().is_empty(), "a covered target took the drop");
            assert_eq!(*above.borrow(), vec![CardId(3)]);
        }
    }

    mod file_drop_headless {
        //! Files dragged in from the operating system, through the real element tree.
        //!
        //! winit reports a file drag one file at a time and attaches no cursor position
        //! to any of it. Both of those are load-bearing here: the coalescing test fails
        //! if a five-file drag produces five callbacks, and the two-zone test fails if
        //! the position is not plumbed through, because the wrong zone answers.

        use std::cell::RefCell;
        use std::path::PathBuf;
        use std::rc::Rc;

        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, WindowEvent};
        use aimer::{
            AimerApp, AnyWidget, Container, DragTargetState, DropZone, Row, SizedBox, Widget,
        };

        /// Every batch a zone received, in the order they arrived.
        type Batches = Rc<RefCell<Vec<Vec<PathBuf>>>>;

        /// Whether a zone is currently highlighted, sampled on its last build.
        type Highlighted = Rc<RefCell<bool>>;

        fn zone(
            batches: Batches,
            highlighted: Highlighted,
            extensions: Option<&[&str]>,
        ) -> AnyWidget {
            let zone = DropZone::new()
                .on_drop(move |paths: Vec<PathBuf>| batches.borrow_mut().push(paths));
            let zone = match extensions {
                Some(extensions) => zone.extensions(extensions.to_vec()),
                None => zone,
            };
            zone.child(move |state: DragTargetState| {
                *highlighted.borrow_mut() = state.is_hovered;
                SizedBox::new().width(100).height(100)
            })
            .boxed()
        }

        fn batches() -> Batches {
            Rc::new(RefCell::new(Vec::new()))
        }

        fn highlighted() -> Highlighted {
            Rc::new(RefCell::new(false))
        }

        /// Puts the cursor at `x`, so the file events that follow are hit-tested there.
        fn point_at<W: Widget + 'static>(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
            x: f64,
            y: f64,
        ) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
        }

        fn hover<W: Widget + 'static>(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
            path: &str,
        ) {
            app.send_window_event(WindowEvent::HoveredFile(PathBuf::from(path)));
        }

        fn drop_file<W: Widget + 'static>(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
            path: &str,
        ) {
            app.send_window_event(WindowEvent::DroppedFile(PathBuf::from(path)));
        }

        #[test]
        fn one_dropped_file_is_delivered_once() {
            let received = batches();
            let lit = highlighted();
            let page = Container::new().child(zone(received.clone(), lit.clone(), None));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            hover(&mut app, "/tmp/a.png");
            app.render_frame();
            assert!(*lit.borrow(), "a hovering file must highlight the zone");

            drop_file(&mut app, "/tmp/a.png");
            app.render_frame();

            assert_eq!(received.borrow().len(), 1);
            assert_eq!(received.borrow()[0], vec![PathBuf::from("/tmp/a.png")]);
            assert!(!*lit.borrow(), "the highlight must clear on a drop");
        }

        /// The platform reports five files as five events. The application asked for a
        /// drag, not for five drags.
        #[test]
        fn five_files_dropped_together_arrive_as_one_batch() {
            let received = batches();
            let page = Container::new().child(zone(received.clone(), highlighted(), None));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            let paths = ["a.png", "b.png", "c.png", "d.png", "e.png"];
            for path in paths {
                hover(&mut app, path);
            }
            for path in paths {
                drop_file(&mut app, path);
            }
            app.render_frame();

            assert_eq!(received.borrow().len(), 1, "one drag, one callback");
            assert_eq!(received.borrow()[0].len(), 5);
        }

        /// Two zones side by side. This is the test that fails when the file events
        /// carry no position: whichever zone answers first takes everything.
        #[test]
        fn only_the_zone_under_the_cursor_receives_the_drop() {
            let left = batches();
            let right = batches();
            let page = Container::new().child(Row::new().children([
                zone(left.clone(), highlighted(), None),
                zone(right.clone(), highlighted(), None),
            ]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 150.0, 50.0);
            hover(&mut app, "/tmp/a.png");
            drop_file(&mut app, "/tmp/a.png");
            app.render_frame();

            assert!(left.borrow().is_empty(), "the zone beside the cursor fired");
            assert_eq!(right.borrow().len(), 1);
        }

        /// A restricted zone is invisible to files it does not want: no highlight, no
        /// callback.
        #[test]
        fn a_zone_restricted_by_extension_ignores_everything_else() {
            let received = batches();
            let lit = highlighted();
            let page =
                Container::new().child(zone(received.clone(), lit.clone(), Some(&["png", "jpg"])));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            hover(&mut app, "/tmp/notes.txt");
            app.render_frame();
            assert!(!*lit.borrow(), "a filtered zone must not highlight");

            drop_file(&mut app, "/tmp/notes.txt");
            app.render_frame();
            assert!(received.borrow().is_empty());

            // The same zone still takes what it asked for.
            hover(&mut app, "/tmp/photo.PNG");
            drop_file(&mut app, "/tmp/photo.PNG");
            app.render_frame();
            assert_eq!(received.borrow().len(), 1);
        }

        /// The platform announces a file *entering* the window and then goes quiet, but
        /// the file keeps moving. The zones have to follow it: the one under it lights
        /// up, the one it left goes dark, and background leaves nothing lit.
        #[test]
        fn a_hovering_file_is_tracked_on_every_move() {
            let left_lit = highlighted();
            let right_lit = highlighted();
            let page = Container::new().child(Row::new().children([
                zone(batches(), left_lit.clone(), None),
                zone(batches(), right_lit.clone(), None),
            ]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            hover(&mut app, "/tmp/a.png");
            app.render_frame();
            assert!(*left_lit.borrow(), "the zone under the file must light up");
            assert!(!*right_lit.borrow());

            // The file travels on. The platform says nothing about it.
            point_at(&mut app, 150.0, 50.0);
            app.render_frame();
            assert!(!*left_lit.borrow(), "the zone the file left stayed lit");
            assert!(
                *right_lit.borrow(),
                "the zone the file moved onto never lit"
            );

            // And off both of them, onto the background.
            point_at(&mut app, 150.0, 250.0);
            app.render_frame();
            assert!(!*left_lit.borrow());
            assert!(!*right_lit.borrow(), "background left a zone lit");
        }

        /// Where the file was picked up is irrelevant; where it was let go is not.
        #[test]
        fn a_file_is_delivered_where_it_was_released_not_where_it_arrived() {
            let left = batches();
            let right = batches();
            let page = Container::new().child(Row::new().children([
                zone(left.clone(), highlighted(), None),
                zone(right.clone(), highlighted(), None),
            ]));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            hover(&mut app, "/tmp/a.png");
            point_at(&mut app, 150.0, 50.0);
            drop_file(&mut app, "/tmp/a.png");
            app.render_frame();

            assert!(
                left.borrow().is_empty(),
                "the zone the file only passed over fired"
            );
            assert_eq!(right.borrow().len(), 1);
            assert_eq!(right.borrow()[0], vec![PathBuf::from("/tmp/a.png")]);
        }

        /// A drag that leaves the window takes the highlight with it and leaves nothing
        /// behind for the next one.
        #[test]
        fn a_cancelled_drag_clears_the_highlight_and_the_collected_paths() {
            let received = batches();
            let lit = highlighted();
            let page = Container::new().child(zone(received.clone(), lit.clone(), None));

            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            point_at(&mut app, 50.0, 50.0);
            hover(&mut app, "/tmp/a.png");
            app.render_frame();
            assert!(*lit.borrow());

            app.send_window_event(WindowEvent::HoveredFileCancelled);
            app.render_frame();

            assert!(!*lit.borrow(), "a cancelled drag left the zone highlighted");
            assert!(received.borrow().is_empty());

            // Nothing was carried over into the next drag.
            hover(&mut app, "/tmp/b.png");
            drop_file(&mut app, "/tmp/b.png");
            app.render_frame();

            assert_eq!(received.borrow().len(), 1);
            assert_eq!(received.borrow()[0], vec![PathBuf::from("/tmp/b.png")]);
        }
    }

    mod floating_headless {
        use std::cell::Cell;
        use std::rc::Rc;

        use aimer::modal::{
            Anchor, AnchorHandle, Floating, FloatingAlign, FloatingSide, ModalHandle,
        };
        use aimer::style::{LayoutSpacing, Spacing};
        use aimer::{
            AnyElement, BuildContext, Container, Dimension, Drawable, Element, EventElement,
            LayoutElement, Rebuildable, ResolvedSize, SizedBox, VisitorElement, Widget,
        };

        /// Records where its parent placed it during the paint pass.
        #[derive(Clone)]
        struct PositionProbe {
            observed: Rc<Cell<(f32, f32)>>,
        }

        struct PositionProbeElement {
            observed: Rc<Cell<(f32, f32)>>,
            size: ResolvedSize,
        }

        impl Widget for PositionProbe {
            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                PositionProbeElement {
                    observed: self.observed.clone(),
                    size: ResolvedSize {
                        width: 100.0,
                        height: 60.0,
                    },
                }
                .boxed()
            }
        }

        impl aimer::PortableWidget for PositionProbe {}

        impl Drawable for PositionProbeElement {
            fn update(&self, ctx: &BuildContext) {
                self.observed.set(ctx.canvas.get_transform_translation());
            }
        }

        impl EventElement for PositionProbeElement {}
        impl Rebuildable for PositionProbeElement {}

        impl LayoutElement for PositionProbeElement {
            fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
                self.size
            }

            fn content_size(&self, _ctx: &BuildContext) -> ResolvedSize {
                self.size
            }
        }

        impl VisitorElement for PositionProbeElement {
            fn debug_name(&self) -> &'static str {
                "PositionProbe"
            }
        }

        /// Builds a page whose only content is a 120x40 trigger inset by 20 logical
        /// pixels, so the tracked anchor rectangle is `(20, 20, 120, 40)`.
        fn anchored_page(handle: AnchorHandle) -> impl Widget {
            Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(20)))
                .child(
                    Anchor::new().handle(handle).child(
                        Container::new()
                            .width(Dimension::Px(120.0))
                            .height(Dimension::Px(40.0))
                            .child(SizedBox::new().width(120).height(40)),
                    ),
                )
        }

        fn show_panel(
            handle: AnchorHandle,
            side: FloatingSide,
            align: FloatingAlign,
            probe: Rc<Cell<(f32, f32)>>,
        ) -> ModalHandle {
            Floating::new()
                .anchor(handle)
                .side(side)
                .align(align)
                .gap(4.0)
                .child(PositionProbe { observed: probe })
                .show()
        }

        #[test]
        fn a_panel_is_painted_below_the_rectangle_reported_by_its_anchor() {
            let handle = AnchorHandle::new();
            let observed = Rc::new(Cell::new((f32::NAN, f32::NAN)));
            let panel = show_panel(
                handle.clone(),
                FloatingSide::Bottom,
                FloatingAlign::Start,
                observed.clone(),
            );

            let mut app = aimer::AimerApp::start_headless(anchored_page(handle.clone()));
            app.render_frame();

            assert_eq!(
                handle.bounds().map(|bounds| (bounds.x, bounds.y)),
                Some((20.0, 20.0)),
                "the anchor must report where its child was painted"
            );

            let (x, y) = observed.get();
            assert!(
                (x - 20.0).abs() < 1e-3 && (y - 64.0).abs() < 1e-3,
                "expected the panel at (20, 64), got ({x}, {y})"
            );

            panel.dismiss();
        }

        #[test]
        fn a_panel_aligned_to_the_trailing_edge_ends_where_the_anchor_ends() {
            let handle = AnchorHandle::new();
            let observed = Rc::new(Cell::new((f32::NAN, f32::NAN)));
            let panel = show_panel(
                handle.clone(),
                FloatingSide::Bottom,
                FloatingAlign::End,
                observed.clone(),
            );

            let mut app = aimer::AimerApp::start_headless(anchored_page(handle));
            app.render_frame();

            let (x, y) = observed.get();
            assert!(
                (x - 40.0).abs() < 1e-3 && (y - 64.0).abs() < 1e-3,
                "expected the panel at (40, 64), got ({x}, {y})"
            );

            panel.dismiss();
        }
    }

    mod font_registry {
        use aimer::style::TextStyle;
        use aimer::{Color, RichText, SpanStyle, TextSpan};
        use aimer_assets::{
            FontError, FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight,
        };

        // Test-only fixture: production builds do not pull the optional
        // bundled-fonts feature merely to exercise registration validation.
        const TEST_FONT: &[u8] = include_bytes!("../graphic/cupid_font/fonts/JetBrainsMono-Regular.ttf");

        #[test]
        fn named_font_registration_is_validated_and_stable() {
            assert_ne!(FontFamily::SANS_SERIF, FontFamily::MONOSPACE);

            let family = FontRegistry::register(FontRegistration {
                family: "Aimer Registry Test Mono",
                bytes: TEST_FONT,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            })
            .expect("valid font bytes should register");
            let bold_family = FontRegistry::register(FontRegistration {
                family: " aimer registry test mono ",
                bytes: TEST_FONT,
                weight: FontWeight::Bold,
                style: FontStyle::Normal,
            })
            .expect("a second variant should register under the same normalized family");

            assert_eq!(family, bold_family);

            let duplicate = FontRegistry::register(FontRegistration {
                family: "AIMER REGISTRY TEST MONO",
                bytes: TEST_FONT,
                weight: FontWeight::Value(400),
                style: FontStyle::Normal,
            });
            assert!(matches!(duplicate, Err(FontError::DuplicateVariant { .. })));

            let invalid = FontRegistry::register(FontRegistration {
                family: "Invalid Font Test",
                bytes: b"not a font",
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            });
            assert_eq!(invalid, Err(FontError::InvalidFont));

            let empty_name = FontRegistry::register(FontRegistration {
                family: "  ",
                bytes: TEST_FONT,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            });
            assert_eq!(empty_name, Err(FontError::EmptyFamily));
        }

        #[test]
        fn public_monospace_and_highlighted_rich_text_contracts_compose() {
            let family = FontRegistry::register(FontRegistration {
                family: "Aimer Public Text Test Mono",
                bytes: TEST_FONT,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            })
            .unwrap();
            let base = TextStyle::new().font_family(family);
            let span = TextSpan::root([
                TextSpan::new("let "),
                TextSpan::new("answer")
                    .style(SpanStyle::new().background_color(Color::Rgba(255, 240, 120, 255))),
            ]);
            let flattened = span.flatten(&base);

            assert!(
                flattened
                    .iter()
                    .all(|span| span.style.font_family == family)
            );
            assert_eq!(flattened[0].style.background_color, None);
            assert_eq!(
                flattened[1].style.background_color,
                Some(Color::Rgba(255, 240, 120, 255))
            );

            let _widget = RichText::new(span).text_style(base).wrapped();
        }
    }

    mod grid_headless {
        use std::cell::Cell;
        use std::rc::Rc;

        use aimer::{
            AimerApp, AnyElement, BuildContext, Drawable, Element, EventElement, LayoutElement,
            Rebuildable, ResolvedSize, VisitorElement, Widget,
        };
use aimer_grid::{Grid, GridItem, GridTrack};

        #[derive(Clone)]
        struct SizeProbe {
            observed: Rc<Cell<ResolvedSize>>,
        }

        struct SizeProbeElement {
            observed: Rc<Cell<ResolvedSize>>,
        }

        impl Widget for SizeProbe {
            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                SizeProbeElement {
                    observed: self.observed.clone(),
                }
                .boxed()
            }
        }

        impl aimer::PortableWidget for SizeProbe {}

        impl Drawable for SizeProbeElement {
            fn update(&self, ctx: &BuildContext) {
                self.observed.set(ctx.parent_size);
            }
        }

        impl EventElement for SizeProbeElement {}
        impl LayoutElement for SizeProbeElement {}
        impl Rebuildable for SizeProbeElement {}

        impl VisitorElement for SizeProbeElement {
            fn debug_name(&self) -> &'static str {
                "SizeProbe"
            }
        }

        #[test]
        fn grid_assigns_cell_constraints_during_a_headless_frame() {
            let first = Rc::new(Cell::new(ResolvedSize::default()));
            let second = Rc::new(Cell::new(ResolvedSize::default()));
            let grid = Grid::new()
                .columns([GridTrack::Px(100.0), GridTrack::Px(200.0)])
                .rows([GridTrack::Px(50.0)])
                .children([
                    GridItem::new(SizeProbe {
                        observed: first.clone(),
                    }),
                    GridItem::new(SizeProbe {
                        observed: second.clone(),
                    }),
                ]);

            let mut app = AimerApp::start_headless(grid);
            app.render_frame();

            assert_eq!(
                first.get(),
                ResolvedSize {
                    width: 100.0,
                    height: 50.0
                }
            );
            assert_eq!(
                second.get(),
                ResolvedSize {
                    width: 200.0,
                    height: 50.0
                }
            );
        }

        #[test]
        fn invalid_grid_configuration_renders_in_a_headless_frame() {
            let observed = Rc::new(Cell::new(ResolvedSize::default()));
            let grid = Grid::new().children([GridItem::new(SizeProbe { observed })]);

            let mut app = AimerApp::start_headless(grid);
            app.render_frame();
        }
    }

    #[cfg(feature = "markdown")]
    mod markdown_custom_widget_headless {
        //! Custom widgets rendered by Markdown must remain interactive.

        use std::cell::Cell;
        use std::rc::Rc;
        use std::time::Duration;

        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
        use aimer::{
            AimerApp, MarkdownInlineRule, MarkdownInlineSyntax, MarkdownViewer, SizedBox, Widget,
        };

        fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
        }

        fn click<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
            move_to(app, x, y);
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.render_frame();
            std::thread::sleep(Duration::from_millis(400));
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Left,
            });
        }
        //
        // #[test]
        // fn custom_inline_button_receives_events_inside_markdown_viewer() {
        //
        //     let presses = Rc::new(Cell::new(0));
        //     let callback_presses = presses.clone();
        //     let viewer = MarkdownViewer::new()
        //         .markdown("{{button:Press}}")
        //         .custom_inline(
        //             MarkdownInlineRule::new(
        //                 "button",
        //                 MarkdownInlineSyntax::Paired {
        //                     opening: "{{button:",
        //                     closing: "}}",
        //                 },
        //             ),
        //             move |_| {
        //                 let presses = callback_presses.clone();
        //                 Button::new()
        //                     .on_press(move || presses.set(presses.get() + 1))
        //                     .child(SizedBox::new().width(120).height(40))
        //                     .boxed()
        //             },
        //         );
        //     let mut app = AimerApp::start_headless(viewer);
        //     app.render_frame();
        //
        //     click(&mut app, 20.0, 20.0);
        //
        //     assert_eq!(presses.get(), 1);
        // }
    }

    mod resize_rebuild_headless {
        //! What a window resize is allowed to rebuild.
        //!
        //! A drag delivers a resize event per pixel of travel, so the frame answering it
        //! decides whether the window follows the cursor. Laying the tree out again is
        //! unavoidable — every constraint below the root changed — but re-running the
        //! `build` of a widget that never asked about the window produces the identical
        //! subtree at the cost of the whole application.

        use std::cell::Cell;
        use std::rc::Rc;

        use aimer::screen::MediaQuery;
        use aimer::quiver::winit::dpi::PhysicalSize;
        use aimer::quiver::winit::event::WindowEvent;
        use aimer::{
            AimerApp, AnyElement, BuildContext, Column, Element, SizedBox, StatelessElement, Widget,
        };

        /// A widget that records every build and reports whether the window is narrow.
        ///
        /// Written out rather than declared with `#[widget]` so the test owns the build
        /// closure and can count its invocations.
        #[derive(Clone)]
        struct Probe {
            builds: Rc<Cell<u32>>,
            compact: Option<Rc<Cell<bool>>>,
            /// Whether the breakpoint is read as a question about the window rather
            /// than by reading the window and answering it afterwards.
            selected: bool,
        }

        impl Probe {
            /// A widget that reads the window itself.
            fn watching(builds: &Rc<Cell<u32>>, compact: &Rc<Cell<bool>>) -> Self {
                Self {
                    builds: builds.clone(),
                    compact: Some(compact.clone()),
                    selected: false,
                }
            }

            /// A widget that reads only the breakpoint.
            fn selecting(builds: &Rc<Cell<u32>>, compact: &Rc<Cell<bool>>) -> Self {
                Self {
                    selected: true,
                    ..Self::watching(builds, compact)
                }
            }

            /// A widget that never looks at the window.
            fn indifferent(builds: &Rc<Cell<u32>>) -> Self {
                Self {
                    builds: builds.clone(),
                    compact: None,
                    selected: false,
                }
            }
        }

        impl Widget for Probe {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                let source = self.clone();
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        source.builds.set(source.builds.get() + 1);
                        if let Some(compact) = &source.compact {
                            compact.set(if source.selected {
                                MediaQuery::select(ctx, |media| media.size.width < 600.0)
                            } else {
                                MediaQuery::of(ctx).size.width < 600.0
                            });
                        }
                        SizedBox::new().width(10).height(10).to_element(ctx)
                    },
                    None,
                    "Probe",
                )
                .boxed()
            }
        }

        impl aimer::PortableWidget for Probe {}

        #[test]
        fn a_resize_rebuilds_only_the_widgets_that_read_the_window() {
            let watching_builds = Rc::new(Cell::new(0));
            let indifferent_builds = Rc::new(Cell::new(0));
            let compact = Rc::new(Cell::new(false));

            let page = Column::new().children([
                Probe::watching(&watching_builds, &compact).boxed(),
                Probe::indifferent(&indifferent_builds).boxed(),
            ]);

            let mut app = AimerApp::start_headless(page);
            app.render_frame();
            assert_eq!(watching_builds.get(), 1);
            assert_eq!(indifferent_builds.get(), 1);
            assert!(
                !compact.get(),
                "the window starts wider than the breakpoint"
            );

            app.send_window_event(WindowEvent::Resized(PhysicalSize::new(390, 844)));
            app.render_frame();

            assert!(
                compact.get(),
                "the window reader kept the answer it gave for the old window"
            );
            assert!(
                watching_builds.get() > 1,
                "the window reader was never rebuilt"
            );
            assert_eq!(
                indifferent_builds.get(),
                1,
                "a widget that never read the window was rebuilt {} times by a resize",
                indifferent_builds.get()
            );
        }

        /// A widget that asks for the breakpoint rather than for the window sits out
        /// every width where its layout cannot differ.
        ///
        /// This is what a drag is: hundreds of resizes, none of which crosses the one
        /// width the widget cares about, and one that does.
        #[test]
        fn a_breakpoint_reader_is_rebuilt_only_when_the_breakpoint_is_crossed() {
            let builds = Rc::new(Cell::new(0));
            let compact = Rc::new(Cell::new(false));

            let mut app = AimerApp::start_headless(Probe::selecting(&builds, &compact));
            app.render_frame();
            let settled = builds.get();

            for width in 700..800 {
                app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, 800)));
                app.render_frame();
            }

            assert_eq!(
                builds.get(),
                settled,
                "a drag that never crossed the breakpoint rebuilt the widget {} times",
                builds.get() - settled
            );

            app.send_window_event(WindowEvent::Resized(PhysicalSize::new(390, 844)));
            app.render_frame();

            assert!(compact.get(), "the breakpoint was crossed unnoticed");
        }

        /// The reader has to keep answering for every later resize, not just the first:
        /// its registration is renewed by the rebuild the previous resize caused.
        #[test]
        fn a_window_reader_keeps_following_the_window_across_repeated_resizes() {
            let builds = Rc::new(Cell::new(0));
            let compact = Rc::new(Cell::new(false));

            let mut app = AimerApp::start_headless(Probe::watching(&builds, &compact));
            app.render_frame();

            for (size, expected) in [
                (PhysicalSize::new(390, 844), true),
                (PhysicalSize::new(1200, 800), false),
                (PhysicalSize::new(420, 900), true),
            ] {
                app.send_window_event(WindowEvent::Resized(size));
                app.render_frame();
                assert_eq!(
                    compact.get(),
                    expected,
                    "the reader stopped following the window at {size:?}"
                );
            }
        }
    }

    mod rich_text_cursor_headless {
        //! The hover cursor of text, driven through a headless application.
        //!
        //! These exercise the real pipeline: a window event enters the app, the element
        //! tree resolves the hover, and the window records the requested shape.

        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, WindowEvent};
        use aimer::quiver::winit::window::CursorIcon;
        use aimer::{AimerApp, RichText, SelectionArea, Text, TextSpan};

        fn hover(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<impl aimer::Widget + 'static>,
            x: f64,
            y: f64,
        ) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
            app.render_frame();
        }

        #[test]
        fn hovering_a_rich_text_link_uses_the_pointer_cursor() {
            let text =
                RichText::new(TextSpan::new("Aimer").link("https://aimer.dev")).on_link(|_| {});
            let mut app = AimerApp::start_headless(text);
            app.render_frame();

            hover(&mut app, 1.0, 1.0);

            assert_eq!(app.cursor_icon(), CursorIcon::Pointer);
        }

        #[test]
        fn hovering_selectable_text_inside_a_region_uses_the_text_cursor() {
            let mut app =
                AimerApp::start_headless(SelectionArea::new().child(Text::new("Selectable")));
            app.render_frame();

            hover(&mut app, 1.0, 1.0);

            assert_eq!(app.cursor_icon(), CursorIcon::Text);
        }
    }

    mod selection_in_scrollable_headless {
        //! Selecting text that lives inside a scroll view, driven through a headless
        //! application.
        //!
        //! A press inside a `Scrollable` is ambiguous — it could be a tap, or the start
        //! of a scroll — so the view arms a pending drag and takes the gesture from
        //! whoever was under the pointer as soon as it travels past the drag threshold,
        //! cancelling that element's capture. For a button that is exactly right. For a
        //! `SelectionArea`, whose selection *is* a drag, it made text inside a scroll
        //! view impossible to select: the highlight died on the first few pixels and the
        //! page scrolled instead.
        //!
        //! A text that has begun selecting therefore claims the pointer, and the scroll
        //! view leaves a claimed pointer alone. These tests drive the real pipeline —
        //! window events into the app, through the element tree — and watch the scroll
        //! offset the `ScrollController` reports.

        use std::thread::sleep;
        use std::time::Duration;

        use aimer::events::pointer::PointerSource;
        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{
            DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
        };
        use aimer::{
            AnyWidget, BoxAlignment, Column, Container, PointerKey, ScrollAxis, ScrollController,
            Scrollable, SelectionArea, SizedBox, Text, Widget, is_pointer_claimed,
            release_all_pointers,
        };
        use aimer_quiver::AimerApp;

        /// Enough lines to make the content far taller than the 800px headless
        /// viewport, so there is somewhere to scroll to.
        const LINE_COUNT: usize = 80;

        /// Where the gesture starts: comfortably inside the content, far enough down
        /// that dragging upwards has room to scroll.
        const PRESS_Y: f64 = 400.0;

        /// Where it ends: upwards by much more than the drag threshold, which is the
        /// direction that scrolls a vertical view away from its top.
        const RELEASE_Y: f64 = 120.0;

        /// How often a touch screen reports the finger, roughly: one sample per frame.
        const SCROLL_STEP_INTERVAL: Duration = Duration::from_millis(12);

        /// Enough samples that the gesture as a whole outlasts the hold a touch
        /// selection waits for, each one as short as a real finger's.
        const SCROLL_STEP_COUNT: usize = 60;

        type HeadlessApp<W> = aimer::quiver::aimer_app::HeadlessAimerApp<W>;

        /// A tall column of selectable lines inside a vertical scroll view.
        fn selectable_page(controller: &ScrollController) -> impl Widget + 'static {
            let lines = (0..LINE_COUNT)
                .map(|index| Text::new(format!("Line {index} of selectable prose")).boxed())
                .collect::<Vec<AnyWidget>>();

            Container::new().box_child(
                Scrollable::new()
                    .controller(controller.clone())
                    .axis(ScrollAxis::Vertical)
                    .child(
                        SelectionArea::new().child(
                            Column::new()
                                .horizontal_alignment(BoxAlignment::Start)
                                .children(lines),
                        ),
                    ),
            )
        }

        /// The same page with nothing selectable in it: the control for every
        /// assertion below.
        fn plain_page(controller: &ScrollController) -> impl Widget + 'static {
            Container::new().box_child(
                Scrollable::new()
                    .controller(controller.clone())
                    .axis(ScrollAxis::Vertical)
                    .child(
                        Container::new()
                            .child(SizedBox::new().height(LINE_COUNT as u32 * 40).width(400)),
                    ),
            )
        }

        fn move_to<W: Widget + 'static>(app: &mut HeadlessApp<W>, x: f64, y: f64) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
            app.render_frame();
        }

        fn press<W: Widget + 'static>(app: &mut HeadlessApp<W>, x: f64, y: f64) {
            move_to(app, x, y);
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.render_frame();
        }

        fn release<W: Widget + 'static>(app: &mut HeadlessApp<W>) {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Left,
            });
            app.render_frame();
        }

        /// Drags from `(x, from)` to `(x, to)` in steps large enough to pass the drag
        /// threshold, the way a real pointer arrives.
        fn drag_up<W: Widget + 'static>(app: &mut HeadlessApp<W>, x: f64, from: f64, to: f64) {
            press(app, x, from);
            let steps = 8;
            let step = (to - from) / steps as f64;
            for index in 1..=steps {
                move_to(app, x, from + step * index as f64);
            }
            release(app);
        }

        /// Drags a finger from `(x, from)` to `(x, to)`, in the phases a touch screen
        /// reports.
        fn touch_drag_up<W: Widget + 'static>(
            app: &mut HeadlessApp<W>,
            x: f64,
            from: f64,
            to: f64,
        ) {
            let contact = |phase, y| {
                WindowEvent::Touch(Touch {
                    device_id: DeviceId::dummy(),
                    phase,
                    location: PhysicalPosition::new(x, y),
                    force: None,
                    id: 0,
                })
            };
            app.send_window_event(contact(TouchPhase::Started, from));
            app.render_frame();
            let steps = 8;
            let step = (to - from) / steps as f64;
            for index in 1..=steps {
                app.send_window_event(contact(TouchPhase::Moved, from + step * index as f64));
                app.render_frame();
            }
            app.send_window_event(contact(TouchPhase::Ended, to));
            app.render_frame();
        }

        /// Drags a finger the way a thumb actually moves a page: far enough to hand the
        /// gesture to the scroll view, and for longer than the hold a touch selection
        /// waits for, so a press the view forgot to revoke has time to ripen.
        fn touch_scroll_slowly<W: Widget + 'static>(
            app: &mut HeadlessApp<W>,
            x: f64,
            from: f64,
            to: f64,
        ) {
            let contact = |phase, y| {
                WindowEvent::Touch(Touch {
                    device_id: DeviceId::dummy(),
                    phase,
                    location: PhysicalPosition::new(x, y),
                    force: None,
                    id: 0,
                })
            };
            app.send_window_event(contact(TouchPhase::Started, from));
            app.render_frame();
            let step = (to - from) / SCROLL_STEP_COUNT as f64;
            for index in 1..=SCROLL_STEP_COUNT {
                sleep(SCROLL_STEP_INTERVAL);
                app.send_window_event(contact(TouchPhase::Moved, from + step * index as f64));
                app.render_frame();
            }
        }

        /// The finger every touch platform reports as pointer zero.
        const fn finger() -> PointerKey {
            PointerKey::new(PointerSource::Touch, 0)
        }

        /// Puts a finger on the glass at `(x, y)` and leaves it there.
        fn touch_down<W: Widget + 'static>(app: &mut HeadlessApp<W>, x: f64, y: f64) {
            app.send_window_event(WindowEvent::Touch(Touch {
                device_id: DeviceId::dummy(),
                phase: TouchPhase::Started,
                location: PhysicalPosition::new(x, y),
                force: None,
                id: 0,
            }));
            app.render_frame();
        }

        /// Scrolls the page the way a platform that reads the finger itself does: as
        /// scroll deltas with a contact still on the glass, and not a single pointer
        /// move. This is how a touch browser reports a finger dragging a page, and how
        /// momentum arrives everywhere.
        fn platform_scroll<W: Widget + 'static>(app: &mut HeadlessApp<W>, dy: f64) {
            app.send_window_event(WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, dy)),
                phase: TouchPhase::Moved,
            });
            app.render_frame();
        }

        /// A page that scrolls itself under a resting finger must not select a word.
        ///
        /// The scroll view revokes a pending press when it wins the finger's *drag* —
        /// but a page moves without any drag to win. A touch browser reports a finger
        /// scrolling the page as scroll deltas, momentum carries a page on after the
        /// finger is gone, and an animation moves a paragraph for reasons of its own. In
        /// each case the text hears nothing at all, so the press must judge itself: the
        /// glyph it was resting on has slid away, and a finger cannot hold still on
        /// content that is moving.
        #[test]
        fn a_page_that_scrolls_itself_under_a_finger_never_starts_selecting() {
            release_all_pointers();
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(selectable_page(&controller));
            app.render_frame();
            app.render_frame();
            assert!(controller.max_extent().y > 0.0);

            touch_down(&mut app, 40.0, PRESS_Y);
            for _ in 0..SCROLL_STEP_COUNT {
                sleep(SCROLL_STEP_INTERVAL);
                platform_scroll(&mut app, -20.0);
            }

            assert!(
                controller.offset().y > 0.0,
                "the platform scrolled the page while the finger was down"
            );
            assert!(
                !is_pointer_claimed(finger()),
                "the content moved under the finger, so the press was never a hold"
            );
        }

        /// A thumb that keeps scrolling must never start selecting.
        ///
        /// The press a finger leaves behind ripens into a selection once the hold has
        /// elapsed, and a frame promotes it without consulting how far the finger has
        /// travelled since — it cannot, because a finger whose gesture the scroll view
        /// took no longer reports its moves to the text. The view must therefore revoke
        /// that press when it takes the gesture, and this is the test that it does:
        /// eight moves spread over more than the hold, with the finger still down.
        #[test]
        fn a_thumb_that_keeps_scrolling_never_starts_selecting() {
            release_all_pointers();
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(selectable_page(&controller));
            app.render_frame();
            app.render_frame();
            assert!(controller.max_extent().y > 0.0);

            touch_scroll_slowly(&mut app, 40.0, PRESS_Y, RELEASE_Y);

            assert!(
                controller.offset().y > 0.0,
                "the finger travelled far enough to be a scroll"
            );
        }

        /// A finger dragged over selectable text scrolls the page.
        ///
        /// A finger press means too many things to act on, so the text records it and
        /// takes the *pointer* — the only way an enclosing view can tell it the gesture
        /// is gone — while claiming nothing, which leaves that view free to take the
        /// drag. Both halves matter: claiming here would make a page refuse to scroll
        /// wherever there is text on it, and taking nothing would leave the recorded
        /// press to ripen into a selection several frames after the finger has left the
        /// glass.
        #[test]
        fn a_finger_dragged_over_selectable_text_scrolls_the_page() {
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(selectable_page(&controller));
            app.render_frame();
            app.render_frame();
            assert!(controller.max_extent().y > 0.0);

            touch_drag_up(&mut app, 40.0, PRESS_Y, RELEASE_Y);

            assert!(
                controller.offset().y > 0.0,
                "a finger that travels over text is scrolling, not selecting"
            );
        }

        #[test]
        fn dragging_across_text_inside_a_scroll_view_does_not_scroll_it() {
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(selectable_page(&controller));
            app.render_frame();
            app.render_frame();
            assert!(
                controller.max_extent().y > 0.0,
                "the page must be scrollable for this test to mean anything"
            );

            drag_up(&mut app, 40.0, PRESS_Y, RELEASE_Y);

            assert_eq!(
                controller.offset().y,
                0.0,
                "the drag selected text, so the scroll view must not have moved"
            );
        }

        /// A finger can scroll plain content, while mouse drags stay available
        /// for selection and other desktop gestures.
        #[test]
        fn a_finger_drag_over_plain_content_still_scrolls() {
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(plain_page(&controller));
            app.render_frame();
            app.render_frame();
            assert!(controller.max_extent().y > 0.0);

            touch_drag_up(&mut app, 40.0, PRESS_Y, RELEASE_Y);

            assert!(
                controller.offset().y > 0.0,
                "a finger drag over nothing selectable is a scroll"
            );
        }

        /// A gesture that is over releases its claim, so the next finger drag scrolls even
        /// though the previous one selected. Without this the first selection would
        /// deadlock the view forever.
        #[test]
        fn a_scroll_view_scrolls_again_after_a_selection_gesture_ends() {
            let controller = ScrollController::new();
            let mut app = AimerApp::start_headless(selectable_page(&controller));
            app.render_frame();
            app.render_frame();

            drag_up(&mut app, 40.0, PRESS_Y, RELEASE_Y);
            assert_eq!(controller.offset().y, 0.0);
            assert!(
                !is_pointer_claimed(PointerKey::new(PointerSource::Mouse, 0)),
                "ending selection releases the mouse claim"
            );

            // The second gesture starts on the background to the right of the text,
            // where nothing selectable lives.
            touch_drag_up(&mut app, 900.0, PRESS_Y, RELEASE_Y);

            assert!(
                controller.offset().y > 0.0,
                "the selection released the pointer, so this drag is a scroll"
            );
        }
    }

    mod sidebar_resizable_headless {
        //! A `Resizable` wrapping an interactive child — the shape of RustForum's
        //! side bar — must not swallow the child's events.
        //!
        //! The side bar wraps everything in an `ImplicitAnimatedBuilder`, whose target
        //! flips whenever the pointer crosses the resize handle. The animation element
        //! rebuilds its subtree on every frame it draws, and each rebuild replaces the
        //! subtree's element identities. The dispatcher routes captured pointers and
        //! focus by identity, so a press that spans one of those rebuilds dies: the
        //! release is addressed to an element that no longer exists and is dropped.
        //!
        //! The interesting claims are about *routing*, so everything here is driven by
        //! window events — the same ones a mouse produces — through the real headless
        //! event loop.

        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use std::time::Duration;

        use aimer::animation::{ImplicitAnimatedBuilder, curves};
        use aimer::input::button::Button;
        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
        use aimer::style::LayoutSpacing;
        use aimer::{
            AimerApp, AnyElement, BuildContext, Container, Direction, Resizable, ResolvedSize,
            SizedBox, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
        };

        /// The side bar itself: an animated border that grows a stroke while the
        /// resize handle is hovered, wrapped around a `Resizable` holding a button —
        /// the same shape `AppSideBar` builds, with a long animation so every event
        /// below lands while it is still running.
        struct SideBarLike {
            presses: Rc<Cell<u32>>,
            resizes: Rc<RefCell<Vec<ResolvedSize>>>,
        }

        struct SideBarLikeState {
            resizable: bool,
            presses: Rc<Cell<u32>>,
            resizes: Rc<RefCell<Vec<ResolvedSize>>>,
            updater: StateUpdater<Self>,
        }

        impl Widget for SideBarLike {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                let key = Widget::key(&self);
                StatefulElement::new_with_name(self, ctx, "SideBarLike", key)
                    .0
                    .boxed()
            }
        }

        impl aimer::PortableWidget for SideBarLike {}

        impl StatefulWidget for SideBarLike {
            type State = SideBarLikeState;

            fn create_state(self) -> Self::State {
                SideBarLikeState {
                    resizable: false,
                    presses: self.presses,
                    resizes: self.resizes,
                    updater: StateUpdater::new(),
                }
            }
        }

        impl State<SideBarLike> for SideBarLikeState {
            fn init_state(&mut self, updater: StateUpdater<Self>)
            where
                Self: Sized,
            {
                self.updater = updater;
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                let zone_updater = self.updater;
                let presses = self.presses.clone();
                let resizes = self.resizes.clone();

                ImplicitAnimatedBuilder::new(
                    if self.resizable { 1.0 } else { 0.0 },
                    // Long enough that every event below lands mid-animation.
                    Duration::from_secs(30),
                    curves::EASE_IN_OUT,
                    move |_delta| {
                        let zone_updater = zone_updater;
                        let presses = presses.clone();
                        let resizes = resizes.clone();

                        Resizable::new()
                            .width(250.0)
                            .height(500.0)
                            .min_width(200.0)
                            .max_width(500.0)
                            .handle_thickness(10.0)
                            .handle_outset(6.0)
                            .direction(Direction::RIGHT)
                            .on_resize(move |size: ResolvedSize| resizes.borrow_mut().push(size))
                            .on_resize_zone(move |zone| {
                                zone_updater.set_state(move |state| {
                                    state.resizable = zone != Direction::NONE;
                                });
                            })
                            .child(
                                Container::new().padding(LayoutSpacing::all(12.0)).child(
                                    Button::new()
                                        .on_press(move || presses.set(presses.get() + 1))
                                        .child(SizedBox::new().width(100.0).height(40.0)),
                                ),
                            )
                    },
                )
            }
        }

        fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
        }

        fn press<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, state: ElementState) {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            });
        }

        fn click<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
            // The recognizer merges two taps within 300ms into a double tap, so a
            // test click must stay clear of the previous one to count as a press.
            std::thread::sleep(Duration::from_millis(400));
            move_to(app, x, y);
            press(app, ElementState::Pressed);
            // A real click spans several redraws: draw the frame a window would draw
            // between the press and the release.
            app.render_frame();
            press(app, ElementState::Released);
        }

        #[test]
        fn the_sidebar_child_keeps_receiving_events_while_the_resize_zone_animates() {
            let presses = Rc::new(Cell::new(0));
            let resizes = Rc::new(RefCell::new(Vec::new()));

            let mut app = AimerApp::start_headless(SideBarLike {
                presses: presses.clone(),
                resizes: resizes.clone(),
            });
            app.render_frame();

            // The baseline: the button inside the resizable works before any zone
            // change has animated.
            click(&mut app, 50.0, 30.0);
            assert_eq!(presses.get(), 1, "the baseline click never fired");

            // Hover the resize handle: the zone flips, the state rebuilds, and a long
            // animation starts — every pumped frame rebuilds the whole subtree with
            // fresh element identities.
            move_to(&mut app, 245.0, 250.0);
            app.pump_frames(3);

            // A click while the animation is running must still fire.
            click(&mut app, 50.0, 30.0);
            assert_eq!(
                presses.get(),
                2,
                "the click during the zone animation was swallowed"
            );

            // Drag the handle while the animation is running, with a frame between
            // each step the way a real drag spans redraws.
            move_to(&mut app, 245.0, 250.0);
            press(&mut app, ElementState::Pressed);
            app.render_frame();
            move_to(&mut app, 300.0, 250.0);
            app.render_frame();
            move_to(&mut app, 330.0, 250.0);
            app.render_frame();
            press(&mut app, ElementState::Released);

            // The width follows the pointer: 250 + (330 - 245).
            let final_width = resizes.borrow().last().map(|size| size.width);
            assert!(
                final_width.is_some_and(|width| (width - 335.0).abs() < 1.0),
                "the drag did not reach 335: {:?}",
                resizes.borrow()
            );

            // And the child still works after the drag.
            click(&mut app, 50.0, 30.0);
            assert_eq!(presses.get(), 3, "the click after the drag was swallowed");
        }
    }

    mod system_theme_headless {
        //! What the system appearance is allowed to do to a running application.
        //!
        //! The user switches appearance in the system settings while the application is
        //! open, and the platform announces it instead of restarting the app. An
        //! `AnimatedTheme` that follows the system therefore has to cross into the other
        //! theme mid-run, and one that was told to use a specific theme has to stay
        //! exactly where it is — an application that pins its appearance is not asking
        //! for the platform's opinion.
        //!
        //! The application may also change its mind about following at all, and that
        //! change is a theme change like any other: it animates.

        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use std::thread::sleep;
        use std::time::Duration;

        use aimer::modal::ModalHost;
        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::quiver::winit::event::WindowEvent;
        use aimer::quiver::winit::window::Theme as SystemTheme;
        use aimer::style::{AnimatedTheme, Theme, ThemeData, ThemeMode};
        use aimer::{
            AimerApp, AnyElement, BuildContext, Color, Element, SizedBox, State, StateUpdater,
            StatefulElement, StatefulWidget, StatelessElement, Widget,
        };

        /// A widget that records the theme it was built with, and counts its builds.
        #[derive(Clone)]
        struct Probe {
            background: Rc<Cell<Color>>,
            builds: Rc<Cell<u32>>,
        }

        impl Probe {
            fn new() -> Self {
                Self {
                    background: Rc::new(Cell::new(Color::Transparent)),
                    builds: Rc::new(Cell::new(0)),
                }
            }
        }

        impl Widget for Probe {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                let source = self.clone();
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        source.builds.set(source.builds.get() + 1);
                        source.background.set(ThemeData::of(ctx).background_color);
                        SizedBox::new().width(10).height(10).to_element(ctx)
                    },
                    None,
                    "Probe",
                )
                .boxed()
            }
        }

        impl aimer::PortableWidget for Probe {}

        /// Announces the appearance the system switched to, the way the platform does.
        fn switch_system_to<W: Widget + 'static>(
            app: &mut HeadlessAimerApp<W>,
            theme: SystemTheme,
        ) {
            app.send_window_event(WindowEvent::ThemeChanged(theme));
            app.render_frame();
        }

        /// How long a transition in these tests lasts.
        const TRANSITION: Duration = Duration::from_millis(250);

        thread_local! {
            /// The shell's state, so a test can flip the mode the way a button does.
            static MODE: RefCell<Option<StateUpdater<ThemeSwitchState>>> = const { RefCell::new(None) };
        }

        /// An application with its own "follow the system" switch, as `jaime/src/system_theme.rs`
        /// offers it.
        struct ThemeSwitch {
            initial: ThemeMode,
            probe: Probe,
        }

        struct ThemeSwitchState {
            mode: ThemeMode,
            probe: Probe,
        }

        impl StatefulWidget for ThemeSwitch {
            type State = ThemeSwitchState;

            fn create_state(self) -> Self::State {
                ThemeSwitchState {
                    mode: self.initial,
                    probe: self.probe.clone(),
                }
            }
        }

        impl State<ThemeSwitch> for ThemeSwitchState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                MODE.replace(Some(updater));
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                AnimatedTheme::new()
                    .mode(self.mode)
                    .duration(TRANSITION)
                    .child(self.probe.clone())
            }
        }

        impl Widget for ThemeSwitch {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatefulElement::new_with_name(self, ctx, "ThemeSwitch", None)
                    .0
                    .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "ThemeSwitch"
            }
        }

        impl aimer::PortableWidget for ThemeSwitch {}

        /// Flips the application's mode, the way pressing its button does.
        fn set_mode(mode: ThemeMode) {
            MODE.with_borrow(|updater| {
                updater
                    .as_ref()
                    .expect("the shell state should have published its updater")
                    .set_state(move |state| state.mode = mode)
            });
        }

        /// Draws past the end of a transition.
        fn settle<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) {
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }
        }

        /// Starts an application whose mode a test can flip, settled on `initial`.
        fn app_switching_mode_from(
            initial: ThemeMode,
            probe: &Probe,
        ) -> HeadlessAimerApp<ModalHost<ThemeSwitch>> {
            let mut app = AimerApp::start_headless(ThemeSwitch {
                initial,
                probe: probe.clone(),
            });
            settle(&mut app);
            app
        }

        #[test]
        fn a_system_appearance_switch_crosses_the_application_into_the_other_theme() {
            let probe = Probe::new();
            let mut app = AimerApp::start_headless(
                AnimatedTheme::new()
                    // The transition itself is covered by the theme's own tests; this
                    // one is about the appearance arriving at all.
                    .duration(Duration::ZERO)
                    .child(probe.clone()),
            );

            app.render_frame();
            assert_eq!(
                probe.background.get(),
                ThemeData::light().background_color,
                "a fresh application did not start in the appearance the platform reports"
            );

            switch_system_to(&mut app, SystemTheme::Dark);

            assert_eq!(
                probe.background.get(),
                ThemeData::dark().background_color,
                "the application kept the light theme after the system switched to dark"
            );

            switch_system_to(&mut app, SystemTheme::Light);

            assert_eq!(
                probe.background.get(),
                ThemeData::light().background_color,
                "the application stopped following the system after the first switch"
            );
        }

        #[test]
        fn an_application_that_pins_its_theme_ignores_the_system_appearance() {
            let probe = Probe::new();
            let mut app = AimerApp::start_headless(
                AnimatedTheme::new()
                    .mode(ThemeMode::Light)
                    .duration(Duration::ZERO)
                    .child(probe.clone()),
            );

            app.render_frame();
            let settled = probe.builds.get();

            switch_system_to(&mut app, SystemTheme::Dark);

            assert_eq!(
                probe.background.get(),
                ThemeData::light().background_color,
                "a pinned light theme was overruled by the system"
            );
            assert_eq!(
                probe.builds.get(),
                settled,
                "a system switch the application ignores rebuilt it {} times",
                probe.builds.get() - settled
            );
        }

        #[test]
        fn a_single_theme_ignores_the_system_appearance() {
            let probe = Probe::new();
            let mut app = AimerApp::start_headless(
                AnimatedTheme::new()
                    .data(ThemeData::dark())
                    .duration(Duration::ZERO)
                    .child(probe.clone()),
            );

            app.render_frame();

            switch_system_to(&mut app, SystemTheme::Light);

            assert_eq!(
                probe.background.get(),
                ThemeData::dark().background_color,
                "a named theme was replaced by the system appearance"
            );
        }

        #[test]
        fn following_the_system_again_animates_out_of_the_pinned_theme() {
            let probe = Probe::new();
            // Pinned to dark while the system reports light, so restoring the system
            // means crossing the whole way back.
            let mut app = app_switching_mode_from(ThemeMode::Dark, &probe);
            assert_eq!(probe.background.get(), ThemeData::dark().background_color);

            set_mode(ThemeMode::System);
            app.render_frame();

            assert_ne!(
                probe.background.get(),
                ThemeData::light().background_color,
                "following the system again jumped straight into its theme instead of animating"
            );

            settle(&mut app);

            assert_eq!(
                probe.background.get(),
                ThemeData::light().background_color,
                "the transition into the system appearance never arrived"
            );
        }

        #[test]
        fn pinning_a_theme_animates_out_of_the_system_appearance() {
            let probe = Probe::new();
            let mut app = app_switching_mode_from(ThemeMode::System, &probe);
            assert_eq!(probe.background.get(), ThemeData::light().background_color);

            set_mode(ThemeMode::Dark);
            app.render_frame();

            assert_ne!(
                probe.background.get(),
                ThemeData::dark().background_color,
                "pinning a theme jumped straight into it instead of animating"
            );

            settle(&mut app);

            assert_eq!(
                probe.background.get(),
                ThemeData::dark().background_color,
                "the transition into the pinned theme never arrived"
            );
        }
    }

    mod text_editing_delta_headless {
        //! Native keyboard deltas driven through a headless application.
        //!
        //! A `TextEditingDelta` reported by the iOS / Android keyboard shims targets
        //! an editing *session*, not a screen position: on a phone the finger lifts
        //! off the screen before typing begins, and the last touch may have landed
        //! anywhere. The delta must reach the field that owns the session even when
        //! the pointer no longer rests on that field.

        use aimer::input::{TextEditingController, input::TextField};
        use aimer::quiver::aimer_app::AimerNativePlatformEvent;
        use aimer::quiver::winit::dpi::PhysicalPosition;
        use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
        use aimer::style::{LayoutSpacing, Spacing};
        use aimer::{AimerApp, Container, Widget};
        use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};

        /// Moves the pointer to `(x, y)` and taps there.
        fn tap<W: Widget + 'static>(
            app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
            x: f64,
            y: f64,
        ) {
            let device_id = DeviceId::dummy();
            app.send_window_event(WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(x, y),
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button: MouseButton::Left,
            });
            app.render_frame();
        }

        #[test]
        fn a_native_delta_reaches_the_session_field_after_the_pointer_moves_away() {
            let controller = TextEditingController::new();
            let page = Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(100)))
                .child(TextField::new().controller(controller.clone()));
            let mut app = AimerApp::start_headless(page);
            app.render_frame();

            // Focus the field with a tap inside it, then park the pointer on the
            // container padding, the way a finger leaves the screen before typing.
            tap(&mut app, 150.0, 110.0);
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(1.0, 1.0),
            });
            app.render_frame();

            // The session id the focused field drew is not observable from out here;
            // offering the delta under every id a fresh application can have handed
            // out is fine, because only the owning session applies it.
            for session_id in 1..=8 {
                app.send_user_event(AimerNativePlatformEvent::TextEditingDelta(
                    TextEditingDelta {
                        session_id,
                        revision: controller.revision(),
                        replacement: NativeTextRange::new(0, 0),
                        replacement_text: "你好".into(),
                        selection: NativeTextRange::new(2, 2),
                        composing: None,
                    },
                ));
            }

            assert_eq!(controller.value().text(), "你好");
        }
    }

    mod theme_derive {
        use aimer::animation::Animatable;
        use aimer::style::{AnimatedTheme, Theme};
        use aimer::{BuildContext, Text, Widget};

        #[derive(Clone, Copy, Debug, PartialEq, Theme)]
        struct AppTheme {
            opacity: f32,
            inset: i32,
        }

        #[derive(Clone, Debug, PartialEq, Theme)]
        struct GenericTheme<T>
        where
            T: Send,
        {
            value: T,
        }

        fn assert_theme<T: Theme>() {}

        #[test]
        fn named_theme_interpolates_fields_and_preserves_exact_endpoints() {
            assert_theme::<AppTheme>();
            let begin = AppTheme {
                opacity: 0.0,
                inset: 2,
            };
            let end = AppTheme {
                opacity: 1.0,
                inset: 10,
            };

            assert_eq!(begin.lerp(&end, -1.0), begin);
            assert_eq!(
                begin.lerp(&end, 0.5),
                AppTheme {
                    opacity: 0.5,
                    inset: 6
                }
            );
            assert_eq!(begin.lerp(&end, 2.0), end);
        }

        #[test]
        fn generic_theme_preserves_declared_bounds() {
            assert_theme::<GenericTheme<f32>>();
            let begin = GenericTheme { value: 2.0_f32 };
            let end = GenericTheme { value: 6.0_f32 };

            assert_eq!(begin.lerp(&end, 0.5), GenericTheme { value: 4.0 });
        }

        #[test]
        fn derived_theme_exposes_snapshot_and_copy_lookup_signatures() {
            let _: fn(&BuildContext) -> aimer::provider::Snapshot<AppTheme> = AppTheme::of;
            let _: fn(&BuildContext) -> aimer::provider::Snapshot<AppTheme> = AppTheme::read;
            let _: fn(&BuildContext) -> AppTheme = AppTheme::copied;
        }

        #[test]
        fn animated_theme_builder_accepts_a_derived_custom_theme() {
            fn assert_widget<T: Widget>(_widget: &T) {}

            let widget = AnimatedTheme::new()
                .data(AppTheme {
                    opacity: 0.5,
                    inset: 8,
                })
                .child(Text::new("child"));

            assert_widget(&widget);
        }
    }

    mod theme_toggle_async_builder {
        //! What a rebuild above a route transition may and may not do to the page below.
        //!
        //! `website/src/router.rs` wraps every page in one `AnimatedSwitcher` that keeps
        //! the same identity (`ROUTE_SWITCHER_KEY`) across routes, so the switcher's
        //! state — and its cross-fade — survives navigation. The shell above it
        //! (`website/src/components/app_shell.rs`) animates the theme, and
        //! `website/src/screen/blog.rs` reads that theme with `ThemeData::of` and puts an
        //! `AsyncBuilder` below it. Every tick of the transition therefore rebuilds the
        //! shell, the `Outlet`, the switcher and the page.
        //!
        //! Two things must hold at once:
        //!
        //! * a theme change keeps the request the page already completed — it belongs to
        //!   the element, not to the frame that rebuilt it;
        //! * a navigation still switches the page — the outgoing page's state must not be
        //!   handed to the page replacing it.

        use std::cell::{Cell, RefCell};
        use std::thread::sleep;
        use std::time::Duration;

        use aimer::animation::{AnimatedSwitcher, curves};
        use aimer::base::{ResolvedSize, Size, Vec2d};
        use aimer::modal::ModalHost;
        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::router::{
            Navigator, NavigatorController, NavigatorInstance, Outlet, Route, Router, Shell,
        };
        use aimer::style::{AnimatedTheme, Theme, ThemeData};
        use aimer::{
            AimerApp, AnyElement, AnyWidget, AsyncBuilder, AsyncSnapshot, BuildContext, Column,
            Container, Drawable, Element, EventElement, Expanded, LayoutElement, Rebuildable,
            ScrollAxis, Scrollable, SizedBox, State, StateUpdater, StatefulElement, StatefulWidget,
            StatelessElement, VisitorElement, Widget,
        };

        /// Height of the loaded content: taller than the headless viewport, so a frame
        /// that paints the waiting state is impossible to mistake for a loaded one.
        const CONTENT_HEIGHT: u32 = 4_000;

        /// The identity every route's transition shares, as in `website/src/router.rs`.
        const ROUTE_SWITCHER_KEY: &str = "route-switcher";

        thread_local! {
            /// Labels of what the pages actually painted, in order.
            static PAINTED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
            /// How many times the blog page's request was started.
            static LAUNCHES: Cell<usize> = const { Cell::new(0) };
            /// The shell's state, so a test can toggle the theme the way the header does.
            static THEME: RefCell<Option<StateUpdater<AppShellState>>> = const { RefCell::new(None) };
            /// The navigator the blog page looked up, so a test can navigate from it.
            static NAVIGATOR: RefCell<Option<NavigatorInstance<TestRoute>>> = const { RefCell::new(None) };
        }

        fn painted() -> Vec<&'static str> {
            PAINTED.with_borrow(|painted| painted.clone())
        }

        fn clear_painted() {
            PAINTED.with_borrow_mut(|painted| painted.clear());
        }

        // ---------------------------------------------------------------------------
        // The routes: `website/src/router.rs`.
        // ---------------------------------------------------------------------------

        #[derive(Clone, Debug, PartialEq)]
        enum TestRoute {
            Blog,
            Learn,
        }

        impl Route for TestRoute {
            fn parse(path: &str) -> Option<Self> {
                match path {
                    "/blog" => Some(Self::Blog),
                    "/learn" => Some(Self::Learn),
                    _ => None,
                }
            }

            fn format(&self) -> String {
                match self {
                    Self::Blog => "/blog".to_owned(),
                    Self::Learn => "/learn".to_owned(),
                }
            }
        }

        fn transitioned_page(key: &'static str, child: AnyWidget) -> AnimatedSwitcher<AnyWidget> {
            AnimatedSwitcher::new(Duration::from_millis(200), curves::FAST_OUT_SLOW_IN, child)
                .child_key(key)
                .key(ROUTE_SWITCHER_KEY)
        }

        impl Router for TestRoute {
            fn build(&self, _ctx: &BuildContext) -> AnyWidget {
                match self {
                    Self::Blog => Shell::new(AppShell, |_| {
                        transitioned_page("blog", BlogListPage.boxed()).boxed()
                    })
                    .boxed(),
                    Self::Learn => Shell::new(AppShell, |_| {
                        transitioned_page("learn", LearnPage.boxed()).boxed()
                    })
                    .boxed(),
                }
            }
        }

        impl Widget for TestRoute {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                Router::build(&self, ctx).to_element(ctx)
            }
        }

        impl aimer::PortableWidget for TestRoute {}

        // ---------------------------------------------------------------------------
        // The shell: `website/src/components/app_shell.rs`.
        // ---------------------------------------------------------------------------

        struct AppShell;

        struct AppShellState {
            dark: bool,
        }

        impl StatefulWidget for AppShell {
            type State = AppShellState;

            fn create_state(self) -> Self::State {
                AppShellState { dark: false }
            }
        }

        impl State<AppShell> for AppShellState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                THEME.replace(Some(updater));
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                AnimatedTheme::new()
                    .data(if self.dark {
                        ThemeData::dark()
                    } else {
                        ThemeData::light()
                    })
                    .duration(Duration::from_millis(250))
                    .curve(curves::EASE_IN_OUT)
                    .child(ThemedFrame)
            }
        }

        impl Widget for AppShell {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatefulElement::new_with_name(self, ctx, "AppShell", None)
                    .0
                    .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "AppShell"
            }
        }

        impl aimer::PortableWidget for AppShell {}

        /// The themed frame around the `Outlet`: it reads the theme, so every tick of
        /// the transition rebuilds it and the route below.
        struct ThemedFrame;

        impl Widget for ThemedFrame {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        let theme = ThemeData::of(ctx);
                        Container::new()
                            .color(theme.background_color)
                            .child(
                                Column::new().children([
                                    SizedBox::new().height(40).boxed(),
                                    Expanded::new()
                                        .child(
                                            Container::new()
                                                .color(theme.background_color)
                                                .child(Outlet),
                                        )
                                        .boxed(),
                                ]),
                            )
                            .to_element(ctx)
                    },
                    None,
                    "ThemedFrame",
                )
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "ThemedFrame"
            }
        }

        impl aimer::PortableWidget for ThemedFrame {}

        // ---------------------------------------------------------------------------
        // The pages: `website/src/screen/*`.
        // ---------------------------------------------------------------------------

        /// Reads the theme, looks up the navigator and hosts the request — the shape of
        /// `website/src/screen/blog.rs`.
        struct BlogListPage;

        impl Widget for BlogListPage {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        let theme = ThemeData::of(ctx);
                        NAVIGATOR.replace(Some(NavigatorController::<TestRoute>::of(ctx)));

                        let content = AsyncBuilder::new()
                            .future(|| {
                                LAUNCHES.set(LAUNCHES.get() + 1);
                                async { Ok::<_, String>(CONTENT_HEIGHT) }
                            })
                            .child(blog_list_content)
                            .boxed();

                        Container::new()
                            .color(theme.background_color)
                            .child(Scrollable::new().axis(ScrollAxis::Vertical).child(
                                Container::new().child(Column::new().children([
                                    SizedBox::new().height(32).boxed(),
                                    content,
                                    SizedBox::new().height(48).boxed(),
                                ])),
                            ))
                            .to_element(ctx)
                    },
                    None,
                    "BlogListPage",
                )
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "BlogListPage"
            }
        }

        impl aimer::PortableWidget for BlogListPage {}

        fn blog_list_content(snapshot: &AsyncSnapshot<u32, String>) -> AnyWidget {
            match snapshot {
                AsyncSnapshot::Waiting => Marker::new("waiting", 40).boxed(),
                AsyncSnapshot::Error(_) => Marker::new("error", 40).boxed(),
                AsyncSnapshot::Data(height) => Marker::new("data", *height).boxed(),
            }
        }

        /// The page navigated to.
        ///
        /// Built like every other screen of the site — the theme, a scroll view, an
        /// `AsyncBuilder` — because that is what makes a mistaken hand-over visible: an
        /// `AsyncBuilder` here and one on the blog page are the same widget under the
        /// same name, so state grafted from one renders the other's content.
        struct LearnPage;

        impl Widget for LearnPage {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        let theme = ThemeData::of(ctx);

                        let content = AsyncBuilder::new()
                            .future(|| async { Ok::<_, String>(120_u32) })
                            .child(learn_content)
                            .boxed();

                        Container::new()
                            .color(theme.background_color)
                            .child(Scrollable::new().axis(ScrollAxis::Vertical).child(
                                Container::new().child(Column::new().children([
                                    SizedBox::new().height(32).boxed(),
                                    content,
                                    SizedBox::new().height(48).boxed(),
                                ])),
                            ))
                            .to_element(ctx)
                    },
                    None,
                    "LearnPage",
                )
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "LearnPage"
            }
        }

        impl aimer::PortableWidget for LearnPage {}

        fn learn_content(snapshot: &AsyncSnapshot<u32, String>) -> AnyWidget {
            match snapshot {
                AsyncSnapshot::Waiting => Marker::new("learn-waiting", 40).boxed(),
                AsyncSnapshot::Error(_) => Marker::new("learn-error", 40).boxed(),
                AsyncSnapshot::Data(height) => Marker::new("learn", *height).boxed(),
            }
        }

        // ---------------------------------------------------------------------------
        // A leaf that records the frames it was painted in.
        // ---------------------------------------------------------------------------

        struct Marker {
            label: &'static str,
            height: u32,
        }

        impl Marker {
            fn new(label: &'static str, height: u32) -> Self {
                Self { label, height }
            }
        }

        impl Widget for Marker {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                MarkerElement {
                    label: self.label,
                    child: SizedBox::new().height(self.height).to_element(ctx),
                }
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "Marker"
            }
        }

        impl aimer::PortableWidget for Marker {}

        struct MarkerElement {
            label: &'static str,
            child: AnyElement,
        }

        impl VisitorElement for MarkerElement {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }

            fn debug_name(&self) -> &'static str {
                "Marker"
            }
        }

        impl Drawable for MarkerElement {
            fn update(&self, ctx: &BuildContext) {
                PAINTED.with_borrow_mut(|painted| painted.push(self.label));
                self.child.update(ctx);
            }
        }

        impl EventElement for MarkerElement {}

        impl Rebuildable for MarkerElement {
            fn rebuild_if_dirty(&self, ctx: &BuildContext) {
                self.child.rebuild_if_dirty(ctx);
            }

            fn mark_needs_rebuild(&self) {
                self.child.mark_needs_rebuild();
            }
        }

        impl LayoutElement for MarkerElement {
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

            fn layer(&self) -> u32 {
                self.child.layer()
            }

            fn flex(&self) -> Option<f32> {
                self.child.flex()
            }

            fn get_size_from_child(&self) -> Option<Size> {
                self.child.get_size_from_child()
            }

            fn invalidate_layout(&self) {
                self.child.invalidate_layout();
            }

            fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
                self.child.pos_start_end()
            }
        }

        // ---------------------------------------------------------------------------

        /// Starts the application on the blog route and draws until its request landed.
        fn app_on_the_loaded_blog_route() -> HeadlessAimerApp<ModalHost<Navigator<TestRoute>>> {
            let mut app =
                AimerApp::start_headless(Navigator::<TestRoute>::new(TestRoute::Blog, |route| {
                    route.boxed()
                }));

            app.render_frame();
            sleep(Duration::from_millis(100));
            app.render_frame();
            app.render_frame();

            assert_eq!(
                painted().last().copied(),
                Some("data"),
                "the request never reached the page: {:?}",
                painted()
            );
            assert_eq!(LAUNCHES.get(), 1);

            app
        }

        #[test]
        fn a_theme_change_keeps_the_request_the_page_already_completed() {
            let mut app = app_on_the_loaded_blog_route();
            clear_painted();

            THEME.with_borrow(|theme| {
                theme
                    .as_ref()
                    .expect("the shell state should have published its updater")
                    .set_state(|state| state.dark = !state.dark)
            });

            // Draw the whole transition: every frame of it rebuilds the page.
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            assert_eq!(
                LAUNCHES.get(),
                1,
                "the theme change started the request again; the page painted {:?}",
                painted()
            );
            assert!(
                !painted().contains(&"waiting"),
                "the theme change painted the waiting state again: {:?}",
                painted()
            );
        }

        #[test]
        fn a_navigation_replaces_the_page_the_transition_kept_its_identity_for() {
            let mut app = app_on_the_loaded_blog_route();

            NAVIGATOR.with_borrow(|navigator| {
                navigator
                    .as_ref()
                    .expect("the blog page should have looked up the navigator")
                    .push(TestRoute::Learn)
            });

            // Draw past the end of the cross-fade.
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            clear_painted();
            app.render_frame();

            assert_eq!(
                painted(),
                vec!["learn"],
                "the navigation did not settle on the page pushed"
            );
        }
    }

    mod theme_toggle_keyed_selection {
        //! A theme change must not reset a keyed section's selection.
        //!
        //! `website/src/screen/home_screen.rs` mounts `SameLookingSection` with an
        //! explicit key inside a `Scrollable`, and
        //! `website/src/components/app_shell.rs` animates the theme above it through a
        //! *stateless* themed frame. Every tick of that transition rebuilds the frame,
        //! which rebuilds the page and the section below it.
        //!
        //! The selection the user made lives in the section's own `State`, so it must
        //! survive that rebuild — the section is the same widget, under the same key,
        //! in the same place.

        use std::cell::RefCell;
        use std::thread::sleep;
        use std::time::Duration;

        use aimer::animation::{AnimatedSwitcher, curves};
        use aimer::base::{ResolvedSize, Size, Vec2d};
        use aimer::modal::ModalHost;
        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::router::{Navigator, Outlet, Route, Router, Shell};
        use aimer::style::{AnimatedTheme, Theme, ThemeData};
        use aimer::{
            AimerApp, AnyElement, AnyWidget, BuildContext, Column, Container, Drawable, Element,
            EventElement, Expanded, Key, LayoutElement, Rebuildable, ScrollAxis, Scrollable,
            SizedBox, State, StateUpdater, StatefulElement, StatefulWidget, StatelessElement,
            VisitorElement, Widget,
        };

        /// The identity the section keeps wherever the page rebuilds it, as in
        /// `website/src/screen/home_screen.rs`.
        const SECTION_KEY: &str = "same-looking-section";
        /// The identity the cross-fade inside the section keeps, as in
        /// `website/src/components/same_looking.rs`.
        const SWITCHER_KEY: &str = "platform-image-switcher";
        /// The identity every route's transition shares, as in `website/src/router.rs`.
        const ROUTE_SWITCHER_KEY: &str = "route-switcher";

        const PLATFORMS: [&str; 4] = ["macos", "ios", "web", "android"];

        thread_local! {
            /// The index each frame actually painted, in order.
            static PAINTED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
            /// The shell's state, so a test can toggle the theme the way the header does.
            static THEME: RefCell<Option<StateUpdater<AppShellState>>> = const { RefCell::new(None) };
            /// The section's state, so a test can select a platform the way a tap does.
            static SELECTION: RefCell<Option<StateUpdater<SectionState>>> = const { RefCell::new(None) };
        }

        fn painted() -> Vec<usize> {
            PAINTED.with_borrow(|painted| painted.clone())
        }

        fn clear_painted() {
            PAINTED.with_borrow_mut(|painted| painted.clear());
        }

        // ---------------------------------------------------------------------------
        // The route: `website/src/router.rs`.
        // ---------------------------------------------------------------------------

        #[derive(Clone, Debug, PartialEq)]
        struct HomeRoute;

        impl Route for HomeRoute {
            fn parse(path: &str) -> Option<Self> {
                (path == "/").then_some(Self)
            }

            fn format(&self) -> String {
                "/".to_owned()
            }
        }

        impl Router for HomeRoute {
            fn build(&self, _ctx: &BuildContext) -> AnyWidget {
                Shell::new(AppShell, |_| {
                    AnimatedSwitcher::new(
                        Duration::from_millis(200),
                        curves::FAST_OUT_SLOW_IN,
                        HomePage.boxed(),
                    )
                    .child_key("home")
                    .key(ROUTE_SWITCHER_KEY)
                    .boxed()
                })
                .boxed()
            }
        }

        impl Widget for HomeRoute {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                Router::build(&self, ctx).to_element(ctx)
            }
        }

        impl aimer::PortableWidget for HomeRoute {}

        // ---------------------------------------------------------------------------
        // The shell: `website/src/components/app_shell.rs`.
        // ---------------------------------------------------------------------------

        struct AppShell;

        struct AppShellState {
            dark: bool,
        }

        impl StatefulWidget for AppShell {
            type State = AppShellState;

            fn create_state(self) -> Self::State {
                AppShellState { dark: false }
            }
        }

        impl State<AppShell> for AppShellState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                THEME.replace(Some(updater));
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                AnimatedTheme::new()
                    .data(if self.dark {
                        ThemeData::dark()
                    } else {
                        ThemeData::light()
                    })
                    .duration(Duration::from_millis(250))
                    .curve(curves::EASE_IN_OUT)
                    .child(ThemedFrame)
            }
        }

        impl Widget for AppShell {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatefulElement::new_with_name(self, ctx, "AppShell", None)
                    .0
                    .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "AppShell"
            }
        }

        impl aimer::PortableWidget for AppShell {}

        /// The themed frame around the `Outlet`: a *stateless* widget that reads the
        /// theme, so every tick of the transition rebuilds it and everything below.
        struct ThemedFrame;

        impl Widget for ThemedFrame {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatelessElement::from_builder(
                    ctx,
                    move |ctx| {
                        let theme = ThemeData::of(ctx);
                        Container::new()
                            .color(theme.background_color)
                            .child(
                                Column::new().children([
                                    SizedBox::new().height(40).boxed(),
                                    Expanded::new()
                                        .child(
                                            Container::new()
                                                .color(theme.background_color)
                                                .child(Outlet),
                                        )
                                        .boxed(),
                                ]),
                            )
                            .to_element(ctx)
                    },
                    None,
                    "ThemedFrame",
                )
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "ThemedFrame"
            }
        }

        impl aimer::PortableWidget for ThemedFrame {}

        // ---------------------------------------------------------------------------
        // The page: `website/src/screen/home_screen.rs`.
        // ---------------------------------------------------------------------------

        struct HomePage;

        struct HomePageState;

        impl StatefulWidget for HomePage {
            type State = HomePageState;

            fn create_state(self) -> Self::State {
                HomePageState
            }
        }

        impl State<HomePage> for HomePageState {
            fn init_state(&mut self, _updater: StateUpdater<Self>) {}

            fn build(&self, ctx: &BuildContext) -> impl Widget {
                let theme = ThemeData::of(ctx);
                Container::new().color(theme.background_color).child(
                    Scrollable::new().axis(ScrollAxis::Vertical).child(
                        Column::new().children([
                            SizedBox::new().height(32).boxed(),
                            SelectionSection {
                                key: Some(SECTION_KEY.into()),
                            }
                            .boxed(),
                            SizedBox::new().height(48).boxed(),
                        ]),
                    ),
                )
            }
        }

        impl Widget for HomePage {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatefulElement::new_with_name(self, ctx, "HomePage", None)
                    .0
                    .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "HomePage"
            }
        }

        impl aimer::PortableWidget for HomePage {}

        // ---------------------------------------------------------------------------
        // The section: `website/src/components/same_looking.rs`.
        // ---------------------------------------------------------------------------

        struct SelectionSection {
            key: Option<Key>,
        }

        struct SectionState {
            current_index: usize,
            state: StateUpdater<Self>,
        }

        impl StatefulWidget for SelectionSection {
            type State = SectionState;

            fn create_state(self) -> Self::State {
                SectionState {
                    current_index: 0,
                    state: StateUpdater::new(),
                }
            }
        }

        impl State<SelectionSection> for SectionState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                self.state = updater;
            }

            fn build(&self, ctx: &BuildContext) -> impl Widget {
                let theme = ThemeData::of(ctx);
                let index = self.current_index;
                // The real section hands this same clone to its buttons' callback, so a
                // test drives exactly the updater a tap would.
                SELECTION.replace(Some(self.state));
                Container::new().color(theme.background_color).child(
                    Column::new().children([
                        Container::new()
                            .height(120)
                            .child(
                                AnimatedSwitcher::new(
                                    Duration::from_millis(350),
                                    curves::FAST_OUT_SLOW_IN,
                                    Marker::new(index),
                                )
                                .child_key(PLATFORMS[index % PLATFORMS.len()])
                                .key(SWITCHER_KEY),
                            )
                            .boxed(),
                        SizedBox::new().height(40).boxed(),
                    ]),
                )
            }
        }

        impl Widget for SelectionSection {
            fn key(&self) -> Option<Key> {
                self.key.clone()
            }

            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                let __key = Widget::key(&self);
                StatefulElement::new_with_name(self, ctx, "SelectionSection", __key)
                    .0
                    .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "SelectionSection"
            }
        }

        impl aimer::PortableWidget for SelectionSection {}

        // ---------------------------------------------------------------------------
        // A leaf that records the index it was painted with.
        // ---------------------------------------------------------------------------

        struct Marker {
            index: usize,
        }

        impl Marker {
            fn new(index: usize) -> Self {
                Self { index }
            }
        }

        impl Widget for Marker {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                MarkerElement {
                    index: self.index,
                    child: SizedBox::new().height(80).to_element(ctx),
                }
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "Marker"
            }
        }

        impl aimer::PortableWidget for Marker {}

        struct MarkerElement {
            index: usize,
            child: AnyElement,
        }

        impl VisitorElement for MarkerElement {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }

            fn debug_name(&self) -> &'static str {
                "Marker"
            }
        }

        impl Drawable for MarkerElement {
            fn update(&self, ctx: &BuildContext) {
                PAINTED.with_borrow_mut(|painted| painted.push(self.index));
                self.child.update(ctx);
            }
        }

        impl EventElement for MarkerElement {}

        impl Rebuildable for MarkerElement {
            fn rebuild_if_dirty(&self, ctx: &BuildContext) {
                self.child.rebuild_if_dirty(ctx);
            }

            fn mark_needs_rebuild(&self) {
                self.child.mark_needs_rebuild();
            }
        }

        impl LayoutElement for MarkerElement {
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

            fn layer(&self) -> u32 {
                self.child.layer()
            }

            fn flex(&self) -> Option<f32> {
                self.child.flex()
            }

            fn get_size_from_child(&self) -> Option<Size> {
                self.child.get_size_from_child()
            }

            fn invalidate_layout(&self) {
                self.child.invalidate_layout();
            }

            fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
                self.child.pos_start_end()
            }
        }

        // ---------------------------------------------------------------------------

        fn select(index: usize) {
            SELECTION.with_borrow(|selection| {
                selection
                    .as_ref()
                    .expect("the section should have published its updater")
                    .set_state(move |state| state.current_index = index)
            });
        }

        fn toggle_theme() {
            THEME.with_borrow(|theme| {
                theme
                    .as_ref()
                    .expect("the shell should have published its updater")
                    .set_state(|state| state.dark = !state.dark)
            });
        }

        fn app_on_the_selected_section() -> HeadlessAimerApp<ModalHost<Navigator<HomeRoute>>> {
            let mut app =
                AimerApp::start_headless(Navigator::<HomeRoute>::new(HomeRoute, |route| {
                    route.boxed()
                }));

            app.render_frame();
            select(1);
            // Draw past the cross-fade the selection starts.
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            clear_painted();
            app.render_frame();
            assert_eq!(
                painted().last().copied(),
                Some(1),
                "the selection never reached the section: {:?}",
                painted()
            );

            app
        }

        #[test]
        fn a_theme_change_keeps_the_selection_a_keyed_section_holds() {
            let mut app = app_on_the_selected_section();

            toggle_theme();

            // Draw the whole transition: every frame of it rebuilds the section.
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            clear_painted();
            app.render_frame();

            assert_eq!(
                painted().last().copied(),
                Some(1),
                "the theme change reset the selection: {:?}",
                painted()
            );
        }

        #[test]
        fn a_selection_still_changes_after_a_theme_change() {
            let mut app = app_on_the_selected_section();

            toggle_theme();
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            select(2);
            for _ in 0..20 {
                app.render_frame();
                sleep(Duration::from_millis(20));
            }

            clear_painted();
            app.render_frame();

            assert_eq!(
                painted().last().copied(),
                Some(2),
                "the section stopped responding to selections: {:?}",
                painted()
            );
        }
    }

    mod venus_poll_context_headless {
        //! What a running application gives a task spawned from anywhere in the tree.
        //!
        //! Venus polls futures on the thread that owns the frame, where a runtime-backed
        //! future would otherwise find no runtime to build its resources with — a
        //! `reqwest` connector with no reactor to register a socket on, a `sleep` with
        //! no timer wheel. `AimerApp` installs a
        //! [`PollContext`](aimer::venus::PollContext) for the async runtime it created,
        //! so the runtime is findable for the duration of every poll and no longer.
        //!
        //! The example `examples/http_request_button.rs` is this property with a socket
        //! on the end of it; the timer here proves the same wiring without depending on
        //! a network.

        use std::cell::Cell;
        use std::rc::Rc;
        use std::thread::sleep;
        use std::time::{Duration, Instant};

        use aimer::modal::ModalHost;
        use aimer::quiver::aimer_app::HeadlessAimerApp;
        use aimer::{AimerApp, SizedBox, Venus};

        /// The application under test: what it draws is irrelevant, only that it is a
        /// real one, started the way `main` starts it.
        fn app() -> HeadlessAimerApp<ModalHost<SizedBox>> {
            AimerApp::start_headless(SizedBox::new().width(64).height(64))
        }

        #[test]
        fn a_task_spawned_inside_a_running_application_can_find_the_async_runtime() {
            let mut app = app();
            let venus = Venus::current().expect("a running application installs its runtime");

            let found = Rc::new(Cell::new(false));
            let seen = found.clone();
            venus.spawn(async move { seen.set(tokio::runtime::Handle::try_current().is_ok()) });

            app.render_frame();

            assert!(found.get());
        }

        /// The completion half: the timer runs on the async runtime's own threads and
        /// the task resumes on the UI thread, still holding a non-`Send` capture.
        #[test]
        fn a_runtime_backed_future_resolves_into_a_frame() {
            let mut app = app();
            let venus = Venus::current().expect("a running application installs its runtime");

            let slept = Rc::new(Cell::new(false));
            let flag = slept.clone();
            venus.spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                flag.set(true);
            });

            let deadline = Instant::now() + Duration::from_secs(5);
            while !slept.get() && Instant::now() < deadline {
                app.render_frame();
                sleep(Duration::from_millis(5));
            }

            assert!(slept.get(), "the timer never resolved on the UI thread");
        }
    }

    // The deprecated attribute stays covered until it is removed.
    #[allow(deprecated)]
    mod widget_attribute {
        use aimer::portable::{
            AimerReflectionType, DecodeError, Decoder, EncodeError, Encoder, FieldDescriptor,
            FieldKind, PortableApply, PortableEncode, StableId128, TypeSchema,
        };
        use aimer::*;

        #[widget(Stateless)]
        struct TupleAttributeStateless(String);

        impl StatelessWidget for TupleAttributeStateless {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.clone())
            }
        }

        #[widget(Stateless)]
        enum EnumAttributeStateless {
            Label(String),
            Empty,
        }

        impl StatelessWidget for EnumAttributeStateless {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(match self {
                    Self::Label(label) => label.clone(),
                    Self::Empty => String::from("empty"),
                })
            }
        }

        #[widget(Stateful)]
        struct TupleAttributeStateful(i32);

        struct TupleAttributeStatefulState(i32);

        const TUPLE_STATE_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::new("value", "i32", FieldKind::Retained)];
        const TUPLE_STATE_SCHEMA: TypeSchema = TypeSchema::new(
            "TupleAttributeStatefulState",
            StableId128::from_path("aimer.type.v1", "tests::TupleAttributeStatefulState"),
            TUPLE_STATE_FIELDS,
        );

        impl AimerReflectionType for TupleAttributeStatefulState {
            const TYPE_ID: StableId128 =
                StableId128::from_path("aimer.type.v1", "tests::TupleAttributeStatefulState");

            fn schema() -> &'static TypeSchema {
                &TUPLE_STATE_SCHEMA
            }
        }

        impl PortableEncode for TupleAttributeStatefulState {
            fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                encoder.field(&TUPLE_STATE_FIELDS[0], |encoder| self.0.encode(encoder))
            }
        }

        impl PortableApply for TupleAttributeStatefulState {
            type Retained = i32;

            fn decode_retained(decoder: &mut Decoder<'_>) -> Result<Self::Retained, DecodeError> {
                Ok(decoder.field(&TUPLE_STATE_FIELDS[0])?.unwrap())
            }

            fn apply_retained(&mut self, retained: Self::Retained) {
                self.0 = retained;
            }
        }

        impl StatefulWidget for TupleAttributeStateful {
            type State = TupleAttributeStatefulState;

            fn create_state(self) -> Self::State {
                TupleAttributeStatefulState(self.0)
            }
        }

        impl State<TupleAttributeStateful> for TupleAttributeStatefulState {
            fn init_state(&mut self, _: StateUpdater<Self>) {}

            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.to_string())
            }
        }

        #[widget(Stateful)]
        enum EnumAttributeStateful {
            Count(i32),
            Empty,
        }

        struct EnumAttributeStatefulState(i32);

        const ENUM_STATE_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::new("value", "i32", FieldKind::Retained)];
        const ENUM_STATE_SCHEMA: TypeSchema = TypeSchema::new(
            "EnumAttributeStatefulState",
            StableId128::from_path("aimer.type.v1", "tests::EnumAttributeStatefulState"),
            ENUM_STATE_FIELDS,
        );

        impl AimerReflectionType for EnumAttributeStatefulState {
            const TYPE_ID: StableId128 =
                StableId128::from_path("aimer.type.v1", "tests::EnumAttributeStatefulState");

            fn schema() -> &'static TypeSchema {
                &ENUM_STATE_SCHEMA
            }
        }

        impl PortableEncode for EnumAttributeStatefulState {
            fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                encoder.field(&ENUM_STATE_FIELDS[0], |encoder| self.0.encode(encoder))
            }
        }

        impl PortableApply for EnumAttributeStatefulState {
            type Retained = i32;

            fn decode_retained(decoder: &mut Decoder<'_>) -> Result<Self::Retained, DecodeError> {
                Ok(decoder.field(&ENUM_STATE_FIELDS[0])?.unwrap())
            }

            fn apply_retained(&mut self, retained: Self::Retained) {
                self.0 = retained;
            }
        }

        impl StatefulWidget for EnumAttributeStateful {
            type State = EnumAttributeStatefulState;

            fn create_state(self) -> Self::State {
                EnumAttributeStatefulState(match self {
                    Self::Count(count) => count,
                    Self::Empty => 0,
                })
            }
        }

        impl State<EnumAttributeStateful> for EnumAttributeStatefulState {
            fn init_state(&mut self, _: StateUpdater<Self>) {}

            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.to_string())
            }
        }

        fn assert_widget<W: Widget>() {}

        #[test]
        fn stateless_attribute_supports_tuple_structs_and_enums() {
            assert_widget::<TupleAttributeStateless>();
            assert_widget::<EnumAttributeStateless>();
        }

        #[test]
        fn stateful_attribute_supports_tuple_structs_and_enums() {
            assert_widget::<TupleAttributeStateful>();
            assert_widget::<EnumAttributeStateful>();
            assert_eq!(TupleAttributeStateful(4).create_state().0, 4);
            assert_eq!(EnumAttributeStateful::Count(9).create_state().0, 9);
        }
    }

    mod widget_derive {
        //! The derive forms that replace `#[widget(...)]`, exercised the way a user writes them.
        //!
        //! The unit tests in `aimer_macro` compare token streams; these compile the
        //! generated code against the real traits, which is the only way to catch a
        //! path that resolves inside the macro crate but not at the call site.

        use aimer::portable::{
            AimerReflectionType, DecodeError, Decoder, EncodeError, Encoder, FieldDescriptor,
            FieldKind, PortableApply, PortableEncode, StableId128, TypeSchema,
        };
        use aimer::router::Route;
        use aimer::*;

        macro_rules! impl_portable_i32_state {
            ($fields:ident, $schema:ident, $state:ty, $field:tt) => {
                const $fields: &[FieldDescriptor] =
                    &[FieldDescriptor::new("value", "i32", FieldKind::Retained)];
                const $schema: TypeSchema = TypeSchema::new(
                    stringify!($state),
                    StableId128::from_path("aimer.type.v1", concat!("tests::", stringify!($state))),
                    $fields,
                );

                impl AimerReflectionType for $state {
                    const TYPE_ID: StableId128 = StableId128::from_path(
                        "aimer.type.v1",
                        concat!("tests::", stringify!($state)),
                    );

                    fn schema() -> &'static TypeSchema {
                        &$schema
                    }
                }

                impl PortableEncode for $state {
                    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                        encoder.field(&$fields[0], |encoder| self.$field.encode(encoder))
                    }
                }

                impl PortableApply for $state {
                    type Retained = i32;

                    fn decode_retained(
                        decoder: &mut Decoder<'_>,
                    ) -> Result<Self::Retained, DecodeError> {
                        Ok(decoder.field(&$fields[0])?.unwrap())
                    }

                    fn apply_retained(&mut self, retained: Self::Retained) {
                        self.$field = retained;
                    }
                }
            };
        }

        #[derive(Clone, StatelessWidget)]
        struct Greeting {
            name: String,
        }

        impl StatelessWidget for Greeting {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(format!("Hello, {}", self.name))
            }
        }

        #[derive(Clone, StatelessWidget)]
        struct KeyedGreeting {
            key: Option<Key>,
        }

        impl StatelessWidget for KeyedGreeting {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new("keyed")
            }
        }

        #[derive(StatefulWidget)]
        struct Counter {
            initial_count: i32,
        }

        struct CounterState {
            count: i32,
        }

        impl_portable_i32_state!(
            COUNTER_STATE_FIELDS,
            COUNTER_STATE_SCHEMA,
            CounterState,
            count
        );

        impl StatefulWidget for Counter {
            type State = CounterState;

            fn create_state(self) -> CounterState {
                CounterState {
                    count: self.initial_count,
                }
            }
        }

        impl State<Counter> for CounterState {
            fn init_state(&mut self, _: StateUpdater<Self>) {}

            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(format!("{}", self.count))
            }
        }

        #[derive(StatelessWidget)]
        struct TupleStateless(String);

        impl StatelessWidget for TupleStateless {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.clone())
            }
        }

        #[derive(StatelessWidget)]
        enum EnumStateless {
            Label(String),
            Empty,
        }

        impl StatelessWidget for EnumStateless {
            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(match self {
                    Self::Label(label) => label.clone(),
                    Self::Empty => String::from("empty"),
                })
            }
        }

        #[derive(StatefulWidget)]
        struct TupleStateful(i32);

        struct TupleStatefulState(i32);

        impl_portable_i32_state!(
            TUPLE_STATEFUL_STATE_FIELDS,
            TUPLE_STATEFUL_STATE_SCHEMA,
            TupleStatefulState,
            0
        );

        impl StatefulWidget for TupleStateful {
            type State = TupleStatefulState;

            fn create_state(self) -> Self::State {
                TupleStatefulState(self.0)
            }
        }

        impl State<TupleStateful> for TupleStatefulState {
            fn init_state(&mut self, _: StateUpdater<Self>) {}

            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.to_string())
            }
        }

        #[derive(StatefulWidget)]
        enum EnumStateful {
            Count(i32),
            Empty,
        }

        struct EnumStatefulState(i32);

        impl_portable_i32_state!(
            ENUM_STATEFUL_STATE_FIELDS,
            ENUM_STATEFUL_STATE_SCHEMA,
            EnumStatefulState,
            0
        );

        impl StatefulWidget for EnumStateful {
            type State = EnumStatefulState;

            fn create_state(self) -> Self::State {
                EnumStatefulState(match self {
                    Self::Count(count) => count,
                    Self::Empty => 0,
                })
            }
        }

        impl State<EnumStateful> for EnumStatefulState {
            fn init_state(&mut self, _: StateUpdater<Self>) {}

            fn build(&self, _: &BuildContext) -> impl Widget {
                Text::new(self.0.to_string())
            }
        }

        #[derive(Clone, Debug, PartialEq, Router)]
        enum AppRoute {
            #[route("/")]
            Home,
            #[route("/profile/{name}", name = "profile")]
            Profile { name: String },
            #[route("/search?q={q}&page={page}")]
            Search { q: String, page: u32 },
            #[shell("/dashboard")]
            Dashboard(DashRoute),
        }

        #[derive(Clone, Debug, PartialEq, Router)]
        enum DashRoute {
            #[route("/")]
            Overview,
            #[route("/reports")]
            Reports,
        }

        impl router::Router for AppRoute {
            fn build(&self, _: &BuildContext) -> AnyWidget {
                Greeting {
                    name: "route".to_string(),
                }
                .boxed()
            }
        }

        impl router::Router for DashRoute {
            fn build(&self, _: &BuildContext) -> AnyWidget {
                Greeting {
                    name: "dash".to_string(),
                }
                .boxed()
            }
        }

        fn assert_widget<W: Widget>() {}

        #[test]
        fn the_stateless_derive_makes_the_struct_a_widget() {
            assert_widget::<Greeting>();

            let greeting = Greeting {
                name: "aimer".to_string(),
            };
            assert_eq!(Widget::debug_name(&greeting), "Greeting");
            assert!(Widget::key(&greeting).is_none());
        }

        #[test]
        fn the_stateless_derive_forwards_a_key_field() {
            let keyed = KeyedGreeting {
                key: Some(Key::Static("greeting")),
            };

            assert_eq!(Widget::key(&keyed), Some(Key::Static("greeting")));
        }

        #[test]
        fn the_stateful_derive_makes_the_struct_a_widget() {
            assert_widget::<Counter>();

            let counter = Counter { initial_count: 7 };
            assert_eq!(Widget::debug_name(&counter), "Counter");
            assert_eq!(counter.create_state().count, 7);
        }

        #[test]
        fn stateless_derive_supports_tuple_structs_and_enums() {
            assert_widget::<TupleStateless>();
            assert_widget::<EnumStateless>();
            assert!(Widget::key(&TupleStateless(String::from("tuple"))).is_none());
            assert!(Widget::key(&EnumStateless::Empty).is_none());
        }

        #[test]
        fn stateful_derive_supports_tuple_structs_and_enums() {
            assert_widget::<TupleStateful>();
            assert_widget::<EnumStateful>();
            assert_eq!(TupleStateful(4).create_state().0, 4);
            assert_eq!(EnumStateful::Count(9).create_state().0, 9);
        }

        #[test]
        fn the_router_derive_makes_the_enum_a_widget() {
            assert_widget::<AppRoute>();
            assert_widget::<DashRoute>();
        }

        #[test]
        fn the_router_derive_parses_and_formats_paths() {
            assert_eq!(AppRoute::parse("/"), Some(AppRoute::Home));
            assert_eq!(
                AppRoute::parse("/profile/john"),
                Some(AppRoute::Profile {
                    name: "john".to_string()
                })
            );
            assert_eq!(AppRoute::Home.format(), "/");
            assert_eq!(
                AppRoute::Profile {
                    name: "john".to_string()
                }
                .format(),
                "/profile/john"
            );
        }

        #[test]
        fn the_router_derive_reads_the_query_string() {
            assert_eq!(
                AppRoute::parse("/search?q=aimer&page=2"),
                Some(AppRoute::Search {
                    q: "aimer".to_string(),
                    page: 2,
                })
            );
        }

        #[test]
        fn the_router_derive_delegates_a_shell_to_its_child_enum() {
            assert_eq!(
                AppRoute::parse("/dashboard/reports"),
                Some(AppRoute::Dashboard(DashRoute::Reports))
            );
            assert_eq!(
                AppRoute::Dashboard(DashRoute::Reports).format(),
                "/dashboard/reports"
            );
        }

        #[test]
        fn the_router_derive_resolves_a_named_route() {
            let params =
                std::collections::HashMap::from([("name".to_string(), "john".to_string())]);

            assert_eq!(
                AppRoute::resolve_named("profile", &params),
                Some(AppRoute::Profile {
                    name: "john".to_string()
                })
            );
            assert_eq!(
                AppRoute::Profile {
                    name: "john".to_string()
                }
                .name(),
                Some("profile")
            );
        }
    }
}
