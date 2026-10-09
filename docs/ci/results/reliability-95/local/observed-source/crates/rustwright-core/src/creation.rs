//! Keep allocation acknowledgments and clean up resources not handed to callers.
use crate::error::{Error, Result};
use rustwright_cdp::CdpConnection;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::oneshot;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Creation {
    connection: CdpConnection,
    id: Option<String>,
    method: &'static str,
    key: &'static str,
    runtime: tokio::runtime::Handle,
}
impl Creation {
    pub(crate) fn new(
        connection: CdpConnection,
        id: String,
        method: &'static str,
        key: &'static str,
    ) -> Self {
        Self {
            connection,
            id: Some(id),
            method,
            key,
            runtime: tokio::runtime::Handle::current(),
        }
    }
    pub(crate) fn id(&self) -> &str {
        self.id
            .as_deref()
            .expect("creation guard owns its identifier")
    }
    pub(crate) fn take(mut self) -> String {
        self.id.take().expect("creation guard owns its identifier")
    }
}
impl Drop for Creation {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        {
            let connection = self.connection.clone();
            let method = self.method;
            let key = self.key;
            self.runtime.spawn(async move {
                let _ = connection
                    .send_with_timeout(None, method, json!({key: id}), Some(CLEANUP_TIMEOUT))
                    .await;
            });
        }
    }
}

// The worker must receive the remote identifier even after its caller drops.
// A guard inside the channel also handles cancellation after send succeeds but
// before the caller receives the result. No unguarded identifier crosses awaits.
pub(crate) async fn allocate(
    connection: CdpConnection,
    method: &'static str,
    params: Value,
    result_key: &'static str,
    cleanup_method: &'static str,
    cleanup_key: &'static str,
) -> Result<Creation> {
    let (mut tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = async {
            let value = {
                let command = connection.send_raw(None, method, params);
                tokio::pin!(command);
                tokio::select! {
                    value = &mut command => value,
                    _ = tx.closed() => {
                        // An abandoned caller gets a bounded chance to recover
                        // the identifier. A silent live peer must not retain the
                        // worker/connection and response waiter forever.
                        tokio::time::timeout(CLEANUP_TIMEOUT, &mut command).await
                            .map_err(|_| rustwright_cdp::CdpError::Timeout { method: method.to_owned(), timeout: CLEANUP_TIMEOUT })?
                    },
                }
            }?;
            let id = serde_json::from_value::<String>(value[result_key].clone())?;
            Ok(Creation::new(connection, id, cleanup_method, cleanup_key))
        }
        .await;
        // A rejected send drops the guard and starts cleanup immediately.
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| Error::Io(std::io::Error::other("resource allocation worker stopped")))?
}
