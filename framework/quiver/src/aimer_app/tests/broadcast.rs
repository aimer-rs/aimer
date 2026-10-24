use super::*;

#[derive(Default)]
struct BroadcastCounts {
    structural_visits: Cell<usize>,
    event_visits: Cell<usize>,
    cancels: Cell<usize>,
}

struct BroadcastWidget {
    counts: Rc<BroadcastCounts>,
}

impl Widget for BroadcastWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        let children = (0..32)
            .map(|_| {
                BroadcastElement {
                    counts: self.counts.clone(),
                    children: Vec::new(),
                }
                .boxed()
            })
            .collect();
        BroadcastElement {
            counts: self.counts,
            children,
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for BroadcastWidget {}

struct BroadcastElement {
    counts: Rc<BroadcastCounts>,
    children: Vec<AnyElement>,
}

impl VisitorElement for BroadcastElement {
    fn debug_name(&self) -> &'static str {
        "BroadcastElement"
    }

    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }
}

impl EventElement for BroadcastElement {
    fn event_tree_role(&self) -> aimer_widget::EventTreeRole {
        aimer_widget::EventTreeRole::IndexedTarget
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            self.counts
                .structural_visits
                .set(self.counts.structural_visits.get() + 1);
            visitor(child.as_ref());
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            self.counts
                .event_visits
                .set(self.counts.event_visits.get() + 1);
            visitor(child.as_ref());
        }
    }

    fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
        if matches!(event, ElementEvent::Cancel) {
            self.counts.cancels.set(self.counts.cancels.get() + 1);
        }
        aimer_widget::EventResult::ignored()
    }
}

impl Drawable for BroadcastElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for BroadcastElement {}
impl Rebuildable for BroadcastElement {}

#[test]
fn window_focus_broadcast_reuses_the_application_event_tree() {
    let counts = Rc::new(BroadcastCounts::default());
    let mut app = AimerApp::start_headless(BroadcastWidget {
        counts: counts.clone(),
    });
    app.render_frame();
    app.send_window_event(WindowEvent::Focused(true));
    counts.structural_visits.set(0);
    counts.event_visits.set(0);
    counts.cancels.set(0);

    app.send_window_event(WindowEvent::Focused(true));
    app.send_window_event(WindowEvent::Focused(true));

    assert_eq!(counts.cancels.get(), 66);
    assert_eq!(counts.structural_visits.get(), 0);
    assert_eq!(counts.event_visits.get(), 0);
}
