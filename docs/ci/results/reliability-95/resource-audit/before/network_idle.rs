//! Live, owned HTTP activity, independent of retained network diagnostics.
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use tokio::{
    sync::{broadcast, watch},
    task::JoinHandle,
    time::Instant,
};

use crate::{connection::BidiEvent, BidiError, BidiResult, BidiSession, BrowsingContextInfo};

const QUIET_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
enum Fault {
    Incomplete,
    Lost(u64),
    Closed,
}
struct Request {
    hop: u64,
    explicit_navigation: bool,
    context: String,
    navigation: Option<String>,
}
struct Activity {
    root: String,
    parents: HashMap<String, Option<String>>,
    navigations: HashMap<String, String>,
    committed: HashMap<String, String>,
    requests: HashMap<String, Request>,
    quiet_since: Option<Instant>,
    fault: Option<Fault>,
}
impl Activity {
    fn new(root: String) -> Self {
        Self {
            root,
            parents: HashMap::new(),
            navigations: HashMap::new(),
            committed: HashMap::new(),
            requests: HashMap::new(),
            quiet_since: None,
            fault: Some(Fault::Incomplete),
        }
    }
    fn bootstrap(&mut self, tree: &[BrowsingContextInfo], now: Instant) -> BidiResult<()> {
        fn visit(
            parents: &mut HashMap<String, Option<String>>,
            info: &BrowsingContextInfo,
            parent: Option<&str>,
        ) {
            parents.insert(
                info.context.clone(),
                info.parent.clone().or_else(|| parent.map(str::to_owned)),
            );
            for child in info.children.iter().flatten() {
                visit(parents, child, Some(&info.context));
            }
        }
        if let Some(info) = tree.iter().find(|info| info.context == self.root) {
            visit(&mut self.parents, info, None);
        }
        if !self.parents.contains_key(&self.root) {
            return Err(BidiError::Closed);
        }
        if matches!(self.fault, Some(Fault::Closed)) {
            return Err(BidiError::Closed);
        }
        self.fault = None;
        self.quiet_since = Some(now);
        Ok(())
    }
    fn owns(&self, context: &str) -> bool {
        self.parents.contains_key(context)
    }
    fn event(&mut self, event: &BidiEvent, now: Instant) {
        if self.fault.is_some() {
            return;
        }
        let p = &event.params;
        let Some(context) = p.get("context").and_then(Value::as_str) else {
            return;
        };
        match event.method.as_str() {
            "browsingContext.contextCreated" => {
                if let Some(parent) = p
                    .get("parent")
                    .and_then(Value::as_str)
                    .filter(|parent| self.owns(parent))
                {
                    self.parents
                        .insert(context.to_owned(), Some(parent.to_owned()));
                }
            }
            "browsingContext.contextDestroyed" if self.owns(context) => {
                if context == self.root {
                    self.close();
                    return;
                }
                let mut removed = HashSet::from([context.to_owned()]);
                loop {
                    let before = removed.len();
                    for (id, parent) in &self.parents {
                        if parent.as_ref().is_some_and(|id| removed.contains(id)) {
                            removed.insert(id.clone());
                        }
                    }
                    if before == removed.len() {
                        break;
                    }
                }
                self.parents.retain(|id, _| !removed.contains(id));
                self.navigations.retain(|id, _| !removed.contains(id));
                self.committed.retain(|id, _| !removed.contains(id));
                self.remove_requests(|r| removed.contains(&r.context), now);
            }
            "browsingContext.navigationStarted" if self.owns(context) => {
                if let Some(id) = p.get("navigation").and_then(Value::as_str) {
                    self.navigations.insert(context.to_owned(), id.to_owned());
                }
            }
            "browsingContext.domContentLoaded" | "browsingContext.load" if self.owns(context) => {
                if let Some(id) = p.get("navigation").and_then(Value::as_str) {
                    // A previous document can finish after its replacement has
                    // started. Its late commit cannot retire the incoming work.
                    if self
                        .navigations
                        .get(context)
                        .is_some_and(|current| current != id)
                    {
                        return;
                    }
                    if self
                        .committed
                        .get(context)
                        .is_some_and(|committed| committed == id)
                    {
                        return;
                    }
                    self.committed.insert(context.to_owned(), id.to_owned());
                    // Only a committed document retires the previous document's
                    // work. Explicit navigation requests can precede their
                    // navigationStarted event, so only their own terminal event
                    // or context destruction can retire them.
                    self.remove_requests(
                        |r| {
                            r.context == context
                                && !r.explicit_navigation
                                && r.navigation.as_deref() != Some(id)
                        },
                        now,
                    );
                    self.navigations.insert(context.to_owned(), id.to_owned());
                    if self.requests.is_empty() {
                        self.quiet_since = Some(now);
                    }
                }
            }
            "network.beforeRequestSent" if self.owns(context) => {
                let r = &p["request"];
                let Some(id) = r
                    .get("request")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    return;
                };
                let Some(url) = r.get("url").and_then(Value::as_str) else {
                    return;
                };
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return;
                }
                let hop = p.get("redirectCount").and_then(Value::as_u64).unwrap_or(0);
                if self.requests.get(id).is_some_and(|r| r.hop > hop) {
                    return;
                }
                // A before event replayed at completion has a populated terminal
                // timing. It is history, not a new live request. This does not
                // reconstruct activity before the acknowledged subscription.
                if r["timings"]["responseEnd"]
                    .as_f64()
                    .is_some_and(|end| end > 0.0)
                {
                    return;
                }
                let explicit_navigation = p.get("navigation").and_then(Value::as_str);
                let navigation = explicit_navigation
                    .map(str::to_owned)
                    .or_else(|| self.navigations.get(context).cloned());
                self.requests.insert(
                    id.to_owned(),
                    Request {
                        hop,
                        explicit_navigation: explicit_navigation.is_some(),
                        context: context.to_owned(),
                        navigation,
                    },
                );
                self.quiet_since = None;
            }
            "network.responseCompleted" | "network.fetchError" if self.owns(context) => {
                let Some(id) = p["request"]["request"].as_str() else {
                    return;
                };
                let hop = p.get("redirectCount").and_then(Value::as_u64).unwrap_or(0);
                if self
                    .requests
                    .get(id)
                    .is_some_and(|r| r.hop == hop && r.context == context)
                {
                    self.requests.remove(id);
                    if self.requests.is_empty() {
                        self.quiet_since = Some(now);
                    }
                }
            }
            _ => {}
        }
    }
    fn remove_requests(&mut self, removed: impl Fn(&Request) -> bool, now: Instant) {
        let before = self.requests.len();
        self.requests.retain(|_, r| !removed(r));
        if before != self.requests.len() && self.requests.is_empty() {
            self.quiet_since = Some(now);
        }
    }
    fn close(&mut self) {
        self.fault = Some(Fault::Closed);
        self.requests.clear();
        self.parents.clear();
        self.navigations.clear();
        self.committed.clear();
        self.quiet_since = None;
    }
    fn deadline(&self) -> BidiResult<Option<Instant>> {
        match self.fault {
            Some(Fault::Incomplete) => Err(BidiError::NetworkObservationIncomplete),
            Some(Fault::Lost(skipped)) => Err(BidiError::NetworkEventsLost { skipped }),
            Some(Fault::Closed) => Err(BidiError::Closed),
            None => Ok(self.quiet_since.map(|since| since + QUIET_WINDOW)),
        }
    }
}

/// A session's weak registry shares this observer without owning its page.
pub(crate) struct IdleObserver {
    state: Arc<Mutex<Activity>>,
    changes: watch::Sender<()>,
    setup: tokio::sync::Mutex<bool>,
    pump: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for IdleObserver {
    fn drop(&mut self) {
        if let Some(pump) = self
            .pump
            .get_mut()
            .expect("idle pump mutex poisoned")
            .take()
        {
            pump.abort();
        }
    }
}
impl IdleObserver {
    pub(crate) fn new(root: String) -> Self {
        let (changes, _) = watch::channel(());
        Self {
            state: Arc::new(Mutex::new(Activity::new(root))),
            changes,
            setup: tokio::sync::Mutex::new(false),
            pump: Mutex::new(None),
        }
    }
    pub(crate) fn mark_closed(&self) {
        self.state
            .lock()
            .expect("idle state mutex poisoned")
            .close();
        self.changes.send_replace(());
    }
    pub(crate) async fn initialize(self: &Arc<Self>, session: BidiSession) -> BidiResult<()> {
        let observer = self.clone();
        tokio::spawn(async move {
            tokio::time::timeout(
                crate::session::NETWORK_SETUP_TIMEOUT,
                observer.setup_owned(session),
            )
            .await
            .map_err(|_| BidiError::Timeout {
                method: "network idle setup".to_owned(),
                timeout: crate::session::NETWORK_SETUP_TIMEOUT,
            })?
        })
        .await
        .map_err(|e| BidiError::Unexpected(format!("network idle setup task failed: {e}")))?
    }

    async fn setup_owned(&self, session: BidiSession) -> BidiResult<()> {
        let mut ready = self.setup.lock().await;
        if *ready {
            return self
                .state
                .lock()
                .expect("idle state mutex poisoned")
                .deadline()
                .map(|_| ());
        }
        let mut events = session.events();
        let shutdown = session.connection().shutdown_receiver();
        session.ensure_network_subscription().await?;
        let root = self
            .state
            .lock()
            .expect("idle state mutex poisoned")
            .root
            .clone();
        let tree = session.get_tree_from(&root).await?;
        let mut activity = Activity::new(root);
        activity.bootstrap(&tree, Instant::now())?;
        // Replay the finite buffered prefix preceding snapshot readiness. An
        // overflowing receiver cannot reconstruct the lost activity safely.
        let buffered = events.len();
        for _ in 0..buffered {
            match events.try_recv() {
                Ok(event) => activity.event(&event, Instant::now()),
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    return Err(BidiError::NetworkEventsLost { skipped });
                }
                Err(broadcast::error::TryRecvError::Closed) => return Err(BidiError::Closed),
            }
        }
        activity.deadline()?;
        if *shutdown.borrow() {
            return Err(BidiError::Closed);
        }
        {
            let mut state = self.state.lock().expect("idle state mutex poisoned");
            if matches!(state.fault, Some(Fault::Closed)) {
                return Err(BidiError::Closed);
            }
            *state = activity;
        }
        let pump = spawn_idle_pump(self.state.clone(), self.changes.clone(), events, shutdown);
        *self.pump.lock().expect("idle pump mutex poisoned") = Some(pump);
        *ready = true;
        self.changes.send_replace(());
        Ok(())
    }

    pub(crate) async fn wait(&self, timeout: Duration) -> BidiResult<()> {
        let mut changes = self.changes.subscribe();
        // Firefox can acknowledge a triggering script before delivering its
        // request event. Observe at least one full quiet window per call.
        let floor = Instant::now() + QUIET_WINDOW;
        tokio::time::timeout(timeout, async {
            loop {
                let deadline = self
                    .state
                    .lock()
                    .expect("idle state mutex poisoned")
                    .deadline()?
                    .map(|deadline| deadline.max(floor));
                match deadline {
                    Some(deadline) if deadline <= Instant::now() => return Ok(()),
                    Some(deadline) => {
                        tokio::select! {
                            result = changes.changed() => result.map_err(|_| BidiError::Closed)?,
                            _ = tokio::time::sleep_until(deadline) => {},
                        }
                    }
                    None => changes.changed().await.map_err(|_| BidiError::Closed)?,
                }
            }
        })
        .await
        .map_err(|_| BidiError::Timeout {
            method: "network idle".to_owned(),
            timeout,
        })?
    }
}

fn spawn_idle_pump(
    state: Arc<Mutex<Activity>>,
    changes: watch::Sender<()>,
    mut events: broadcast::Receiver<BidiEvent>,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if *shutdown.borrow() {
                state.lock().expect("idle state mutex poisoned").close();
                changes.send_replace(());
                break;
            }
            tokio::select! {
                result = events.recv() => {
                    let mut state = state.lock().expect("idle state mutex poisoned");
                    match result {
                        Ok(event) => state.event(&event, Instant::now()),
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            state.fault = Some(Fault::Lost(skipped));
                            state.requests.clear();
                        }
                        Err(broadcast::error::RecvError::Closed) => state.close(),
                    }
                    changes.send_replace(());
                    if state.fault.is_some() { break; }
                },
                _ = shutdown.changed() => {
                    state.lock().expect("idle state mutex poisoned").close();
                    changes.send_replace(());
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;
