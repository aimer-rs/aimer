//! Links the platform hands the app, from whichever thread it does so.
//!
//! The registry that opens links ([`aimer_events::link`]) belongs to the UI
//! thread. The platform does not: Android's `onCreate` and `onNewIntent` run on
//! the JVM's thread, and the launch link can arrive before the event loop
//! exists to be woken. So links are posted here, which any thread may do at any
//! time, and the event loop drains them on its own thread: at start, and
//! whenever a post wakes it.

use std::collections::VecDeque;
use std::sync::Mutex;

/// A bounded, thread-safe queue of link URLs.
#[derive(Default)]
pub struct Inbox {
    links: Mutex<VecDeque<String>>,
}

impl Inbox {
    /// More than a person can tap before the loop wakes; the oldest are dropped.
    pub const CAPACITY: usize = 16;

    pub const fn new() -> Self {
        Self { links: Mutex::new(VecDeque::new()) }
    }

    /// Queues `url`, returning whether the event loop needs waking: only the
    /// first link after a drain does, because a wake-up already on its way will
    /// drain the rest. An empty URL is not a link and is ignored.
    pub fn post(&self, url: String) -> bool {
        if url.is_empty() {
            return false;
        }
        // A poisoned lock only means another thread panicked holding it; the
        // queue itself is still a valid queue.
        let mut links = self.links.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_empty = links.is_empty();
        if links.len() == Self::CAPACITY {
            links.pop_front();
        }
        links.push_back(url);
        was_empty
    }

    /// Takes every queued link, oldest first.
    pub fn drain(&self) -> Vec<String> {
        let mut links = self.links.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        links.drain(..).collect()
    }
}

static INBOX: Inbox = Inbox::new();

/// Hands a link to the app from any thread, waking the event loop if it exists.
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub fn post_link(url: String) {
    if INBOX.post(url)
        && let Some(proxy) = crate::event::EVENT_PROXY.get()
    {
        let _ = proxy.send_event(crate::event::AimerNativePlatformEvent::OpenLinks);
    }
}

/// Delivers every posted link to the registered handlers. Called on the UI
/// thread, where the handlers live.
pub fn deliver_posted_links() {
    for url in INBOX.drain() {
        aimer_events::link::deliver_link(&url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_come_out_in_the_order_they_went_in_and_only_once() {
        let inbox = Inbox::default();
        inbox.post("app://one".into());
        inbox.post("app://two".into());
        assert_eq!(inbox.drain(), ["app://one", "app://two"]);
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn posting_says_whether_the_loop_needs_waking() {
        let inbox = Inbox::default();
        assert!(inbox.post("app://a".into()), "the first link after a drain wakes the loop");
        assert!(!inbox.post("app://b".into()), "one already pending: the wake-up is on its way");
        inbox.drain();
        assert!(inbox.post("app://c".into()));
    }

    #[test]
    fn a_flood_keeps_the_newest_links_and_never_grows_without_bound() {
        let inbox = Inbox::default();
        for n in 0..(Inbox::CAPACITY + 10) {
            inbox.post(format!("app://{n}"));
        }
        let links = inbox.drain();
        assert_eq!(links.len(), Inbox::CAPACITY);
        assert_eq!(links.last().map(String::as_str), Some(format!("app://{}", Inbox::CAPACITY + 9).as_str()));
    }

    #[test]
    fn an_empty_link_is_not_posted() {
        let inbox = Inbox::default();
        assert!(!inbox.post(String::new()));
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn posting_from_another_thread_reaches_the_drain() {
        let inbox = std::sync::Arc::new(Inbox::default());
        let poster = inbox.clone();
        std::thread::spawn(move || poster.post("app://from-the-jvm".into())).join().unwrap();
        assert_eq!(inbox.drain(), ["app://from-the-jvm"]);
    }
}
