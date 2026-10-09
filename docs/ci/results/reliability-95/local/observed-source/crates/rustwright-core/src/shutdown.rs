//! One cleanup worker, shared completion, and no task ownership in caller futures.
use crate::error::{Error, Result};
use rustwright_cdp::CdpError;
use std::{future::Future, sync::Mutex, time::Duration};
use tokio::sync::watch;

pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

// Error itself contains non-cloneable I/O errors. Preserve typed shutdown
// protocol failures and deadlines for every waiter rather than reporting Ok on
// subsequent close calls. Other failures retain their diagnostic text.
#[derive(Clone)]
enum Failure {
    Protocol {
        method: String,
        code: i64,
        message: String,
        data: Option<String>,
    },
    CdpTimeout {
        method: String,
        timeout: Duration,
    },
    Timeout {
        what: String,
        timeout: Duration,
    },
    Closed,
    Other(String),
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Cdp(CdpError::Protocol {
                method,
                code,
                message,
                data,
            }) => Self::Protocol {
                method,
                code,
                message,
                data,
            },
            Error::Cdp(CdpError::Timeout { method, timeout }) => {
                Self::CdpTimeout { method, timeout }
            }
            Error::Cdp(CdpError::Closed) => Self::Closed,
            Error::Timeout { what, timeout } => Self::Timeout { what, timeout },
            other => Self::Other(other.to_string()),
        }
    }
}
impl Failure {
    fn error(self) -> Error {
        match self {
            Self::Protocol {
                method,
                code,
                message,
                data,
            } => CdpError::Protocol {
                method,
                code,
                message,
                data,
            }
            .into(),
            Self::CdpTimeout { method, timeout } => CdpError::Timeout { method, timeout }.into(),
            Self::Timeout { what, timeout } => Error::Timeout { what, timeout },
            Self::Closed => CdpError::Closed.into(),
            Self::Other(message) => Error::Io(std::io::Error::other(message)),
        }
    }
}
type Outcome = std::result::Result<(), Failure>;
#[derive(Default)]
pub(crate) struct Shutdown {
    completion: Mutex<Option<watch::Receiver<Option<Outcome>>>>,
}
struct Completion(watch::Sender<Option<Outcome>>);
impl Drop for Completion {
    fn drop(&mut self) {
        if self.0.borrow().is_none() {
            self.0.send_replace(Some(Err(Failure::Other(
                "shutdown worker stopped before completing cleanup".into(),
            ))));
        }
    }
}
impl Shutdown {
    pub(crate) async fn run<F, Fut>(&self, start: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let mut receiver = {
            let mut current = self.completion.lock().expect("shutdown mutex poisoned");
            if let Some(receiver) = &*current {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                let work = start();
                tokio::spawn(async move {
                    let completion = Completion(sender);
                    let result = work.await.map_err(Failure::from);
                    completion.0.send_replace(Some(result));
                });
                *current = Some(receiver.clone());
                receiver
            }
        };
        loop {
            let outcome = receiver.borrow().clone();
            if let Some(result) = outcome {
                return result.map_err(Failure::error);
            }
            receiver.changed().await.map_err(|_| {
                Error::Io(std::io::Error::other("shutdown completion channel stopped"))
            })?;
        }
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::sync::Notify;

    #[tokio::test]
    async fn cancelled_waiter_does_not_cancel_single_worker_or_late_result() {
        let shutdown = Arc::new(Shutdown::default());
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let first = {
            let shutdown = shutdown.clone();
            let entered = entered.clone();
            let release = release.clone();
            let starts = starts.clone();
            tokio::spawn(async move {
                shutdown
                    .run(move || async move {
                        starts.fetch_add(1, Ordering::SeqCst);
                        entered.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                    .await
            })
        };
        entered.notified().await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let mut waiters = Vec::new();
        for _ in 0..16 {
            let shutdown = shutdown.clone();
            waiters.push(tokio::spawn(async move {
                shutdown
                    .run(|| async { panic!("cleanup must not restart") })
                    .await
            }));
        }
        release.notify_one();
        for waiter in waiters {
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        shutdown
            .run(|| async { panic!("late waiter must not restart cleanup") })
            .await
            .unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn worker_panic_wakes_all_waiters_with_failure() {
        let shutdown = Shutdown::default();
        for attempt in 0..2 {
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                shutdown.run(|| async { panic!("deliberate cleanup panic") }),
            )
            .await
            .unwrap();
            assert!(
                matches!(result, Err(Error::Io(ref error)) if error.to_string().contains("shutdown worker stopped")),
                "attempt {attempt}: {result:?}"
            );
        }
    }
}
