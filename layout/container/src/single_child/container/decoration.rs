use super::*;

/// A decoration selected by retained native state without replacing its owner.
///
/// This is an internal integration for controls such as Button. The returned
/// decoration must remain valid for the borrow, and `revision` must distinguish
/// different current paint values. Changes to border widths must call
/// `aimer_widget::notify_element_tree_changed`; changes to clipping radii or
/// paint outsets must call `aimer_widget::notify_retained_render_structure_changed`.
/// Those notifications refresh cached ancestor layout, hit areas and render geometry.
#[doc(hidden)]
pub trait RetainedBoxDecoration {
    /// Returns the complete currently selected decoration.
    fn decoration(&self) -> &BoxDecoration;

    /// Returns the revision identifying the selected decoration.
    fn revision(&self) -> u64;
}

// Keep ordinary containers in the same pooled allocation class. Only a
// container with a retained decoration source needs these revision cells.
pub(super) struct RetainedDecorationState {
    source: Rc<dyn RetainedBoxDecoration>,
    layout_revision: std::cell::Cell<Option<u64>>,
    paint_revision: std::cell::Cell<Option<u64>>,
}

impl RetainedDecorationState {
    pub(super) fn new(source: Rc<dyn RetainedBoxDecoration>) -> Box<Self> {
        Box::new(Self {
            source,
            layout_revision: std::cell::Cell::new(None),
            paint_revision: std::cell::Cell::new(None),
        })
    }
}

impl<E: Element, D: Decoration> RawContainer<E, D> {
    #[inline]
    pub(super) fn decoration(&self) -> ActiveDecoration<'_, D> {
        match &self.decoration_source {
            Some(state) => ActiveDecoration::Retained(state.source.decoration()),
            None => ActiveDecoration::Direct(&self.box_decoration),
        }
    }

    #[inline]
    pub(super) fn decoration_insets(&self, width: f32, height: f32, scale: f32) -> [f32; 4] {
        self.decoration()
            .content_insets(
                ResolvedSize {
                    width: width / scale,
                    height: height / scale,
                },
                scale,
            )
            .map(|inset| finite_nonnegative(inset) * scale)
    }

    #[inline]
    pub(super) fn mark_decoration_recorded(&self) {
        if let Some(state) = &self.decoration_source {
            state.paint_revision.set(Some(state.source.revision()));
        }
    }

    #[inline]
    pub(super) fn retained_decoration_needs_recording(&self) -> bool {
        self.decoration_source
            .as_ref()
            .is_some_and(|state| state.paint_revision.get() != Some(state.source.revision()))
    }

    #[inline]
    pub(super) fn refresh_decoration_layout(&self) {
        if let Some(state) = &self.decoration_source {
            let revision = Some(state.source.revision());
            if state.layout_revision.replace(revision) != revision {
                self.cache.invalidate();
            }
        }
    }
}

pub(super) enum ActiveDecoration<'a, D> {
    Direct(&'a D),
    Retained(&'a BoxDecoration),
}

impl<D: Decoration> Decoration for ActiveDecoration<'_, D> {
    #[inline]
    fn paint(&self, canvas: &aimer_canvas::Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        match self {
            Self::Direct(decoration) => decoration.paint(canvas, origin, size, scale),
            Self::Retained(decoration) => decoration.paint(canvas, origin, size, scale),
        }
    }

    #[inline]
    fn paint_with_background(
        &self,
        canvas: &aimer_canvas::Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<Color>,
    ) {
        match self {
            Self::Direct(decoration) => {
                decoration.paint_with_background(canvas, origin, size, scale, background)
            }
            Self::Retained(decoration) => {
                decoration.paint_with_background(canvas, origin, size, scale, background)
            }
        }
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        match self {
            Self::Direct(decoration) => decoration.paint_outsets(size, scale),
            Self::Retained(decoration) => decoration.paint_outsets(size, scale),
        }
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        match self {
            Self::Direct(decoration) => decoration.content_insets(size, scale),
            Self::Retained(decoration) => decoration.content_insets(size, scale),
        }
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        match self {
            Self::Direct(decoration) => decoration.child_clip_radius(size, scale),
            Self::Retained(decoration) => decoration.child_clip_radius(size, scale),
        }
    }

    #[inline]
    fn background_color(&self) -> Option<Color> {
        match self {
            Self::Direct(decoration) => decoration.background_color(),
            Self::Retained(decoration) => decoration.background_color(),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            Self::Direct(decoration) => decoration.is_empty(),
            Self::Retained(decoration) => decoration.is_empty(),
        }
    }

    #[inline]
    fn revision(&self) -> u64 {
        match self {
            Self::Direct(decoration) => decoration.revision(),
            Self::Retained(decoration) => decoration.revision(),
        }
    }
}
