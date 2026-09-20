//! A handle to a single flat CDP session (a target).

use serde_json::Value;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::connection::{CdpConnection, CdpEvent};
use crate::error::CdpResult;

/// A logical CDP session attached to one target.
///
/// Chrome's *flat* protocol mode multiplexes many sessions over one WebSocket;
/// a `CdpSession` pins the `sessionId` returned by `Target.attachToTarget` so
/// callers can issue target-scoped commands without threading the id manually.
#[derive(Clone)]
pub struct CdpSession {
    connection: CdpConnection,
    session_id: String,
    target_id: String,
}

impl std::fmt::Debug for CdpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpSession")
            .field("session_id", &self.session_id)
            .field("target_id", &self.target_id)
            .finish()
    }
}

impl CdpSession {
    /// Create a session handle from an attached target.
    pub fn new(connection: CdpConnection, session_id: String, target_id: String) -> Self {
        Self {
            connection,
            session_id,
            target_id,
        }
    }

    /// The flat session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The target (page) id this session is attached to.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    /// The underlying browser connection.
    pub fn connection(&self) -> &CdpConnection {
        &self.connection
    }

    /// Send a command scoped to this session.
    pub async fn send(&self, method: &str, params: Value) -> CdpResult<Value> {
        self.connection
            .send_raw(Some(&self.session_id), method, params)
            .await
            .map_err(|error| enrich_method(error, method))
    }

    /// Send a command scoped to this session with an explicit timeout.
    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> CdpResult<Value> {
        self.connection
            .send_with_timeout(Some(&self.session_id), method, params, Some(timeout))
            .await
            .map_err(|error| enrich_method(error, method))
    }

    /// Subscribe to the shared event bus. Callers must filter by
    /// [`Self::session_id`] where appropriate.
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.connection.subscribe()
    }
}

fn enrich_method(error: crate::error::CdpError, method: &str) -> crate::error::CdpError {
    match error {
        crate::error::CdpError::Protocol {
            method: existing,
            code,
            message,
            data,
        } if existing.is_empty() => crate::error::CdpError::Protocol {
            method: method.to_string(),
            code,
            message,
            data,
        },
        other => other,
    }
}
