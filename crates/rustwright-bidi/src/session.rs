//! Typed helpers over the WebDriver BiDi protocol.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

/// An established BiDi session.
#[derive(Clone)]
pub struct BidiSession {
    connection: BidiConnection,
    session_id: String,
    capabilities: Value,
    network_subscribed: Arc<AtomicBool>,
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
            network_subscribed: Arc::new(AtomicBool::new(false)),
        })
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
    pub async fn ensure_network_subscription(&self) -> BidiResult<()> {
        if self.network_subscribed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = self
            .subscribe(&[
                "network.beforeRequestSent",
                "network.responseCompleted",
                "network.fetchError",
            ])
            .await;
        if result.is_err() {
            self.network_subscribed.store(false, Ordering::SeqCst);
        }
        result
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
        let result = self
            .connection
            .send("browsingContext.create", params)
            .await?;
        result
            .get("context")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| BidiError::Unexpected("browsingContext.create missing context".into()))
    }

    /// Create an isolated user context (its own cookies, storage and cache).
    pub async fn create_user_context(&self) -> BidiResult<String> {
        let result = self
            .connection
            .send("browser.createUserContext", json!({}))
            .await?;
        result
            .get("userContext")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                BidiError::Unexpected("browser.createUserContext missing userContext".into())
            })
    }

    /// Remove an isolated user context.
    pub async fn remove_user_context(&self, user_context: &str) -> BidiResult<()> {
        self.connection
            .send(
                "browser.removeUserContext",
                json!({ "userContext": user_context }),
            )
            .await?;
        Ok(())
    }

    /// Close a browsing context.
    pub async fn close_context(&self, context: &str) -> BidiResult<()> {
        self.connection
            .send("browsingContext.close", json!({ "context": context }))
            .await?;
        Ok(())
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
        let remote = result.get("result").cloned().unwrap_or(Value::Null);
        Ok(remote_value_to_json(&remote))
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
    pub async fn add_intercept(&self, context: &str) -> BidiResult<String> {
        let params = crate::network::AddInterceptParams {
            phases: vec!["beforeRequestSent".to_string()],
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
        self.connection
            .send(
                "network.continueRequest",
                json!(crate::network::ContinueRequestParams {
                    request: request.to_string()
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
    use super::remote_value_to_json;
    use serde_json::json;

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
