/// Maximum number of Unicode scalar values accepted in one announcement.
pub const MAX_ANNOUNCEMENT_CHARS: usize = 512;

/// The kind of status being announced to an assistive technology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnouncementKind {
    /// A validation error that needs the user's attention.
    ValidationError,
    /// A loading state or loading-state change.
    Loading,
    /// An important non-error status update.
    Status,
}

/// The urgency requested from an announcement adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnouncementPriority {
    /// The adapter may announce when its queue is ready.
    Polite,
    /// The adapter should interrupt less urgent announcements.
    Assertive,
}

/// A validated, bounded announcement request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Announcement {
    kind: AnnouncementKind,
    priority: AnnouncementPriority,
    text: String,
}

impl Announcement {
    /// Creates an announcement with a default priority based on its kind.
    pub fn try_new(
        kind: AnnouncementKind,
        text: impl Into<String>,
    ) -> Result<Self, AnnouncementError> {
        let priority = match kind {
            AnnouncementKind::ValidationError => AnnouncementPriority::Assertive,
            AnnouncementKind::Loading | AnnouncementKind::Status => AnnouncementPriority::Polite,
        };
        Self::try_with_priority(kind, priority, text)
    }

    /// Creates an announcement with an explicit urgency.
    pub fn try_with_priority(
        kind: AnnouncementKind,
        priority: AnnouncementPriority,
        text: impl Into<String>,
    ) -> Result<Self, AnnouncementError> {
        let text = text.into();
        let character_count = text.chars().count();
        if text.trim().is_empty() {
            return Err(AnnouncementError::Empty);
        }
        if character_count > MAX_ANNOUNCEMENT_CHARS {
            return Err(AnnouncementError::TooLong {
                actual: character_count,
                maximum: MAX_ANNOUNCEMENT_CHARS,
            });
        }
        if text
            .chars()
            .any(|character| character == '\0' || character.is_control() && character != '\n')
        {
            return Err(AnnouncementError::ContainsControlCharacter);
        }
        Ok(Self {
            kind,
            priority,
            text,
        })
    }

    /// Returns the announcement category.
    pub const fn kind(&self) -> AnnouncementKind {
        self.kind
    }

    /// Returns the adapter urgency.
    pub const fn priority(&self) -> AnnouncementPriority {
        self.priority
    }

    /// Returns the validated message.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Delivers this request to an explicitly supplied adapter.
    pub fn deliver_to<P: AnnouncementPort>(&self, port: &mut P) {
        port.announce(self);
    }
}

/// A bounded announcement validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnouncementError {
    /// The message was empty or only whitespace.
    Empty,
    /// The message exceeded the per-request character bound.
    TooLong {
        /// Number of Unicode scalar values in the rejected message.
        actual: usize,
        /// Maximum accepted number of Unicode scalar values.
        maximum: usize,
    },
    /// The message included a control character that should not reach a
    /// platform accessibility channel.
    ContainsControlCharacter,
}

/// Receives announcements on a platform or test adapter.
pub trait AnnouncementPort {
    /// Handles one already validated announcement.
    fn announce(&mut self, announcement: &Announcement);
}

/// A deterministic adapter for hosts without an installed announcement port.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopAnnouncementPort;

impl AnnouncementPort for NoopAnnouncementPort {
    fn announce(&mut self, _announcement: &Announcement) {}
}
