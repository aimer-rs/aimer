// The name the pointer rests on: found by the layout when the pointer has moved, and again each frame while it waits its delay,
// told once it has waited it, and taken back when something ends it.

use crate::input_field::hover::HoverSpot;

impl RawTextField {
    /// Said of every event before the field acts on it: what moves the pointer asks for the name to be found again, what takes
    /// the pointer away or does something else takes back the name that was told, and what is not a move keeps it from being told
    /// again until the pointer moves.
    fn note_hover_event(&self, event: &ElementEvent) {
        if self.hover.is_none() {
            return;
        }
        match event {
            ElementEvent::PointerMove(info) if info.source == PointerSource::Mouse => {
                self.hover_blocked.set(false);
                self.pending_hover.set(true);
                aimer_events::window::request_animation_frame();
            }
            ElementEvent::PointerExited(_, _) => self.hide_hover(false),
            ElementEvent::PointerDown(_)
            | ElementEvent::Scroll { .. }
            | ElementEvent::CharInput { .. }
            | ElementEvent::TextInput { .. } => self.hide_hover(true),
            // The name is taken back, but the app is not told to take away what it shows: a press on that, a row of a list over the text,
            // is what took the focus from the field, and the list is not to go before the press is done.
            ElementEvent::FocusLost => self.hide_name(true),
            // A key takes back the name, but it is not yet known whether it is the app's, which takes it for a list that is open
            // and keeps it: the app is told when the key is left to the field.
            ElementEvent::KeyInput { action, .. } if *action != KeyAction::Released => self.hide_name(true),
            _ => {}
        }
    }

    /// Takes back the name that was told, if one was, and forgets the one that was being waited for. `block` keeps any from being
    /// told until the pointer moves.
    fn hide_hover(&self, block: bool) {
        self.hide_name(block);
        // What the app shows for the name goes at once, and not after the moment the pointer is given to get to it.
        if block {
            self.dismiss_hover();
        }
    }

    /// Takes back the name that was told, and tells the app so, but not that what it shows is to go.
    fn hide_name(&self, block: bool) {
        self.hover_candidate.replace(None);
        if block {
            self.hover_blocked.set(true);
        }
        if self.hover_shown.replace(None).is_some()
            && let Some(hover) = &self.hover
        {
            (hover.on_change)(None);
        }
    }

    /// Tells the app that what it shows for a name is to go at once.
    pub(super) fn dismiss_hover(&self) {
        if let Some(dismiss) = self.hover.as_ref().and_then(|hover| hover.on_dismiss.as_ref()) {
            dismiss();
        }
    }

    /// The bytes of the name that holds `byte`, as the app says what a name is, or as a run of letters when it does not.
    fn hover_name_around(&self, byte: usize) -> Option<Range<usize>> {
        match self.hover.as_ref().and_then(|hover| hover.range_at.as_ref()) {
            Some(range_at) => range_at(byte).filter(|name| name.start <= byte && byte < name.end),
            None => self.link_around(byte),
        }
    }

    /// Finds the name under the pointer and tells the app of it once the pointer has been on it for the delay. Called by the layout
    /// of a frame, where the position of a byte is known, and asked for each frame while a name waits.
    pub(super) fn resolve_hover(
        &self,
        geometry: &Rc<EditableGeometry>,
        scale: f32,
        (abs_x, abs_y): (f32, f32),
        (pad_left, pad_top): (f32, f32),
        base_y: f32,
        is_multiline: bool,
    ) {
        let Some(hover) = &self.hover else {
            return;
        };
        // What was told is about the text as it was.
        let revision = self.controller.revision();
        if self.hover_shown.borrow().as_ref().is_some_and(|(_, told_of)| *told_of != revision) {
            self.hide_hover(false);
        }
        if !self.pending_hover.take() && self.hover_candidate.borrow().is_none() {
            return;
        }
        let pointer = self.hover_pos.get().filter(|_| !self.hover_blocked.get() && self.mouse_held.get().is_none());
        let found = pointer.and_then(|pos: Vec2d| {
            let x = pos.x * scale - abs_x - pad_left + self.scroll_x_offset();
            let y = pos.y * scale - abs_y - pad_top - base_y + if is_multiline { self.scroll_y.get() } else { 0.0 };
            let grapheme = geometry.hit_test(x, y)?;
            let byte = self.controller.value().byte_of_grapheme(grapheme);
            self.name_at(geometry, (x, y), byte, &|at| self.hover_name_around(at))
        });
        let Some(name) = found else {
            // The pointer is not on a name: what was told is taken back, and nothing is waited for.
            self.hover_candidate.replace(None);
            if self.hover_shown.borrow().is_some() {
                self.hide_hover(false);
            }
            return;
        };
        if self.hover_shown.borrow().as_ref().is_some_and(|(shown, _)| *shown == name) {
            self.hover_candidate.replace(None);
            return;
        }
        // Another name than the one that was told: that one goes, and this one is waited for.
        if self.hover_shown.borrow().is_some() {
            self.hide_hover(false);
        }
        let now = self.now();
        let since = match self.hover_candidate.borrow().as_ref() {
            Some((waiting, since)) if *waiting == name => *since,
            _ => now,
        };
        if now.duration_since(since) < hover.delay {
            self.hover_candidate.replace(Some((name, since)));
            aimer_events::window::request_animation_frame();
            return;
        }
        let Some((left, top, right, bottom)) = self.range_box(geometry, name.start, name.end) else {
            self.hover_candidate.replace(None);
            return;
        };
        self.hover_candidate.replace(None);
        self.hover_shown.replace(Some((name.clone(), revision)));
        let scroll_y = if is_multiline { self.scroll_y.get() } else { 0.0 };
        (hover.on_change)(Some(HoverSpot {
            range: name,
            x: (left + pad_left - self.scroll_x_offset()) / scale,
            y: (top + pad_top + base_y - scroll_y) / scale,
            width: (right - left) / scale,
            height: (bottom - top) / scale,
            // The box of the field, which is known whether or not it has the focus: the caret says where its text ends only once it is in it.
            extent: self.cached_bounds.get_bounds().map(|bounds| (bounds.width, bounds.height)).or_else(|| self.caret_context.visible_extent()),
        }));
    }
}
