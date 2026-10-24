mod single_child;

pub use single_child::aspecratio::{AspectRatio, RatioOption};
pub use single_child::container::Container;
#[doc(hidden)]
pub use single_child::container::RetainedBoxDecoration;
pub use single_child::custom_shape::CustomShape;
pub use single_child::glass::{Glass, GlassMaterial, MaterialMotionPolicy};
pub use single_child::liquid::{Liquid, LiquidMaterial};
pub use single_child::opacity::Opacity;
pub use single_child::resizable::band::ResizeBand;
pub use single_child::resizable::direction::Direction;
pub use single_child::resizable::handle::ResizeHandle;
pub use single_child::resizable::{RawResizable, Resizable};
pub use single_child::scalable::Scalable;
pub use single_child::sized_box::SizedBox;
pub use single_child::spacing::{Margin, Padding};
pub use single_child::zero_size_box::ZeroSizedBox;

#[doc(no_inline)]
pub use aimer_utils::cursor::CursorIcon;

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use aimer_attribute::BoxConstraint;
    use aimer_attribute::size::{ResolvedSize, Size};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_flex::flex_child::RawExpanded;
    use aimer_flex::raw_flex::RawFlex;
    use aimer_flex::{Column, Expanded, FlexDirection, Row};
    use aimer_scroll::{ScrollAxis, Scrollable};
    use aimer_space::positioned::RawPositionedElement;
    use aimer_space::{Positioned, Stack};
    use aimer_widget::base::BuildContext;
    use aimer_widget::{
        AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, NamedWidget,
        Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget, StatelessElement,
        VisitorElement, Widget,
    };

    use super::*;

    struct MeasuredPositionedChild {
        observed_parent_size: Rc<Cell<ResolvedSize>>,
    }

    impl Drawable for MeasuredPositionedChild {
        fn update(&self, ctx: &BuildContext) {
            self.observed_parent_size.set(ctx.parent_size);
        }
    }

    impl EventElement for MeasuredPositionedChild {}

    impl LayoutElement for MeasuredPositionedChild {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.content_size(ctx)
        }

        fn content_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 320.0,
                height: 96.0,
            }
        }

        fn get_size_from_child(&self) -> Option<Size> {
            Some(Size {
                width: 320.into(),
                height: 0.into(),
            })
        }
    }

    impl Rebuildable for MeasuredPositionedChild {}
    impl VisitorElement for MeasuredPositionedChild {
        fn debug_name(&self) -> &'static str {
            "MeasuredPositionedChild"
        }
    }

    // ─── A faithful stand-in for a `TextButton` ───────────────────────────
    //
    // Mirrors `crates/aimer_input/src/text_button.rs`: a `StatefulWidget`
    // whose `State` stores the parent-provided config (`index`, `selected`)
    // plus a runtime field (`hovered`). It refreshes the config on reconcile
    // via `adopt_config_from` exactly like `ButtonState` does, and records its
    // rendered `selected` value into a shared observer so the test can see
    // which button believes it is highlighted. Constructed via `NamedWidget`
    // (as `#[derive(WidgetConstructor)]` does for `TextButton`) so the element
    // goes through the exact same wrapper path.
    struct ButtonLike {
        index: usize,
        selected: bool,
        observers: Rc<Vec<Rc<Cell<i32>>>>,
    }

    struct ButtonLikeState {
        index: usize,
        selected: bool,
        #[allow(dead_code)]
        hovered: bool,
        observers: Rc<Vec<Rc<Cell<i32>>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for ButtonLike {
        type State = ButtonLikeState;

        fn create_state(self) -> Self::State {
            ButtonLikeState {
                index: self.index,
                selected: self.selected,
                hovered: false,
                observers: self.observers.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl Widget for ButtonLike {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            // Bare stateful with the default "Unknown" name, exactly like
            // `TextButton::to_element`. The surrounding `NamedWidget` then
            // wraps it in a `StatelessElement("ButtonLike")`.
            StatefulElement::new(self, ctx).0.boxed()
        }
    }

    impl aimer_widget::PortableWidget for ButtonLike {}

    impl State<ButtonLike> for ButtonLikeState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        // Mirrors `ButtonState::adopt_config_from`: refresh the config
        // (`index`, `selected`) while keeping runtime (`hovered`).
        fn adopt_config_from(&mut self, new: Self) {
            self.index = new.index;
            self.selected = new.selected;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.observers[self.index].set(if self.selected { 1 } else { 0 });
            Container::new().height(32).child(crate::ZeroSizedBox)
        }
    }

    fn button(index: usize, selected: bool, observers: Rc<Vec<Rc<Cell<i32>>>>) -> AnyWidget {
        NamedWidget::new(
            ButtonLike {
                index,
                selected,
                observers,
            }
            .boxed(),
            "ButtonLike",
        )
        .boxed()
    }

    struct TabWidget {
        observer: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<TabState>>>>,
        button_observers: Rc<Vec<Rc<Cell<i32>>>>,
    }

    struct TabState {
        index: usize,
        observer: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<Self>>>>,
        button_observers: Rc<Vec<Rc<Cell<i32>>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for TabWidget {
        type State = TabState;

        fn create_state(self) -> Self::State {
            TabState {
                index: 0,
                observer: self.observer.clone(),
                live_updater: self.live_updater.clone(),
                button_observers: self.button_observers.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl Widget for TabWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::new_with_name(self, ctx, "TabWidget", None)
                .0
                .boxed()
        }

        fn debug_name(&self) -> &'static str {
            "TabWidget"
        }
    }

    impl aimer_widget::PortableWidget for TabWidget {}

    impl State<TabWidget> for TabState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.observer.set(self.index);
            *self.live_updater.borrow_mut() = Some(self.updater);
            // Content follows the selection (the image in the real app) AND a
            // Row of buttons whose highlight must follow the selection too.
            Column::new().children(vec![
                Container::new()
                    .height(180)
                    .child(crate::ZeroSizedBox)
                    .boxed(),
                Row::new()
                    .children(vec![
                        button(0, self.index == 0, self.button_observers.clone()),
                        button(1, self.index == 1, self.button_observers.clone()),
                        button(2, self.index == 2, self.button_observers.clone()),
                        button(3, self.index == 3, self.button_observers.clone()),
                    ])
                    .boxed(),
            ])
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Verdict {
        Survived,
        Reset,
    }

    impl Verdict {
        fn classify(observer_after_resize: usize, live_index_after_resize: usize) -> Self {
            match (observer_after_resize, live_index_after_resize) {
                (3, 3) => Self::Survived,
                (0, 0) => Self::Reset,
                other => panic!("unexpected post-resize state: {other:?}"),
            }
        }
    }

    #[derive(Debug)]
    struct VariantResult {
        label: &'static str,
        observer_after_resize: usize,
        live_index_after_resize: usize,
        verdict: Verdict,
        /// For each of the 4 buttons: 1 if it rendered as selected/highlighted,
        /// 0 if not. After picking tab 3 and resizing this must be [0,0,0,1].
        button_highlight_after_resize: Vec<i32>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn dummy_build_context(
        width: f32,
        height: f32,
        visible_rect: Option<(f32, f32, f32, f32)>,
    ) -> BuildContext<'static> {
        let canvas = {
            let leaked: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
            FrameCanvas::new(leaked)
        };

        let mut context = BuildContext::new(
            canvas,
            ResolvedSize { width, height },
            1.0,
            Default::default(),
            Default::default(),
            aimer_widget::base::WindowHandle::headless(
                winit::dpi::PhysicalSize::new(width as u32, height as u32),
                1.0,
            ),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: width,
            max_height: height,
        };
        context.visible_rect = visible_rect;
        context
    }

    fn placeholder_section(height: i32) -> AnyWidget {
        Container::new()
            .height(height)
            .child(crate::ZeroSizedBox)
            .boxed()
    }

    fn build_home_page(
        ctx: &BuildContext,
        observer: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<TabState>>>>,
        button_observers: Rc<Vec<Rc<Cell<i32>>>>,
    ) -> AnyElement {
        let header = Positioned::new()
            .top(0)
            .left(0)
            .layer(1)
            .child(Container::new().height(48).child(ZeroSizedBox))
            .boxed();
        let content = Positioned::new()
            .top(0)
            .left(0)
            .layer(0)
            .child(
                Scrollable::new()
                    .axis(ScrollAxis::Vertical)
                    .child(Column::new().children(vec![
                placeholder_section(100),
                placeholder_section(100),
                placeholder_section(100),
                TabWidget { observer,
                            live_updater,
                            button_observers, }.boxed(),
            ])),
            )
            .boxed();

        Container::new()
            .child(Stack::new().children(vec![header, content]))
            .to_element(ctx)
    }

    fn current_live_updater(
        live_updater: &Rc<RefCell<Option<StateUpdater<TabState>>>>,
    ) -> StateUpdater<TabState> {
        live_updater
            .borrow()
            .as_ref()
            .copied()
            .expect("current live updater should be published from build()")
    }

    fn run_variant(culled: bool, resize_count: usize) -> VariantResult {
        let initial_ctx = dummy_build_context(500.0, 600.0, None);
        let observer = Rc::new(Cell::new(usize::MAX));
        let live_updater = Rc::new(RefCell::new(None));
        let button_observers: Rc<Vec<Rc<Cell<i32>>>> =
            Rc::new((0..4).map(|_| Rc::new(Cell::new(-1))).collect());

        let initial_child = build_home_page(
            &initial_ctx,
            observer.clone(),
            live_updater.clone(),
            button_observers.clone(),
        );
        let rebuild_observer = observer.clone();
        let rebuild_live_updater = live_updater.clone();
        let rebuild_button_observers = button_observers.clone();
        let driver = StatelessElement::new(
            initial_child,
            move |ctx| {
                build_home_page(
                    ctx,
                    rebuild_observer.clone(),
                    rebuild_live_updater.clone(),
                    rebuild_button_observers.clone(),
                )
            },
            None,
            "Root",
        );

        driver.update(&initial_ctx);
        assert_eq!(
            observer.get(),
            0,
            "initial build should publish the default tab index"
        );

        current_live_updater(&live_updater).set_state(|state| state.index = 3);
        driver.update(&initial_ctx);

        assert_eq!(
            observer.get(),
            3,
            "setup failed: observer should record the selected tab before resize"
        );
        assert_eq!(
            current_live_updater(&live_updater).read(|state| state.index),
            3,
            "setup failed: live state should store index=3 before resize"
        );

        let resize_ctx = if culled {
            dummy_build_context(640.0, 250.0, Some((0.0, 0.0, 640.0, 250.0)))
        } else {
            dummy_build_context(640.0, 600.0, None)
        };

        for _ in 0..resize_count {
            Rebuildable::mark_needs_rebuild(&driver);
            driver.update(&resize_ctx);
        }

        let observer_after_resize = observer.get();
        let live_index_after_resize = current_live_updater(&live_updater).read(|state| state.index);
        let verdict = Verdict::classify(observer_after_resize, live_index_after_resize);
        let button_highlight_after_resize: Vec<i32> =
            button_observers.iter().map(|o| o.get()).collect();

        VariantResult {
            label: match (culled, resize_count) {
                (false, 1) => "visible / one resize",
                (false, 2) => "visible / two resizes",
                (true, 1) => "culled / one resize",
                (true, 2) => "culled / two resizes",
                _ => "unexpected",
            },
            observer_after_resize,
            live_index_after_resize,
            verdict,
            button_highlight_after_resize,
        }
    }

    /// Regression for the reported "selected tab is lost on window resize" bug.
    ///
    /// Faithfully reproduces the real `website` tree
    /// (`Container → Stack → Positioned → Scrollable → Column → [.. , Tab]`)
    /// with the actual container widgets and a real `StatefulElement`, picks
    /// tab index 3, then simulates one/two window resizes for both the
    /// on-screen and the flex-culled cases. The selected tab MUST survive every
    /// variant.
    #[test]
    fn real_widget_resize_repro_keeps_selected_tab() {
        let results = [
            run_variant(false, 1),
            run_variant(false, 2),
            run_variant(true, 1),
            run_variant(true, 2),
        ];

        for result in &results {
            eprintln!(
                "{} => observer={}, live_index={}, verdict={:?}, buttons={:?}",
                result.label,
                result.observer_after_resize,
                result.live_index_after_resize,
                result.verdict,
                result.button_highlight_after_resize
            );
        }

        for result in &results {
            assert_eq!(
                result.verdict,
                Verdict::Survived,
                "the selected tab must survive a window resize ({}): observer={}, live_index={}",
                result.label,
                result.observer_after_resize,
                result.live_index_after_resize
            );
        }
    }

    // Locate the `RawScrollableContainer` buried anywhere in an element tree so
    // a test can read its live scroll offset / cached scroll range.
    // fn find_scrollable(el: &dyn Element) ->
    // Option<&RawScrollableContainer<AnyElement>> {     if let Some(s) =
    // el.as_any().downcast_ref::<RawScrollableContainer<AnyElement>>() {
    //         return Some(s);
    //     }
    //     let mut found: Option<&RawScrollableContainer<AnyElement>> = None;
    //     // Some layout elements (e.g. `Positioned`) expose their child through
    //     // `event_children` rather than `visit_children`, so walk both.
    //     el.visit_children(&mut |c| {
    //         if found.is_none() {
    //             found = find_scrollable(c);
    //         }
    //     });
    //     if found.is_none() {
    //         el.event_children(&mut |c| {
    //             if found.is_none() {
    //                 found = find_scrollable(c);
    //             }
    //         });
    //     }
    //     found
    // }

    /// Regression for the reported "button active/selected highlight is stuck
    /// on the initially-selected tab after a window resize" bug.
    ///
    /// The section content follows the live selection (verified by
    /// `real_widget_resize_repro_keeps_selected_tab`), but each platform button
    /// is its own `StatefulWidget` (`TextButton`) whose `State` mirrors the
    /// parent-provided `selected` prop. After picking tab 3 ("Android") and
    /// resizing, ONLY button 3 must render highlighted — the buttons'
    /// `selected` config must be refreshed to match the live selection.
    #[test]
    fn real_widget_resize_repro_keeps_button_highlight() {
        let results = [
            run_variant(false, 1),
            run_variant(false, 2),
            run_variant(true, 1),
            run_variant(true, 2),
        ];

        for result in &results {
            eprintln!(
                "{} => buttons={:?}",
                result.label, result.button_highlight_after_resize
            );
        }

        for result in &results {
            assert_eq!(
                result.button_highlight_after_resize,
                vec![0, 0, 0, 1],
                "after picking tab 3 and resizing, ONLY button 3 must stay highlighted ({}): got {:?}",
                result.label,
                result.button_highlight_after_resize
            );
        }
    }

    // A leaf element that records the main-axis (`max_width`) constraint it is
    // laid out with, so a test can observe exactly how much space its flex
    // parent handed it. It reports a size that fills whatever it is given.
    struct MainAxisProbe {
        seen: Rc<Cell<f32>>,
    }

    impl VisitorElement for MainAxisProbe {
        fn debug_name(&self) -> &'static str {
            "MainAxisProbe"
        }
    }
    impl Drawable for MainAxisProbe {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl EventElement for MainAxisProbe {}
    impl LayoutElement for MainAxisProbe {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.seen.set(ctx.box_constraint.max_width);
            ResolvedSize {
                width: ctx.box_constraint.max_width,
                height: 24.0,
            }
        }
    }
    impl Rebuildable for MainAxisProbe {}

    fn expanded_probe(flex: f32, seen: &Rc<Cell<f32>>) -> AnyElement {
        RawExpanded::new(MainAxisProbe { seen: seen.clone() }, flex, "Expanded").boxed()
    }

    // A leaf element with a fixed intrinsic main-axis size that ignores the
    // constraint it is given — emulating a `Text` (which reports no explicit
    // `size()` yet measures to its content width). It records the constraint it
    // saw so a test can assert it was *not* stretched.
    struct IntrinsicProbe {
        intrinsic_width: f32,
        seen: Rc<Cell<f32>>,
    }

    impl VisitorElement for IntrinsicProbe {
        fn debug_name(&self) -> &'static str {
            "IntrinsicProbe"
        }
    }
    impl Drawable for IntrinsicProbe {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl EventElement for IntrinsicProbe {}
    impl LayoutElement for IntrinsicProbe {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.seen.set(ctx.box_constraint.max_width);
            ResolvedSize {
                width: self.intrinsic_width,
                height: ctx.box_constraint.max_height,
            }
        }
    }
    impl Rebuildable for IntrinsicProbe {}

    fn intrinsic_probe(intrinsic_width: f32, seen: &Rc<Cell<f32>>) -> AnyElement {
        IntrinsicProbe {
            intrinsic_width,
            seen: seen.clone(),
        }
        .boxed()
    }

    struct ConstraintSensitiveProbe {
        intrinsic_width: f32,
        drawn_width: Rc<Cell<f32>>,
    }

    impl VisitorElement for ConstraintSensitiveProbe {
        fn debug_name(&self) -> &'static str {
            "ConstraintSensitiveProbe"
        }
    }
    impl Drawable for ConstraintSensitiveProbe {
        fn update(&self, ctx: &BuildContext) {
            self.drawn_width.set(ctx.box_constraint.max_width);
        }
    }
    impl EventElement for ConstraintSensitiveProbe {}
    impl LayoutElement for ConstraintSensitiveProbe {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: if ctx.box_constraint.max_width == f32::MAX {
                    self.intrinsic_width
                } else {
                    ctx.box_constraint.max_width
                },
                height: 24.0,
            }
        }
    }
    impl Rebuildable for ConstraintSensitiveProbe {}

    fn constraint_sensitive_probe(intrinsic_width: f32, drawn_width: &Rc<Cell<f32>>) -> AnyElement {
        ConstraintSensitiveProbe {
            intrinsic_width,
            drawn_width: drawn_width.clone(),
        }
        .boxed()
    }

    struct ConstraintSensitiveColumnProbe {
        intrinsic_height: f32,
        drawn_height: Rc<Cell<f32>>,
    }

    impl VisitorElement for ConstraintSensitiveColumnProbe {
        fn debug_name(&self) -> &'static str {
            "ConstraintSensitiveColumnProbe"
        }
    }
    impl Drawable for ConstraintSensitiveColumnProbe {
        fn update(&self, ctx: &BuildContext) {
            self.drawn_height.set(ctx.box_constraint.max_height);
        }
    }
    impl EventElement for ConstraintSensitiveColumnProbe {}
    impl LayoutElement for ConstraintSensitiveColumnProbe {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 24.0,
                height: if ctx.box_constraint.max_height == f32::MAX {
                    self.intrinsic_height
                } else {
                    ctx.box_constraint.max_height
                },
            }
        }
    }
    impl Rebuildable for ConstraintSensitiveColumnProbe {}

    fn constraint_sensitive_column_probe(
        intrinsic_height: f32,
        drawn_height: &Rc<Cell<f32>>,
    ) -> AnyElement {
        ConstraintSensitiveColumnProbe {
            intrinsic_height,
            drawn_height: drawn_height.clone(),
        }
        .boxed()
    }

    fn row_of(children: Vec<AnyElement>) -> RawFlex {
        RawFlex::new(FlexDirection::Row, children, "Row")
    }

    fn column_of(children: Vec<AnyElement>) -> RawFlex {
        RawFlex::new(FlexDirection::Column, children, "Column")
    }

    /// A single `Expanded` in a `Row` fills the whole parent width.
    #[test]
    fn expanded_single_child_fills_row() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let c1 = Rc::new(Cell::new(0.0));
        let row = row_of(vec![expanded_probe(1.0, &c1)]);

        let size = row.computed_size(&ctx);

        assert_eq!(
            c1.get(),
            300.0,
            "the only Expanded must receive the full width"
        );
        assert_eq!(
            size,
            ResolvedSize {
                width: 300.0,
                height: 24.0
            }
        );
    }

    /// Two equal `Expanded` children split the width evenly.
    #[test]
    fn expanded_two_equal_children_split_evenly() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let c1 = Rc::new(Cell::new(0.0));
        let c2 = Rc::new(Cell::new(0.0));
        let row = row_of(vec![expanded_probe(1.0, &c1), expanded_probe(1.0, &c2)]);

        let _ = row.computed_size(&ctx);

        assert_eq!(c1.get(), 150.0);
        assert_eq!(c2.get(), 150.0);
    }

    /// `flex = 1` and `flex = 2` split the width 1/3 : 2/3.
    #[test]
    fn expanded_weighted_children_split_proportionally() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let c1 = Rc::new(Cell::new(0.0));
        let c2 = Rc::new(Cell::new(0.0));
        let row = row_of(vec![expanded_probe(1.0, &c1), expanded_probe(2.0, &c2)]);

        let _ = row.computed_size(&ctx);

        assert_eq!(c1.get(), 100.0, "flex=1 child gets 1/3 of the width");
        assert_eq!(c2.get(), 200.0, "flex=2 child gets 2/3 of the width");
    }

    #[test]
    fn zero_weight_expanded_child_receives_no_remaining_space() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let child = Rc::new(Cell::new(f32::MAX));
        let row = row_of(vec![expanded_probe(0.0, &child)]);

        row.update(&ctx);

        assert_eq!(child.get(), 0.0);
    }

    /// A fixed-size sibling is subtracted first; the remaining space is shared
    /// by the flex children according to their weights.
    #[test]
    fn expanded_shares_space_left_by_fixed_sibling() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let c1 = Rc::new(Cell::new(0.0));
        let c2 = Rc::new(Cell::new(0.0));
        let fixed: AnyElement = Container::new()
            .width(60)
            .child(crate::ZeroSizedBox)
            .to_element(&ctx);
        let row = row_of(vec![
            fixed,
            expanded_probe(1.0, &c1),
            expanded_probe(2.0, &c2),
        ]);

        let _ = row.computed_size(&ctx);

        // 300 - 60 = 240 free, split 1:2 => 80 and 160.
        assert_eq!(
            c1.get(),
            80.0,
            "flex=1 child gets 1/3 of the remaining 240px"
        );
        assert_eq!(
            c2.get(),
            160.0,
            "flex=2 child gets 2/3 of the remaining 240px"
        );
    }

    /// Regression for the website header: a size-less intrinsic child (a
    /// `Text`) next to an `Expanded` must NOT be treated as flexible. The
    /// text keeps its intrinsic width and the `Expanded` fills *all* the
    /// remaining space, not half of it.
    #[test]
    fn intrinsic_child_does_not_steal_flex_space_from_expanded() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let text = Rc::new(Cell::new(0.0));
        let exp = Rc::new(Cell::new(0.0));
        // Row: [ Text(width 50), Expanded ]
        let row = row_of(vec![
            intrinsic_probe(50.0, &text),
            expanded_probe(1.0, &exp),
        ]);

        let _ = row.computed_size(&ctx);

        // The Expanded must get the whole remaining 300 - 50 = 250, not (300)/2.
        assert_eq!(
            exp.get(),
            250.0,
            "the Expanded must fill ALL space left by the intrinsic-sized child"
        );
    }

    /// Two intrinsic (size-less) children and a single `Expanded`: the plain
    /// children keep their own widths and the `Expanded` swallows the rest.
    #[test]
    fn multiple_intrinsic_children_keep_size_expanded_fills_rest() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let a = Rc::new(Cell::new(0.0));
        let b = Rc::new(Cell::new(0.0));
        let exp = Rc::new(Cell::new(0.0));
        let row = row_of(vec![
            intrinsic_probe(30.0, &a),
            intrinsic_probe(70.0, &b),
            expanded_probe(1.0, &exp),
        ]);

        let _ = row.computed_size(&ctx);

        assert_eq!(exp.get(), 200.0, "Expanded fills 300 - 30 - 70 = 200");
    }

    #[test]
    fn regular_row_children_draw_at_intrinsic_width() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let first = Rc::new(Cell::new(0.0));
        let second = Rc::new(Cell::new(0.0));
        let row = row_of(vec![
            constraint_sensitive_probe(50.0, &first),
            constraint_sensitive_probe(70.0, &second),
        ]);

        row.update(&ctx);

        assert_eq!(first.get(), 50.0);
        assert_eq!(second.get(), 70.0);
    }

    #[test]
    fn expanded_draws_in_space_left_by_regular_child() {
        let ctx = dummy_build_context(300.0, 100.0, None);
        let regular = Rc::new(Cell::new(0.0));
        let expanded = Rc::new(Cell::new(0.0));
        let row = row_of(vec![
            constraint_sensitive_probe(50.0, &regular),
            expanded_probe(1.0, &expanded),
        ]);

        row.update(&ctx);

        assert_eq!(regular.get(), 50.0);
        assert_eq!(expanded.get(), 250.0);
    }

    #[test]
    fn regular_column_children_draw_at_intrinsic_height() {
        let ctx = dummy_build_context(100.0, 300.0, None);
        let first = Rc::new(Cell::new(0.0));
        let second = Rc::new(Cell::new(0.0));
        let column = column_of(vec![
            constraint_sensitive_column_probe(50.0, &first),
            constraint_sensitive_column_probe(70.0, &second),
        ]);

        column.update(&ctx);

        assert_eq!(first.get(), 50.0);
        assert_eq!(second.get(), 70.0);
    }

    #[test]
    fn positioned_uses_measured_child_size_when_intrinsic_height_is_zero() {
        let ctx = dummy_build_context(1000.0, 500.0, None);
        let observed_parent_size = Rc::new(Cell::new(ResolvedSize::default()));
        let positioned = RawPositionedElement::new(MeasuredPositionedChild {
            observed_parent_size: observed_parent_size.clone(),
        })
        .left(12)
        .top(16);

        positioned.update(&ctx);

        assert_eq!(
            observed_parent_size.get(),
            ResolvedSize {
                width: 320.0,
                height: 96.0
            },
            "positioned children must receive their measured size, not a zero intrinsic height"
        );
    }

    #[cfg(feature = "portable-guest")]
    mod semantic_graph {
        use aimer_anteros::{
            PROPERTY_CONTAINER_BOX_DECORATION, PROPERTY_CONTAINER_COLOR, PROPERTY_CONTAINER_HEIGHT,
            PROPERTY_CONTAINER_MARGIN, PROPERTY_CONTAINER_PADDING, PROPERTY_CONTAINER_WIDTH,
            PROPERTY_TEXT_ALIGN, PROPERTY_TEXT_CONTENT, PROPERTY_TEXT_STYLE, PropertyPresence,
            PropertyValue, PropertyValueKind, Version, WIDGET_CONTAINER, WIDGET_TEXT,
            WidgetDocumentView, WidgetProperty,
        };
        use aimer_color::prelude::Color;
        use aimer_style::{BoxDecoration, LayoutSpacing, Spacing};
        use aimer_text::Text;
        use aimer_widget::portable::{
            __anteros::stable_schema_hash64, PortableNativeWidget, PortableWidgetSchema,
        };
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };
        use aimer_widget::{PortableWidget, RequiredChild, Widget};

        use crate::Container;

        fn context() -> PortableBuildContext {
            PortableBuildContext::new(
                7,
                11,
                PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048).with_max_blob_bytes(4_096),
                PortableLimits::new(8, 16, 64, 128, 1_024),
            )
            .unwrap()
        }

        #[test]
        fn container_and_text_form_the_documented_semantic_graph() {
            let mut context = context();
            let root = Container::new()
                .width(320.0)
                .height(180.0)
                .color(Color::HexA(0x112233FF))
                .child(Text::new("Hello"))
                .to_portable_node(
                    &mut context,
                    SourceFingerprint::new(StableId128::from_path("test", "container-text")),
                )
                .unwrap();

            let graph = context.finish_graph(root).unwrap();

            assert_eq!(graph.root(), root);
            assert_eq!(graph.node_count(), 2);
            assert_eq!(graph.string_count(), 1);
            assert_eq!(graph.string(0), Some("Hello"));
            let container = graph.node(root).unwrap();
            assert_eq!(container.widget_type(), WIDGET_CONTAINER);
            assert_eq!(container.widget_schema(), Version::new(1, 0));
            assert_eq!(
                container.properties(),
                &[
                    WidgetProperty::new(PROPERTY_CONTAINER_WIDTH, PropertyValue::BlobRef(0))
                        .optional(),
                    WidgetProperty::new(PROPERTY_CONTAINER_COLOR, PropertyValue::Rgba(0x112233FF))
                        .optional(),
                    WidgetProperty::new(PROPERTY_CONTAINER_HEIGHT, PropertyValue::BlobRef(1))
                        .optional(),
                ],
            );
            assert_eq!(graph.blob(0), Some(&[1, 1, 0, 0, 160, 67][..]));
            assert_eq!(graph.blob(1), Some(&[1, 1, 0, 0, 52, 67][..]));
            let child = container.children().next().unwrap();
            let text = graph.node(child).unwrap();
            assert_eq!(text.widget_type(), WIDGET_TEXT);
            assert_eq!(
                text.properties(),
                &[WidgetProperty::new(
                    PROPERTY_TEXT_CONTENT,
                    PropertyValue::StringRef(0),
                )],
            );
            assert_eq!(text.children().len(), 0);

            let document = graph.compile();
            let encoded = document.encode().unwrap();
            let view = aimer_anteros::WidgetDocumentView::decode(&encoded, document.model_limits())
                .unwrap();
            assert_eq!(view.root_node(), root.index());
            assert_eq!(view.string(0), Some("Hello"));
        }

        #[test]
        fn builtins_expose_their_complete_derived_portable_schemas() {
            let container = <Container<RequiredChild> as PortableWidgetSchema>::SCHEMA;
            assert_eq!(container.widget().id(), WIDGET_CONTAINER);
            assert_eq!(container.children().minimum(), 1);
            assert_eq!(container.children().maximum(), 1);
            assert_eq!(
                container
                    .properties()
                    .iter()
                    .map(|property| property.canonical_name())
                    .collect::<Vec<_>>(),
                vec![
                    "aimer.property:aimer_container::single_child::Container:width",
                    "aimer.property:aimer_container::single_child::Container:height",
                    "aimer.property:aimer_container::single_child::Container:padding",
                    "aimer.property:aimer_container::single_child::Container:margin",
                    "aimer.property:aimer_container::single_child::Container:box_decoration",
                    "aimer.property:aimer_container::single_child::Container:color",
                ],
            );
            assert!(
                container
                    .properties()
                    .iter()
                    .all(|property| { property.presence() == PropertyPresence::Optional })
            );
            assert_eq!(
                container.properties()[0].value_kind(),
                PropertyValueKind::BlobRef
            );
            assert_eq!(
                container.properties()[2].value_kind(),
                PropertyValueKind::BlobRef
            );
            assert_eq!(
                container.properties()[4].value_kind(),
                PropertyValueKind::BlobRef
            );
            assert_eq!(
                container.properties()[0].id().value(),
                stable_schema_hash64(
                    "aimer.property:aimer_container::single_child::Container:width"
                ),
            );

            let text = <Text as PortableWidgetSchema>::SCHEMA;
            assert_eq!(text.widget().id(), WIDGET_TEXT);
            assert_eq!(text.children().maximum(), 0);
            assert_eq!(text.properties().len(), 3);
            assert_eq!(
                text.properties()[0].canonical_name(),
                PROPERTY_TEXT_CONTENT_NAME
            );
            assert_eq!(
                text.properties()[0].value_kind(),
                PropertyValueKind::StringRef
            );
            assert_eq!(text.properties()[0].presence(), PropertyPresence::Required);
            assert_eq!(text.properties()[1].id(), PROPERTY_TEXT_ALIGN);
            assert_eq!(text.properties()[1].value_kind(), PropertyValueKind::I64);
            assert_eq!(text.properties()[1].presence(), PropertyPresence::Optional);
            assert_eq!(text.properties()[2].id(), PROPERTY_TEXT_STYLE);
            assert_eq!(
                text.properties()[2].value_kind(),
                PropertyValueKind::BlobRef
            );
            assert_eq!(text.properties()[2].presence(), PropertyPresence::Optional);
        }

        #[test]
        fn container_derived_lowering_accepts_spacing_and_full_decoration() {
            let mut context = context();
            let root = Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(20)))
                .margin(LayoutSpacing::all(Spacing::Px(4)))
                .box_decoration(BoxDecoration::new().background_color(Color::GREEN))
                .child(Text::new("Hello"))
                .to_portable_node(
                    &mut context,
                    SourceFingerprint::new(StableId128::from_path("test", "styled-container")),
                )
                .unwrap();
            let graph = context.finish_graph(root).unwrap();
            let container = graph.node(root).unwrap();

            assert_eq!(graph.blob_count(), 3);
            assert!(container.properties().iter().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_PADDING
                    && matches!(property.value(), PropertyValue::BlobRef(_))
                    && property.is_optional()
            }));
            assert!(container.properties().iter().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_MARGIN
                    && matches!(property.value(), PropertyValue::BlobRef(_))
                    && property.is_optional()
            }));
            assert!(container.properties().iter().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_BOX_DECORATION
                    && matches!(property.value(), PropertyValue::BlobRef(_))
                    && property.is_optional()
            }));

            let document = graph.compile();
            let encoded = document.encode().unwrap();
            let view = WidgetDocumentView::decode(&encoded, document.model_limits()).unwrap();
            let child = view.node(root.index()).unwrap().children().next().unwrap();
            let _container =
                <Container<RequiredChild> as PortableNativeWidget>::materialize_widget(
                    &view,
                    view.node(root.index()).unwrap(),
                    vec![Text::new("native child").boxed()],
                )
                .unwrap();
            let _text = <Text as PortableNativeWidget>::materialize_widget(
                &view,
                view.node(child).unwrap(),
                Vec::new(),
            )
            .unwrap();
        }

        const PROPERTY_TEXT_CONTENT_NAME: &str = "aimer.property:aimer_text::Text:text";
    }
}
