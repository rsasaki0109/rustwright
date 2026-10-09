//! Typed helpers over the WebDriver BiDi protocol.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const NETWORK_SETUP_TIMEOUT: Duration = Duration::from_secs(5);

use crate::browser::PageHelper;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::connection::{BidiConnection, BidiEvent};
use crate::error::{BidiError, BidiResult};

/// Information about a browsing context (tab or frame).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsingContextInfo {
    /// The context id.
    pub context: String,
    /// The current URL.
    #[serde(default)]
    pub url: String,
    /// The parent context, for frames.
    #[serde(default)]
    pub parent: Option<String>,
    /// The user context (isolated profile) this context belongs to.
    #[serde(default)]
    pub user_context: Option<String>,
    /// Child contexts, for frames.
    #[serde(default)]
    pub children: Option<Vec<BrowsingContextInfo>>,
}

/// A cookie reported by BiDi `storage.getCookies`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BidiCookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie domain.
    #[serde(default)]
    pub domain: String,
    /// Cookie path.
    #[serde(default)]
    pub path: String,
    /// Whether the cookie is HTTP-only.
    #[serde(default)]
    pub http_only: bool,
    /// Whether the cookie is secure.
    #[serde(default)]
    pub secure: bool,
    /// SameSite policy.
    #[serde(default)]
    pub same_site: Option<String>,
    /// Expiry as seconds since the epoch.
    #[serde(default)]
    pub expiry: Option<i64>,
}

/// A serialized `localStorage` item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BidiStorageItem {
    /// Item name.
    pub name: String,
    /// Item value.
    pub value: String,
}

/// `localStorage` for a single origin.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BidiOriginStorage {
    /// The origin, e.g. `https://example.com`.
    pub origin: String,
    /// The stored items.
    #[serde(default)]
    pub local_storage: Vec<BidiStorageItem>,
}

/// A portable snapshot of cookies and local storage (BiDi flavour).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BidiStorageState {
    /// Cookies visible to the page.
    #[serde(default)]
    pub cookies: Vec<BidiCookie>,
    /// Per-origin local storage.
    #[serde(default)]
    pub origins: Vec<BidiOriginStorage>,
}

type CloseLocks = Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>;

struct HelperEntry {
    user_context: String,
    helper: Weak<PageHelper>,
}

/// An established BiDi session.
#[derive(Clone)]
pub struct BidiSession {
    connection: BidiConnection,
    session_id: String,
    capabilities: Value,
    network_subscribed: Arc<tokio::sync::Mutex<bool>>,
    helpers: Arc<Mutex<HashMap<String, HelperEntry>>>,
    registered_helpers: Arc<AtomicUsize>,
    closing: Arc<CloseLocks>,
}

impl std::fmt::Debug for BidiSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiSession")
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl BidiSession {
    /// Connect to a BiDi endpoint and create a session.
    pub async fn connect(ws_url: &str) -> BidiResult<Self> {
        let connection = BidiConnection::connect(ws_url).await?;
        let result = connection
            .send("session.new", json!({ "capabilities": {} }))
            .await?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);
        Ok(Self {
            connection,
            session_id,
            capabilities,
            network_subscribed: Arc::new(tokio::sync::Mutex::new(false)),
            helpers: Arc::new(Mutex::new(HashMap::new())),
            registered_helpers: Arc::new(AtomicUsize::new(0)),
            closing: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) fn page_helper(&self, context: &str, user_context: Option<&str>) -> Arc<PageHelper> {
        let mut helpers = self.helpers.lock().expect("bidi helpers mutex poisoned");
        helpers.retain(|_, entry| entry.helper.strong_count() > 0);
        if let Some(helper) = helpers
            .get(context)
            .and_then(|entry| entry.helper.upgrade())
        {
            return helper;
        }
        let helper = Arc::new(PageHelper::new(
            self.connection.clone(),
            self.registered_helpers.clone(),
        ));
        helpers.insert(
            context.to_string(),
            HelperEntry {
                user_context: user_context.unwrap_or("default").to_string(),
                helper: Arc::downgrade(&helper),
            },
        );
        helper
    }

    /// Internal helper preloads acknowledged by the browser but not yet removed.
    ///
    /// This session-local diagnostic includes asynchronous last-handle cleanup.
    /// It excludes caller-managed raw init scripts. It is not a browser heap
    /// measurement or enumeration of remote scripts: external removals and session
    /// termination can invalidate the count until local cleanup observes them.
    pub fn helper_preload_count(&self) -> usize {
        self.registered_helpers.load(Ordering::Relaxed)
    }

    /// The underlying connection.
    pub fn connection(&self) -> &BidiConnection {
        &self.connection
    }

    /// The BiDi session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The negotiated capabilities.
    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }

    /// A convenience getter for the browser version, when present.
    pub fn browser_version(&self) -> Option<&str> {
        self.capabilities
            .get("browserVersion")
            .and_then(Value::as_str)
    }

    /// Subscribe to BiDi events.
    pub async fn subscribe(&self, events: &[&str]) -> BidiResult<()> {
        self.connection
            .send("session.subscribe", json!({ "events": events }))
            .await?;
        Ok(())
    }

    /// Query the remote end status.
    pub async fn status(&self) -> BidiResult<Value> {
        self.connection.send("session.status", json!({})).await
    }

    /// Subscribe to raw BiDi events.
    pub fn events(&self) -> tokio::sync::broadcast::Receiver<BidiEvent> {
        self.connection.subscribe()
    }

    /// Subscribe to network events once, for diagnostics.
    ///
    /// Concurrent callers wait for acknowledged readiness. Setup continues after
    /// caller cancellation, bounded to five seconds including lock contention.
    /// A failed setup can be retried; lost acknowledgments leave remote
    /// subscription state uncertain even after the local waiter is released.
    pub async fn ensure_network_subscription(&self) -> BidiResult<()> {
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::timeout(NETWORK_SETUP_TIMEOUT, async {
                let mut ready = session.network_subscribed.lock().await;
                if *ready {
                    return Ok(());
                }
                session
                    .subscribe(&[
                        "network.beforeRequestSent",
                        "network.responseStarted",
                        "network.responseCompleted",
                        "network.fetchError",
                    ])
                    .await?;
                *ready = true;
                Ok(())
            })
            .await
            .map_err(|_| BidiError::Timeout {
                method: "session.subscribe".to_owned(),
                timeout: NETWORK_SETUP_TIMEOUT,
            })?
        })
        .await
        .map_err(|error| {
            BidiError::Unexpected(format!("network subscription task failed: {error}"))
        })?
    }

    /// List browsing contexts (tabs and frames).
    pub async fn get_tree(&self) -> BidiResult<Vec<BrowsingContextInfo>> {
        let result = self
            .connection
            .send("browsingContext.getTree", json!({}))
            .await?;
        parse_contexts(&result)
    }

    /// Get a single context's information.
    pub async fn get_context(&self, context: &str) -> BidiResult<Option<BrowsingContextInfo>> {
        let result = self
            .connection
            .send("browsingContext.getTree", json!({ "root": context }))
            .await?;
        Ok(parse_contexts(&result)?.into_iter().next())
    }

    /// Get the context tree rooted at `root` (its descendants are nested).
    pub async fn get_tree_from(&self, root: &str) -> BidiResult<Vec<BrowsingContextInfo>> {
        let result = self
            .connection
            .send("browsingContext.getTree", json!({ "root": root }))
            .await?;
        parse_contexts(&result)
    }

    /// Create a new tab in the default user context.
    pub async fn create_context(&self) -> BidiResult<String> {
        self.create_context_in(None).await
    }

    /// Create a new tab, optionally in an isolated user context.
    pub async fn create_context_in(&self, user_context: Option<&str>) -> BidiResult<String> {
        let mut params = json!({ "type": "tab" });
        if let Some(user_context) = user_context {
            params["userContext"] = json!(user_context);
        }
        Ok(crate::creation::allocate(
            self.connection.clone(),
            "browsingContext.create",
            params,
            "context",
            "browsingContext.close",
            "context",
        )
        .await?
        .take())
    }

    /// Create an isolated user context (its own cookies, storage and cache).
    pub async fn create_user_context(&self) -> BidiResult<String> {
        Ok(crate::creation::allocate(
            self.connection.clone(),
            "browser.createUserContext",
            json!({}),
            "userContext",
            "browser.removeUserContext",
            "userContext",
        )
        .await?
        .take())
    }

    /// Remove an isolated user context and its internal helper registrations.
    ///
    /// Once polled, cleanup continues after caller cancellation for up to five
    /// seconds. Repeated removal also retries any earlier helper-cleanup failure.
    pub async fn remove_user_context(&self, user_context: &str) -> BidiResult<()> {
        self.close_resource(user_context, true).await
    }

    /// Close a browsing context and remove its internal helper registration.
    ///
    /// Cleanup continues after caller cancellation, with a five-second deadline.
    /// Closing a context already removed by the browser is harmless.
    pub async fn close_context(&self, context: &str) -> BidiResult<()> {
        self.close_resource(context, false).await
    }

    async fn close_resource(&self, id: &str, user: bool) -> BidiResult<()> {
        let method = if user {
            "browser.removeUserContext"
        } else {
            "browsingContext.close"
        };
        let key = if user { "userContext" } else { "context" };
        let id = id.to_owned();
        let session = self.clone();
        // Only active close workers own the lock. Pruning weak entries bounds
        // registry growth without retaining every previously closed context id.
        let lock = {
            let mut closing = self.closing.lock().expect("bidi close mutex poisoned");
            closing.retain(|_, entry| entry.strong_count() > 0);
            let entry = closing.entry(format!("{method}:{id}")).or_default();
            let lock = entry
                .upgrade()
                .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
            *entry = Arc::downgrade(&lock);
            lock
        };
        tokio::spawn(async move {
            tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
                let _close = lock.lock().await;
                match session.connection.send(method, json!({key: id})).await {
                    Ok(_) => {}
                    // Only the appropriate missing-resource error confirms that
                    // retrying a previously acknowledged closure is harmless.
                    Err(BidiError::Protocol { error, .. })
                        if (user && error == "no such user context")
                            || (!user && error == "no such frame") => {}
                    Err(error) => return Err(error),
                }
                let helpers: Vec<_> = {
                    let registered = session.helpers.lock().expect("bidi helpers mutex poisoned");
                    registered
                        .iter()
                        .filter_map(|(context, entry)| {
                            if (user && entry.user_context == id) || (!user && context == &id) {
                                entry
                                    .helper
                                    .upgrade()
                                    .map(|helper| (context.clone(), helper))
                            } else {
                                None
                            }
                        })
                        .collect()
                };
                let mut cleanup_error = None;
                for (context, helper) in helpers {
                    // Failure keeps the weak registry entry and preload id so a
                    // later close can retry even while callers retain page handles.
                    if let Err(error) = helper.remove_preload().await {
                        if cleanup_error.is_none() {
                            cleanup_error = Some(error);
                        }
                    } else {
                        let mut registered =
                            session.helpers.lock().expect("bidi helpers mutex poisoned");
                        if registered
                            .get(&context)
                            .is_some_and(|entry| entry.helper.ptr_eq(&Arc::downgrade(&helper)))
                        {
                            registered.remove(&context);
                        }
                    }
                }
                match cleanup_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            })
            .await
            .map_err(|_| BidiError::Timeout {
                method: method.to_owned(),
                timeout: SHUTDOWN_TIMEOUT,
            })?
        })
        .await
        .map_err(|error| BidiError::Unexpected(format!("context shutdown task failed: {error}")))?
    }

    /// Navigate a context and wait for the load to complete.
    pub async fn navigate(&self, context: &str, url: &str) -> BidiResult<String> {
        let result = self
            .connection
            .send(
                "browsingContext.navigate",
                json!({ "context": context, "url": url, "wait": "complete" }),
            )
            .await?;
        Ok(result
            .get("navigation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    /// Evaluate an expression in a context and return a JSON value.
    pub async fn evaluate(&self, context: &str, expression: &str) -> BidiResult<Value> {
        let result = self
            .connection
            .send(
                "script.evaluate",
                json!({
                    "expression": expression,
                    "target": { "context": context },
                    "awaitPromise": true,
                    "resultOwnership": "none",
                }),
            )
            .await?;
        evaluation_result(&result)
    }

    /// Register a preload script that runs in every new document.
    ///
    /// `function_declaration` must be a function (for example
    /// `function () { ... }`). Returns the script id.
    pub async fn add_preload_script(
        &self,
        function_declaration: &str,
        contexts: &[&str],
    ) -> BidiResult<String> {
        let result = self
            .connection
            .send(
                "script.addPreloadScript",
                json!({
                    "functionDeclaration": function_declaration,
                    "contexts": contexts,
                }),
            )
            .await?;
        result
            .get("script")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| BidiError::Unexpected("script.addPreloadScript missing script".into()))
    }

    /// Remove a preload script.
    pub async fn remove_preload_script(&self, script: &str) -> BidiResult<()> {
        self.connection
            .send("script.removePreloadScript", json!({ "script": script }))
            .await?;
        Ok(())
    }

    /// Capture a screenshot of a context as PNG bytes.
    pub async fn screenshot(&self, context: &str) -> BidiResult<Vec<u8>> {
        let result = self
            .connection
            .send(
                "browsingContext.captureScreenshot",
                json!({ "context": context }),
            )
            .await?;
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| BidiError::Unexpected("captureScreenshot missing data".into()))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| BidiError::Unexpected(format!("invalid screenshot data: {error}")))
    }

    /// Override a context's viewport.
    pub async fn set_viewport(
        &self,
        context: &str,
        width: i64,
        height: i64,
        device_pixel_ratio: f64,
    ) -> BidiResult<()> {
        self.connection
            .send(
                "browsingContext.setViewport",
                json!({
                    "context": context,
                    "viewport": { "width": width, "height": height },
                    "devicePixelRatio": device_pixel_ratio,
                }),
            )
            .await?;
        Ok(())
    }

    /// Perform a sequence of input actions against a context.
    ///
    /// `actions` is the BiDi `actions` array (pointer, key, wheel, ...).
    pub async fn perform_actions(&self, context: &str, actions: Value) -> BidiResult<()> {
        self.connection
            .send(
                "input.performActions",
                json!({ "context": context, "actions": actions }),
            )
            .await?;
        Ok(())
    }

    /// Release any inputs still held in a context.
    pub async fn release_actions(&self, context: &str) -> BidiResult<()> {
        self.connection
            .send("input.releaseActions", json!({ "context": context }))
            .await?;
        Ok(())
    }

    /// End the session.
    pub async fn end(&self) -> BidiResult<()> {
        let _ = self.connection.send("session.end", json!({})).await;
        Ok(())
    }

    // -- Request interception ----------------------------------------------

    /// Intercept requests for a context (all URLs; filtering is done locally).
    pub async fn add_intercept(&self, context: &str, phases: &[&str]) -> BidiResult<String> {
        let params = crate::network::AddInterceptParams {
            phases: phases.iter().map(|phase| (*phase).to_string()).collect(),
            url_patterns: vec![crate::network::UrlPattern {
                kind: "pattern".to_string(),
                pattern: "*".to_string(),
            }],
            contexts: vec![context.to_string()],
        };
        let result = self
            .connection
            .send("network.addIntercept", json!(params))
            .await?;
        result
            .get("intercept")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| BidiError::Unexpected("network.addIntercept missing intercept".into()))
    }

    /// Remove an interception.
    pub async fn remove_intercept(&self, intercept: &str) -> BidiResult<()> {
        self.connection
            .send(
                "network.removeIntercept",
                json!(crate::network::RemoveInterceptParams {
                    intercept: intercept.to_string()
                }),
            )
            .await?;
        Ok(())
    }

    /// Continue a blocked request unchanged.
    pub async fn continue_request(&self, request: &str) -> BidiResult<()> {
        self.continue_request_with_headers(request, None).await
    }

    /// Continue a blocked request, optionally replacing its headers.
    pub(crate) async fn continue_request_with_headers(
        &self,
        request: &str,
        headers: Option<Vec<crate::network::HeaderEntry>>,
    ) -> BidiResult<()> {
        self.connection
            .send(
                "network.continueRequest",
                json!(crate::network::ContinueRequestParams {
                    request: request.to_string(),
                    headers,
                }),
            )
            .await?;
        Ok(())
    }

    /// Continue a blocked response, optionally replacing headers/status.
    pub(crate) async fn continue_response(
        &self,
        request: &str,
        status: Option<i64>,
        headers: Option<Vec<crate::network::HeaderEntry>>,
    ) -> BidiResult<()> {
        self.connection
            .send(
                "network.continueResponse",
                json!(crate::network::ContinueResponseParams {
                    request: request.to_string(),
                    status_code: status,
                    reason_phrase: None,
                    headers,
                }),
            )
            .await?;
        Ok(())
    }

    /// Fail a blocked request.
    pub async fn fail_request(&self, request: &str) -> BidiResult<()> {
        self.connection
            .send(
                "network.failRequest",
                json!(crate::network::FailRequestParams {
                    request: request.to_string()
                }),
            )
            .await?;
        Ok(())
    }

    /// Answer a blocked request with a synthetic response.
    pub async fn provide_response(
        &self,
        request: &str,
        status: i64,
        content_type: &str,
        body: &[u8],
    ) -> BidiResult<()> {
        let params = crate::network::ProvideResponseParams {
            request: request.to_string(),
            status_code: status,
            headers: vec![crate::network::HeaderEntry {
                name: "Content-Type".to_string(),
                value: crate::network::BytesValue {
                    kind: "string".to_string(),
                    value: content_type.to_string(),
                },
            }],
            body: Some(crate::network::BytesValue {
                kind: "base64".to_string(),
                value: base64::engine::general_purpose::STANDARD.encode(body),
            }),
        };
        self.connection
            .send("network.provideResponse", json!(params))
            .await?;
        Ok(())
    }

    // -- Cookies and storage ------------------------------------------------

    /// Cookies visible to a context.
    pub async fn get_cookies(&self, context: &str) -> BidiResult<Vec<BidiCookie>> {
        let result = self
            .connection
            .send(
                "storage.getCookies",
                json!({ "partition": { "type": "context", "context": context } }),
            )
            .await?;
        let cookies = result
            .get("cookies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(cookies
            .iter()
            .map(|cookie| BidiCookie {
                name: string_field(cookie, "name"),
                value: bytes_value_string(cookie.get("value")),
                domain: string_field(cookie, "domain"),
                path: string_field(cookie, "path"),
                http_only: cookie
                    .get("httpOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                secure: cookie
                    .get("secure")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                same_site: cookie
                    .get("sameSite")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                expiry: cookie.get("expiry").and_then(Value::as_i64),
            })
            .collect())
    }

    /// Set a cookie for a domain.
    pub async fn set_cookie(
        &self,
        context: &str,
        name: &str,
        value: &str,
        domain: &str,
    ) -> BidiResult<()> {
        self.connection
            .send(
                "storage.setCookie",
                json!({
                    "cookie": {
                        "name": name,
                        "value": { "type": "string", "value": value },
                        "domain": domain,
                    },
                    "partition": { "type": "context", "context": context },
                }),
            )
            .await?;
        Ok(())
    }

    /// Delete cookies for a context (best effort).
    pub async fn delete_cookies(&self, context: &str) -> BidiResult<()> {
        self.connection
            .send(
                "storage.deleteCookies",
                json!({ "partition": { "type": "context", "context": context } }),
            )
            .await?;
        Ok(())
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn evaluation_result(result: &Value) -> BidiResult<Value> {
    if result["type"] == "exception" {
        return Err(BidiError::JavaScript(
            result["exceptionDetails"]["text"]
                .as_str()
                .unwrap_or("JavaScript evaluation failed")
                .to_string(),
        ));
    }
    Ok(remote_value_to_json(
        result.get("result").unwrap_or(&Value::Null),
    ))
}

/// Extract a string from a BiDi `BytesValue` (`{type, value}`).
fn bytes_value_string(value: Option<&Value>) -> String {
    value
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_contexts(result: &Value) -> BidiResult<Vec<BrowsingContextInfo>> {
    let contexts = result
        .get("contexts")
        .cloned()
        .ok_or_else(|| BidiError::Unexpected("getTree missing contexts".into()))?;
    Ok(serde_json::from_value(contexts)?)
}

/// Convert a BiDi `RemoteValue` into a plain JSON value.
///
/// Objects and arrays are reconstructed; values that cannot be represented
/// (functions, symbols, cycles) become `null`.
pub(crate) fn remote_value_to_json(value: &Value) -> Value {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "string" | "boolean" => value.get("value").cloned().unwrap_or(Value::Null),
        "number" => match value.get("value") {
            Some(Value::Number(number)) => Value::Number(number.clone()),
            _ => Value::Null,
        },
        "null" | "undefined" => Value::Null,
        "array" => {
            let items = value
                .get("value")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Value::Array(items.iter().map(remote_value_to_json).collect())
        }
        "object" => {
            let mut map = serde_json::Map::new();
            if let Some(entries) = value.get("value").and_then(Value::as_array) {
                for entry in entries {
                    if let Some(pair) = entry.as_array() {
                        if pair.len() == 2 {
                            if let Some(key) = pair[0].as_str() {
                                map.insert(key.to_string(), remote_value_to_json(&pair[1]));
                            }
                        }
                    }
                }
            }
            Value::Object(map)
        }
        _ => value.get("value").cloned().unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluation_result, remote_value_to_json};
    use crate::BidiError;
    use serde_json::json;

    #[test]
    fn evaluation_exceptions_preserve_the_browser_message() {
        let result = evaluation_result(&json!({
            "type": "exception", "exceptionDetails": {"text": "Error: boom"},
        }));
        assert!(matches!(result, Err(BidiError::JavaScript(ref text)) if text == "Error: boom"));
        assert!(matches!(
            evaluation_result(&json!({"type":"exception"})),
            Err(BidiError::JavaScript(_))
        ));
    }

    #[test]
    fn successful_null_and_undefined_are_distinct_from_exceptions() {
        for kind in ["null", "undefined"] {
            assert_eq!(
                evaluation_result(&json!({"type":"success", "result":{"type":kind}})).unwrap(),
                json!(null)
            );
        }
        assert_eq!(
            evaluation_result(&json!({"type":"success", "result":{"type":"number", "value":42}}))
                .unwrap(),
            json!(42)
        );
    }

    #[test]
    fn converts_scalars() {
        assert_eq!(
            remote_value_to_json(&json!({"type": "string", "value": "hi"})),
            json!("hi")
        );
        assert_eq!(
            remote_value_to_json(&json!({"type": "number", "value": 7})),
            json!(7)
        );
        assert_eq!(
            remote_value_to_json(&json!({"type": "boolean", "value": true})),
            json!(true)
        );
        assert_eq!(
            remote_value_to_json(&json!({"type": "null"})),
            serde_json::Value::Null
        );
    }

    #[test]
    fn converts_arrays_and_objects() {
        let value = json!({
            "type": "object",
            "value": [
                ["a", {"type": "number", "value": 1}],
                ["b", {"type": "array", "value": [
                    {"type": "number", "value": 2},
                    {"type": "number", "value": 3}
                ]}]
            ]
        });
        assert_eq!(remote_value_to_json(&value), json!({"a": 1, "b": [2, 3]}));
    }
}
