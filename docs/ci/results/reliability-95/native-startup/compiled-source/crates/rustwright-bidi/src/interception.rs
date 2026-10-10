//! Lifetime and acknowledgement tracking for page-owned BiDi interception.
use crate::network::{spawn_intercept_pump, InterceptPump, InterceptRegistry};
use crate::{BidiError, BidiResult, BidiSession};
use rustwright_common::{Route, RouteAction};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::oneshot;
const ADD_TIMEOUT: Duration = Duration::from_secs(30);
const REMOVE_TIMEOUT: Duration = Duration::from_secs(5);
struct Registration {
    id: String,
    response: bool,
    removing: bool,
}
#[derive(Default)]
struct Registrations {
    ids: Vec<Registration>,
    pump: Option<InterceptPump>,
}
pub(crate) struct RoutingCore {
    runtime: tokio::runtime::Handle,
    session: BidiSession,
    context: String,
    owners: AtomicUsize,
    closed: AtomicBool,
    operations: tokio::sync::Mutex<Registrations>,
    routes: Arc<Mutex<Vec<Route>>>,
    // Unknown IDs are buffered during allocation; existing IDs keep working.
    event_ids: Arc<InterceptRegistry>,
}
// Registration becomes a committed rule when the public waiter acknowledges
// this handoff. Dropping a queued, unobserved result requests rollback.
struct Handoff(Option<oneshot::Sender<bool>>);
impl Handoff {
    fn accept(mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(true);
        }
    }
}
impl Drop for Handoff {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(false);
        }
    }
}
pub(crate) struct Routing {
    core: Arc<RoutingCore>,
}
impl Routing {
    pub(crate) fn new(core: Arc<RoutingCore>) -> Self {
        core.owners.fetch_add(1, Ordering::AcqRel);
        Self { core }
    }
    pub(crate) async fn route(&self, pattern: String, action: RouteAction) -> BidiResult<()> {
        let (tx, rx) = oneshot::channel();
        let (finished, completion) = oneshot::channel();
        let core = self.core.clone();
        self.core.runtime.spawn(async move {
            let mut tx = Some(tx);
            let result = core.add(pattern, action, &mut tx).await;
            if let Some(tx) = tx {
                let _ = tx.send(Err(result.err().unwrap_or(BidiError::Closed)));
            } else {
                let _ = finished.send(result);
            }
        });
        let handoff = rx
            .await
            .map_err(|_| BidiError::Unexpected("interception worker stopped".into()))??;
        handoff.accept();
        completion
            .await
            .map_err(|_| BidiError::Unexpected("interception worker stopped".into()))?
    }

    #[cfg(test)]
    pub(crate) fn weak_core(&self) -> std::sync::Weak<RoutingCore> {
        Arc::downgrade(&self.core)
    }
    #[cfg(test)]
    pub(crate) async fn event_id_count(&self) -> usize {
        self.core
            .event_ids
            .ids
            .lock()
            .expect("bidi intercept IDs mutex poisoned")
            .len()
    }
    pub(crate) async fn clear(&self) -> BidiResult<()> {
        self.core.clone().clear_protected(false).await
    }
}
impl Drop for Routing {
    fn drop(&mut self) {
        if self.core.owners.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let core = self.core.clone();
        self.core.runtime.spawn(async move {
            let mut delay = Duration::from_millis(100);
            loop {
                if core.owners.load(Ordering::Acquire) != 0 {
                    break;
                }
                if core.clear_if_abandoned().await.is_ok() || core.session.connection().is_closed()
                {
                    break;
                }
                // Failed retirement keeps a continue-only pump and known IDs reachable via
                // the session's weak index. A new owner can retry without duplicating them.
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        });
    }
}
impl RoutingCore {
    pub(crate) fn new(session: BidiSession, context: String) -> Self {
        Self {
            runtime: tokio::runtime::Handle::current(),
            session,
            context,
            owners: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            operations: tokio::sync::Mutex::new(Registrations::default()),
            routes: Arc::new(Mutex::new(Vec::new())),
            event_ids: Arc::new(InterceptRegistry::default()),
        }
    }
    async fn add(
        &self,
        pattern: String,
        action: RouteAction,
        tx: &mut Option<oneshot::Sender<BidiResult<Handoff>>>,
    ) -> BidiResult<()> {
        if self.closed.load(Ordering::Acquire) || self.session.connection().is_closed() {
            return Err(BidiError::Closed);
        }
        let mut state = self.operations.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(BidiError::Closed);
        }
        if tx.as_ref().is_none_or(|tx| tx.is_closed()) {
            return Ok(());
        }
        // A failed or timed-out removal may already have succeeded remotely.
        // Confirm the same IDs absent before activating any replacement rule.
        let retiring: Vec<_> = state
            .ids
            .iter()
            .filter(|id| id.removing)
            .map(|id| id.id.clone())
            .collect();
        for id in retiring {
            self.remove(&mut state, &id).await?;
        }
        let prior_has_routes = !self
            .routes
            .lock()
            .expect("bidi routes mutex poisoned")
            .is_empty();
        let prior_response = self
            .routes
            .lock()
            .expect("bidi routes mutex poisoned")
            .iter()
            .any(|route| matches!(route.action, RouteAction::SetResponseHeaders(_)));
        let needs_response = matches!(action, RouteAction::SetResponseHeaders(_)) || prior_response;
        let existing_response = state.ids.iter().any(|id| id.response);
        let mut added = None;
        if state.ids.is_empty() || (needs_response && !existing_response) {
            self.session.ensure_network_subscription().await?;
            if tx.as_ref().is_none_or(|tx| tx.is_closed()) && !prior_has_routes {
                return Ok(());
            }
            if state.pump.is_none() {
                state.pump = Some(spawn_intercept_pump(
                    self.session.clone(),
                    self.routes.clone(),
                    self.event_ids.clone(),
                ));
            }
            self.event_ids.begin_allocation();
            let phases = if needs_response || existing_response {
                vec!["beforeRequestSent", "responseStarted"]
            } else {
                vec!["beforeRequestSent"]
            };
            let response = needs_response || existing_response;
            let result = tokio::time::timeout(
                ADD_TIMEOUT,
                self.session.add_intercept(&self.context, &phases),
            )
            .await;
            let id = match result {
                Ok(Ok(id)) => id,
                Ok(Err(error)) => {
                    self.event_ids.finish_allocation();
                    if state.ids.is_empty() {
                        self.stop(&mut state).await;
                    }
                    return Err(error);
                }
                Err(_) => {
                    self.event_ids.finish_allocation();
                    if state.ids.is_empty() {
                        self.stop(&mut state).await;
                    }
                    return Err(BidiError::Timeout {
                        method: "network.addIntercept".into(),
                        timeout: ADD_TIMEOUT,
                    });
                }
            };
            self.event_ids
                .ids
                .lock()
                .expect("bidi intercept IDs mutex poisoned")
                .insert(id.clone());
            self.event_ids.finish_allocation();
            state.ids.push(Registration {
                id: id.clone(),
                response,
                removing: false,
            });
            added = Some(id);
        }
        // No public handle observed the registration: discard this candidate while
        // preserving the previous committed rules and their owned registration.
        if tx.as_ref().is_none_or(|tx| tx.is_closed()) {
            if let Some(id) = added {
                self.rollback(&mut state, &id).await?;
            }
            if state.ids.is_empty() {
                self.stop(&mut state).await;
            }
            return Ok(());
        }
        // Broader registration is installed before old phases are retired. The pump
        // consumes each event once even if it lists both owned IDs.
        if state.ids.len() > 1 {
            let keep = state.ids.last().unwrap().id.clone();
            let old: Vec<_> = state
                .ids
                .iter()
                .filter(|id| id.id != keep)
                .map(|id| id.id.clone())
                .collect();
            for id in old {
                if let Err(error) = self.remove(&mut state, &id).await {
                    if let Some(id) = added.as_ref() {
                        let _ = self.remove(&mut state, id).await;
                    }
                    return Err(error);
                }
            }
        }
        if self.closed.load(Ordering::Acquire) || tx.as_ref().is_none_or(|tx| tx.is_closed()) {
            if let Some(id) = added {
                self.rollback(&mut state, &id).await?;
            }
            if state.ids.is_empty() {
                self.stop(&mut state).await;
            }
            return if self.closed.load(Ordering::Acquire) {
                Err(BidiError::Closed)
            } else {
                Ok(())
            };
        }
        let (accept, accepted) = oneshot::channel();
        let transferred = tx.take().unwrap().send(Ok(Handoff(Some(accept)))).is_ok();
        if !transferred || !accepted.await.unwrap_or(false) {
            if let Some(id) = added.as_ref() {
                self.rollback(&mut state, id).await?;
            }
            if state.ids.is_empty() {
                self.stop(&mut state).await;
            }
            return Ok(());
        }
        self.routes
            .lock()
            .expect("bidi routes mutex poisoned")
            .push(Route::new(pattern, action));
        Ok(())
    }
    async fn rollback(&self, state: &mut Registrations, id: &str) -> BidiResult<()> {
        // If the older phases were already acknowledged absent, keep the new
        // registration for prior committed rules. The candidate is never added.
        if state.ids.iter().any(|owned| owned.id != id)
            || self
                .routes
                .lock()
                .expect("bidi routes mutex poisoned")
                .is_empty()
        {
            self.remove(state, id).await?;
        }
        Ok(())
    }
    async fn remove(&self, state: &mut Registrations, id: &str) -> BidiResult<()> {
        if let Some(registration) = state.ids.iter_mut().find(|owned| owned.id == id) {
            registration.removing = true;
        }
        match tokio::time::timeout(REMOVE_TIMEOUT, self.session.remove_intercept(id)).await {
            Ok(Ok(())) => {}
            Ok(Err(BidiError::Protocol { error, .. })) if error == "no such intercept" => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(BidiError::Timeout {
                    method: "network.removeIntercept".into(),
                    timeout: REMOVE_TIMEOUT,
                })
            }
        }
        state.ids.retain(|registered| registered.id != id);
        // Drain events queued before the acknowledgement, then prune this ID.
        // Repeated upgrades must not accumulate every historical registration.
        if let Some(pump) = state.pump.as_ref() {
            pump.retire(id).await;
        }
        Ok(())
    }
    async fn stop(&self, state: &mut Registrations) {
        if let Some(pump) = state.pump.take() {
            pump.stop().await;
        }
        self.event_ids
            .ids
            .lock()
            .expect("bidi intercept IDs mutex poisoned")
            .clear();
    }
    async fn clear_locked(&self, state: &mut Registrations) -> BidiResult<()> {
        self.routes
            .lock()
            .expect("bidi routes mutex poisoned")
            .clear();
        let ids: Vec<_> = state.ids.iter().map(|id| id.id.clone()).collect();
        let mut first = None;
        for id in ids {
            if let Err(error) = self.remove(state, &id).await {
                if first.is_none() {
                    first = Some(error)
                }
            }
        }
        if state.ids.is_empty() || self.session.connection().is_closed() {
            self.stop(state).await;
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    async fn clear_if_abandoned(&self) -> BidiResult<()> {
        let mut state = self.operations.lock().await;
        if self.owners.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        self.clear_locked(&mut state).await
    }
    pub(crate) fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }
    pub(crate) async fn clear_protected(self: Arc<Self>, close: bool) -> BidiResult<()> {
        if close {
            self.mark_closed();
        }
        let runtime = self.runtime.clone();
        runtime
            .spawn(async move {
                let mut state = self.operations.lock().await;
                self.clear_locked(&mut state).await
            })
            .await
            .map_err(|error| {
                BidiError::Unexpected(format!("interception cleanup worker failed: {error}"))
            })?
    }
}
