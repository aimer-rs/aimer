//! Lab-only layout, event, state, and counter simulation built on Cupid's v2 render core.

use std::cell::{Cell, RefCell};

use aimer_rubick::Shared;

pub use aimer_canvas::Canvas;
pub use aimer_cupid::draw_cmd_v2::{
    CurrentBuildContext, DrawCommand, DrawCommandList, DrawList, DrawListSnapshot, Rect,
    RenderFrame, RenderItem, RenderNodeId, RenderOp, RenderTree, RenderTreeError,
};

/// Errors from the lab-only layout and event simulation layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum V2RenderError {
    /// A core render-tree operation failed.
    RenderTree(RenderTreeError),
    /// The supplied child is not a direct child of the layout node.
    InvalidLayoutChild {
        parent: RenderNodeId,
        child: RenderNodeId,
    },
    /// The number of child sizes did not match the multi-child layout.
    WrongLayoutChildCount { expected: usize, actual: usize },
    /// A stateful paint node is not a child of the supplied multi-child layout.
    ElementNotInLayout { element: RenderNodeId },
    /// A stateful paint node was paired with a layout node for another child.
    WrongLayoutTarget {
        element: RenderNodeId,
        layout_child: RenderNodeId,
    },
}

impl From<RenderTreeError> for V2RenderError {
    fn from(error: RenderTreeError) -> Self {
        Self::RenderTree(error)
    }
}

#[inline]
fn rect_contains(rect: Rect, x: f32, y: f32) -> bool {
    rect_is_valid(rect)
        && x >= rect.x
        && y >= rect.y
        && x < rect.x + rect.width
        && y < rect.y + rect.height
}

#[inline]
fn rect_is_valid(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

/// Layout owner for one render-tree child.
///
/// This lab layout node owns geometry only. It has no draw list; applying a
/// layout result updates the child's local bounds and damages its old and new
/// world-space areas through [`RenderTree::set_bounds`].
pub struct LayoutElement {
    tree: RenderTree,
    parent: RenderNodeId,
    child: RenderNodeId,
}

impl LayoutElement {
    /// Creates a layout node for a direct child of `parent`.
    pub fn new(
        tree: RenderTree,
        parent: RenderNodeId,
        child: RenderNodeId,
    ) -> Result<Self, V2RenderError> {
        let _ = tree.parent_of(parent)?;
        if tree.parent_of(child)? != Some(parent) {
            return Err(V2RenderError::InvalidLayoutChild { parent, child });
        }
        Ok(Self {
            tree,
            parent,
            child,
        })
    }

    /// Returns the layout owner's render-tree parent.
    #[inline]
    pub const fn parent_id(&self) -> RenderNodeId {
        self.parent
    }

    /// Returns the child whose local bounds this layout node controls.
    #[inline]
    pub const fn child_id(&self) -> RenderNodeId {
        self.child
    }

    /// Applies the child's resolved local layout bounds.
    pub fn layout_child(&self, bounds: Rect) -> Result<(), V2RenderError> {
        Ok(self.tree.set_bounds(self.child, bounds)?)
    }

    /// Returns the child's resolved world-space bounds after layout.
    pub fn child_bounds(&self) -> Result<Rect, V2RenderError> {
        Ok(self.tree.element_bounds(self.child)?)
    }
}

/// Horizontal layout owner for multiple render-tree children.
///
/// The layout object stores child identity and placement rules only. Each
/// child keeps its own retained draw list, and layout changes damage the moved
/// children through [`RenderTree::set_bounds`].
pub struct MultiChildLayoutElement {
    tree: RenderTree,
    parent: RenderNodeId,
    children: Vec<RenderNodeId>,
    origin: (f32, f32),
    gap: f32,
}

impl MultiChildLayoutElement {
    /// Creates a horizontal layout for direct children of `parent`.
    pub fn new(
        tree: RenderTree,
        parent: RenderNodeId,
        children: impl IntoIterator<Item = RenderNodeId>,
        origin: (f32, f32),
        gap: f32,
    ) -> Result<Self, V2RenderError> {
        if !rect_is_valid(Rect::new(origin.0, origin.1, 0.0, 0.0)) || !gap.is_finite() || gap < 0.0
        {
            return Err(V2RenderError::RenderTree(RenderTreeError::InvalidBounds));
        }
        let children = children.into_iter().collect::<Vec<_>>();
        let _ = tree.parent_of(parent)?;
        for child in children.iter().copied() {
            if tree.parent_of(child)? != Some(parent) {
                return Err(V2RenderError::InvalidLayoutChild { parent, child });
            }
        }
        Ok(Self {
            tree,
            parent,
            children,
            origin,
            gap,
        })
    }

    /// Returns the layout owner's render-tree parent.
    #[inline]
    pub const fn parent_id(&self) -> RenderNodeId {
        self.parent
    }

    /// Returns child IDs in horizontal layout order.
    #[inline]
    pub fn child_ids(&self) -> &[RenderNodeId] {
        &self.children
    }

    /// Places every child using its `(width, height)` in local coordinates.
    pub fn layout_row(&self, child_sizes: &[(f32, f32)]) -> Result<Vec<Rect>, V2RenderError> {
        if child_sizes.len() != self.children.len() {
            return Err(V2RenderError::WrongLayoutChildCount {
                expected: self.children.len(),
                actual: child_sizes.len(),
            });
        }

        let mut x = self.origin.0;
        let mut bounds = Vec::with_capacity(child_sizes.len());
        for (width, height) in child_sizes.iter().copied() {
            let child_bounds = Rect::new(x, self.origin.1, width, height);
            if !rect_is_valid(child_bounds) {
                return Err(V2RenderError::RenderTree(RenderTreeError::InvalidBounds));
            }
            bounds.push(child_bounds);
            x += width + self.gap;
        }
        for (child, child_bounds) in self.children.iter().copied().zip(bounds.iter().copied()) {
            self.tree.set_bounds(child, child_bounds)?;
        }
        Ok(bounds)
    }

    /// Returns each child's latest world-space bounds in layout order.
    pub fn child_bounds(&self) -> Result<Vec<Rect>, V2RenderError> {
        self.children
            .iter()
            .copied()
            .map(|child| self.tree.element_bounds(child).map_err(Into::into))
            .collect()
    }
}

/// A pointer-down event in world-space logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerDown {
    /// Pointer position when the event was dispatched.
    pub position: (f32, f32),
}

/// An event-only target whose hit bounds follow a child's current layout box.
///
/// This target does not create a render node or draw list. Layout remains owned
/// by the child, while this wrapper uses the child's resolved world bounds for
/// hit testing and invokes its listener for matching pointer events.
pub struct ChildSizedEventElement {
    tree: RenderTree,
    child: RenderNodeId,
    on_pointer_down: Box<dyn FnMut(PointerDown)>,
}

impl ChildSizedEventElement {
    /// Creates an event-only wrapper around an already laid out child.
    pub fn new(
        tree: RenderTree,
        child: RenderNodeId,
        on_pointer_down: impl FnMut(PointerDown) + 'static,
    ) -> Result<Self, V2RenderError> {
        tree.element_bounds(child)?;
        Ok(Self {
            tree,
            child,
            on_pointer_down: Box::new(on_pointer_down),
        })
    }

    /// Returns the child's latest world-space layout bounds.
    pub fn bounds(&self) -> Result<Rect, V2RenderError> {
        Ok(self.tree.element_bounds(self.child)?)
    }

    /// Dispatches to the listener when the pointer falls inside the child box.
    pub fn dispatch_pointer_down(&mut self, position: (f32, f32)) -> Result<bool, V2RenderError> {
        if !rect_contains(self.bounds()?, position.0, position.1) {
            return Ok(false);
        }
        (self.on_pointer_down)(PointerDown { position });
        Ok(true)
    }
}

/// A state owner whose update invalidates and re-records one render node.
pub struct StatefulRenderElement<S: 'static> {
    tree: RenderTree,
    element: RenderNodeId,
    state: RefCell<S>,
    paint: Box<dyn Fn(&S, &mut Canvas)>,
}

impl<S: 'static> StatefulRenderElement<S> {
    /// Creates a stateful paint node and records its initial local commands.
    pub fn new(
        tree: RenderTree,
        element: RenderNodeId,
        state: S,
        paint: impl Fn(&S, &mut Canvas) + 'static,
    ) -> Result<Self, V2RenderError> {
        let element = Self {
            tree,
            element,
            state: RefCell::new(state),
            paint: Box::new(paint),
        };
        element.record()?;
        Ok(element)
    }

    /// Returns this stateful element's stable render identity.
    #[inline]
    pub const fn element_id(&self) -> RenderNodeId {
        self.element
    }

    /// Mutates this element's state and replaces only its local draw list.
    pub fn update(&self, update: impl FnOnce(&mut S)) -> Result<u64, V2RenderError> {
        update(&mut self.state.borrow_mut());
        self.tree.invalidate_paint(self.element)?;
        self.record()
    }

    /// Updates state, lays out this node, then records its new local commands.
    pub fn update_and_layout(
        &self,
        layout: &LayoutElement,
        update: impl FnOnce(&mut S),
        resolve_bounds: impl FnOnce(&S) -> Rect,
    ) -> Result<u64, V2RenderError> {
        if layout.child_id() != self.element {
            return Err(V2RenderError::WrongLayoutTarget {
                element: self.element,
                layout_child: layout.child_id(),
            });
        }
        update(&mut self.state.borrow_mut());
        let bounds = {
            let state = self.state.borrow();
            resolve_bounds(&state)
        };
        layout.layout_child(bounds)?;
        self.tree.invalidate_paint(self.element)?;
        self.record()
    }

    /// Updates state, lays out a row containing this node, then records it.
    pub fn update_and_layout_row(
        &self,
        layout: &MultiChildLayoutElement,
        update: impl FnOnce(&mut S),
        resolve_sizes: impl FnOnce(&S) -> Vec<(f32, f32)>,
    ) -> Result<u64, V2RenderError> {
        if !layout.child_ids().contains(&self.element) {
            return Err(V2RenderError::ElementNotInLayout {
                element: self.element,
            });
        }
        update(&mut self.state.borrow_mut());
        let sizes = {
            let state = self.state.borrow();
            resolve_sizes(&state)
        };
        layout.layout_row(&sizes)?;
        self.tree.invalidate_paint(self.element)?;
        self.record()
    }

    fn record(&self) -> Result<u64, V2RenderError> {
        let context = self.tree.context(self.element)?;
        let mut canvas = Canvas::of(&context);
        (self.paint)(&self.state.borrow(), &mut canvas);
        Ok(canvas.try_finish()?)
    }
}

/// Three nested container layers with a stateful text leaf for update experiments.
pub struct CounterSimulation {
    tree: RenderTree,
    container_1: RenderNodeId,
    container_2: RenderNodeId,
    container_3: RenderNodeId,
    counter_suffix: RenderNodeId,
    counter_row_layout: MultiChildLayoutElement,
    counter_text: StatefulRenderElement<u32>,
    counter_event: ChildSizedEventElement,
    event_hits: Shared<Cell<u32>>,
}

impl CounterSimulation {
    /// Builds the initial C1 → C2 → C3 tree and records each element once.
    pub fn new() -> Result<Self, V2RenderError> {
        let tree = RenderTree::new();
        let container_1 = tree.add_root(Rect::new(0.0, 0.0, 1000.0, 800.0))?;
        let container_2 = tree.add_child(container_1, Rect::new(180.0, 120.0, 680.0, 550.0))?;
        let container_3 = tree.add_child(container_2, Rect::new(180.0, 130.0, 430.0, 340.0))?;
        let counter_text = tree.add_child(container_3, Rect::new(12.0, 12.0, 240.0, 48.0))?;
        let counter_suffix = tree.add_child(container_3, Rect::new(0.0, 0.0, 80.0, 48.0))?;
        let counter_row_layout = MultiChildLayoutElement::new(
            tree.clone(),
            container_3,
            [counter_text, counter_suffix],
            (12.0, 12.0),
            8.0,
        )?;
        counter_row_layout.layout_row(&[(counter_text_width(9), 48.0), (64.0, 48.0)])?;

        record_fill(&tree, container_1, 1000.0, 800.0, [100, 66, 32, 255])?;
        record_fill(&tree, container_2, 680.0, 550.0, [27, 146, 126, 255])?;
        record_fill(&tree, container_3, 430.0, 340.0, [89, 10, 210, 255])?;

        let counter_text =
            StatefulRenderElement::new(tree.clone(), counter_text, 9, |count, canvas| {
                canvas.draw_text(
                    format!("Count: {count}"),
                    [0.0, 32.0],
                    28.0,
                    [255, 255, 255, 255],
                );
            })?;
        record_text(&tree, counter_suffix, " items", [255, 255, 255, 255])?;
        let event_hits = Shared::new(Cell::new(0));
        let event_hits_listener = event_hits.clone();
        let counter_event =
            ChildSizedEventElement::new(tree.clone(), counter_text.element_id(), move |_| {
                event_hits_listener.set(event_hits_listener.get() + 1)
            })?;

        // Treat construction as the initial frame so the next update measures
        // only the stateful leaf's damage.
        let _ = tree.take_damage();

        Ok(Self {
            tree,
            container_1,
            container_2,
            container_3,
            counter_suffix,
            counter_row_layout,
            counter_text,
            counter_event,
            event_hits,
        })
    }

    /// Builds the first complete paint plan after construction.
    pub fn initial_frame(&self) -> Result<RenderFrame, V2RenderError> {
        Ok(RenderFrame {
            damage: vec![self.tree.element_bounds(self.container_1)?],
            operations: self.tree.render_all(),
        })
    }

    /// Increments the count and returns the resulting damage and composition plan.
    pub fn increment(&self) -> Result<RenderFrame, V2RenderError> {
        self.counter_text.update_and_layout_row(
            &self.counter_row_layout,
            |count| *count += 1,
            |count| vec![(counter_text_width(*count), 48.0), (64.0, 48.0)],
        )?;
        Ok(self.tree.render_pending())
    }

    /// Dispatches a pointer event to the child-sized event wrapper.
    ///
    /// A hit increments the counter and returns its damage-aware render frame;
    /// a miss returns `None` without changing state or paint commands.
    pub fn dispatch_pointer_down(
        &mut self,
        position: (f32, f32),
    ) -> Result<Option<RenderFrame>, V2RenderError> {
        if !self.counter_event.dispatch_pointer_down(position)? {
            return Ok(None);
        }
        self.counter_text.update_and_layout_row(
            &self.counter_row_layout,
            |count| *count += 1,
            |count| vec![(counter_text_width(*count), 48.0), (64.0, 48.0)],
        )?;
        Ok(Some(self.tree.render_pending()))
    }

    /// Returns how many pointer events the child-sized wrapper handled.
    pub fn handled_pointer_events(&self) -> u32 {
        self.event_hits.get()
    }

    /// Returns the event wrapper's current child-derived hit bounds.
    pub fn event_bounds(&self) -> Result<Rect, V2RenderError> {
        self.counter_event.bounds()
    }

    /// Returns the world-space bounds of both children in the counter row.
    pub fn counter_row_bounds(&self) -> Result<Vec<Rect>, V2RenderError> {
        self.counter_row_layout.child_bounds()
    }

    /// Returns the current count.
    pub fn count(&self) -> u32 {
        *self.counter_text.state.borrow()
    }

    /// Returns the stable identities for the three backgrounds and text leaf.
    pub const fn element_ids(&self) -> [RenderNodeId; 5] {
        [
            self.container_1,
            self.container_2,
            self.container_3,
            self.counter_text.element_id(),
            self.counter_suffix,
        ]
    }

    /// Returns the retained tree for inspecting revisions or building a full frame.
    #[inline]
    pub const fn tree(&self) -> &RenderTree {
        &self.tree
    }
}

fn counter_text_width(count: u32) -> f32 {
    format!("Count: {count}").chars().count() as f32 * 14.0 + 16.0
}

fn record_text(
    tree: &RenderTree,
    element: RenderNodeId,
    text: &str,
    color: [u8; 4],
) -> Result<(), V2RenderError> {
    let context = tree.context(element)?;
    let canvas = Canvas::of(&context);
    canvas.draw_text(text, [0.0, 32.0], 28.0, color);
    canvas.try_finish()?;
    Ok(())
}

fn record_fill(
    tree: &RenderTree,
    element: RenderNodeId,
    width: f32,
    height: f32,
    color: [u8; 4],
) -> Result<(), V2RenderError> {
    let context = tree.context(element)?;
    let canvas = Canvas::of(&context);
    canvas.fill_rect(Rect::new(0.0, 0.0, width, height), color);
    canvas.try_finish()?;
    Ok(())
}
