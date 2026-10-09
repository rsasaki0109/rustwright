//! Live HTTP activity for page-wide network-idle waits.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::time::Instant;

const QUIET_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone, Hash, PartialEq, Eq)]
struct RequestKey {
    session: String,
    request: String,
}

pub(crate) struct RequestOwner {
    pub(crate) frame: Option<String>,
    pub(crate) loader: Option<String>,
    pub(crate) document: bool,
}

pub(crate) struct NetworkActivity {
    requests: HashMap<RequestKey, RequestOwner>,
    quiet_since: Option<Instant>,
}

impl Default for NetworkActivity {
    fn default() -> Self {
        Self {
            requests: HashMap::new(),
            quiet_since: Some(Instant::now()),
        }
    }
}

impl NetworkActivity {
    pub(crate) fn start(&mut self, session: &str, request: &str, owner: RequestOwner) {
        // Redirects reuse the request id. Replacing its owner cannot add an
        // extra in-flight request that would never receive a terminal event.
        self.requests.insert(
            RequestKey {
                session: session.to_string(),
                request: request.to_string(),
            },
            owner,
        );
        self.quiet_since = None;
    }

    fn key(&self, session: &str, request: &str) -> Option<RequestKey> {
        let key = RequestKey {
            session: session.to_string(),
            request: request.to_string(),
        };
        if self.requests.contains_key(&key) {
            return Some(key);
        }
        // A document can begin in the parent session and finish in its new
        // renderer. Subresources remain strictly scoped to their own session.
        let mut documents = self
            .requests
            .iter()
            .filter(|(key, owner)| key.request == request && owner.document);
        let (key, _) = documents.next()?;
        documents.next().is_none().then(|| key.clone())
    }

    pub(crate) fn transfer_document(&mut self, session: &str, request: &str) {
        if let Some(key) = self.key(session, request) {
            if key.session != session {
                let owner = self.requests.remove(&key).expect("active document request");
                self.requests.insert(
                    RequestKey {
                        session: session.to_string(),
                        request: request.to_string(),
                    },
                    owner,
                );
            }
        }
    }

    pub(crate) fn finish(&mut self, session: &str, request: &str, now: Instant) {
        if let Some(key) = self.key(session, request) {
            self.requests.remove(&key);
            if self.requests.is_empty() {
                self.quiet_since = Some(now);
            }
        }
    }

    pub(crate) fn commit_frame(&mut self, frame: &str, loader: &str, main: bool, now: Instant) {
        self.requests
            .retain(|_, owner| match owner.frame.as_deref() {
                Some(id) if id == frame => owner.loader.as_deref().is_none_or(|id| id == loader),
                Some(_) if main => false, // Requests from the old document's children.
                _ => true,
            });
        // A new document starts its own quiet window, including BFCache restores.
        self.quiet_since = self.requests.is_empty().then_some(now);
    }

    pub(crate) fn remove_sessions(&mut self, sessions: &HashSet<String>, now: Instant) {
        self.remove_matching(|key, _| sessions.contains(&key.session), now);
    }

    pub(crate) fn remove_frames(&mut self, frames: &HashSet<String>, now: Instant) {
        self.remove_matching(
            |_, owner| owner.frame.as_ref().is_some_and(|id| frames.contains(id)),
            now,
        );
    }

    fn remove_matching(
        &mut self,
        mut removed: impl FnMut(&RequestKey, &RequestOwner) -> bool,
        now: Instant,
    ) {
        let before = self.requests.len();
        self.requests.retain(|key, owner| !removed(key, owner));
        if before != self.requests.len() && self.requests.is_empty() {
            self.quiet_since = Some(now);
        }
    }

    pub(crate) fn idle_deadline(&self) -> Option<Instant> {
        self.quiet_since.map(|since| since + QUIET_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::{NetworkActivity, RequestOwner, QUIET_WINDOW};
    use std::{collections::HashSet, time::Duration};
    use tokio::time::Instant;

    fn owner(frame: &str, loader: &str, document: bool) -> RequestOwner {
        RequestOwner {
            frame: Some(frame.to_string()),
            loader: Some(loader.to_string()),
            document,
        }
    }

    #[test]
    fn colliding_ids_and_duplicate_terminal_events_preserve_the_last_request_window() {
        let mut activity = NetworkActivity::default();
        let now = Instant::now();
        activity.start("parent", "same", owner("main", "page", false));
        activity.start("child", "same", owner("child", "iframe", false));
        activity.finish("unrelated", "same", now);
        assert!(activity.idle_deadline().is_none());
        activity.finish("parent", "same", now + Duration::from_millis(100));
        assert!(activity.idle_deadline().is_none());
        let finished = now + Duration::from_millis(200);
        activity.finish("child", "same", finished);
        assert_eq!(activity.idle_deadline(), Some(finished + QUIET_WINDOW));
        activity.finish("child", "same", now + Duration::from_secs(1));
        assert_eq!(activity.idle_deadline(), Some(finished + QUIET_WINDOW));
        activity.start("parent", "later", owner("main", "page", false));
        assert!(activity.idle_deadline().is_none());
    }

    #[test]
    fn redirects_and_document_transfers_do_not_create_phantom_requests() {
        let mut activity = NetworkActivity::default();
        let now = Instant::now();
        activity.start("parent", "document", owner("iframe", "new", true));
        activity.start("parent", "document", owner("iframe", "new", true));
        activity.transfer_document("child", "document");
        activity.remove_sessions(&HashSet::from(["parent".to_string()]), now);
        assert!(activity.idle_deadline().is_none());
        activity.finish("child", "document", now);
        assert_eq!(activity.idle_deadline(), Some(now + QUIET_WINDOW));

        // A completion alone can also transfer a parent-started document.
        activity.start("parent", "next-document", owner("iframe", "next", true));
        activity.finish("next-child", "next-document", now);
        assert_eq!(activity.idle_deadline(), Some(now + QUIET_WINDOW));
    }

    #[test]
    fn new_main_document_discards_old_requests_but_preserves_its_pending_navigation() {
        let mut activity = NetworkActivity::default();
        let now = Instant::now();
        activity.start("parent", "old-keepalive", owner("main", "old", false));
        activity.start("child", "old-fetch", owner("child", "iframe", false));
        activity.start("parent", "new-document", owner("main", "new", true));
        activity.commit_frame("main", "new", true, now);
        assert!(activity.idle_deadline().is_none());
        activity.finish("parent", "new-document", now);
        assert_eq!(activity.idle_deadline(), Some(now + QUIET_WINDOW));
        activity.finish("parent", "old-keepalive", now + Duration::from_secs(1));
        assert_eq!(activity.idle_deadline(), Some(now + QUIET_WINDOW));
    }

    #[test]
    fn removing_frames_and_sessions_preserves_other_pending_frames() {
        let mut activity = NetworkActivity::default();
        let now = Instant::now();
        activity.start("parent", "local", owner("local-frame", "local", false));
        activity.start("child", "remote", owner("remote-frame", "remote", false));
        activity.start("sibling", "other", owner("other-frame", "other", false));
        activity.remove_frames(&HashSet::from(["local-frame".to_string()]), now);
        assert!(activity.idle_deadline().is_none());
        activity.remove_sessions(&HashSet::from(["child".to_string()]), now);
        assert!(activity.idle_deadline().is_none());
        activity.finish("sibling", "other", now);
        assert_eq!(activity.idle_deadline(), Some(now + QUIET_WINDOW));
    }
}
