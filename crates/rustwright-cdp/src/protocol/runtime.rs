//! `Runtime` domain.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `Runtime.evaluate`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateParams {
    /// Expression to evaluate.
    pub expression: String,
    /// Return the value by value instead of as a remote object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
    /// Await a returned promise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub await_promise: Option<bool>,
    /// Evaluate in a specific execution context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<i64>,
    /// Treat the evaluation as user-initiated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_gesture: Option<bool>,
}

/// Response of `Runtime.evaluate` and `Runtime.callFunctionOn`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResult {
    /// The resulting remote object.
    pub result: RemoteObject,
    /// Populated when evaluation threw.
    #[serde(default)]
    pub exception_details: Option<ExceptionDetails>,
}

/// A CDP remote object reference.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteObject {
    /// JSON type name (`object`, `string`, `number`, ...).
    #[serde(rename = "type")]
    pub object_type: String,
    /// Subtype (`null`, `array`, `node`, ...).
    #[serde(default)]
    pub subtype: Option<String>,
    /// Class name for objects.
    #[serde(default)]
    pub class_name: Option<String>,
    /// The value when returned by value.
    #[serde(default)]
    pub value: Option<Value>,
    /// String description.
    #[serde(default)]
    pub description: Option<String>,
    /// Handle for further operations.
    #[serde(default)]
    pub object_id: Option<String>,
}

impl RemoteObject {
    /// Whether the result represents JavaScript `null` or `undefined`.
    pub fn is_nullish(&self) -> bool {
        self.subtype.as_deref() == Some("null")
            || (self.object_type == "undefined")
            || (self.object_type == "object" && self.object_id.is_none() && self.value.is_none())
    }
}

/// Details about a thrown exception.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDetails {
    /// The exception message.
    #[serde(default)]
    pub text: String,
    /// The thrown value.
    #[serde(default)]
    pub exception: Option<RemoteObject>,
    /// Line number.
    #[serde(default)]
    pub line_number: i64,
    /// Column number.
    #[serde(default)]
    pub column_number: i64,
}

impl ExceptionDetails {
    /// A human-readable description of the exception.
    pub fn message(&self) -> String {
        if let Some(exception) = &self.exception {
            if let Some(description) = &exception.description {
                return description.clone();
            }
        }
        self.text.clone()
    }
}

/// Parameters of `Runtime.callFunctionOn`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFunctionOnParams {
    /// The function source, e.g. `function(a){ return a + 1; }`.
    pub function_declaration: String,
    /// The object to bind as `this`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    /// Arguments passed to the function.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<CallArgument>,
    /// Return by value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
    /// Await a returned promise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub await_promise: Option<bool>,
    /// Treat as user-initiated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_gesture: Option<bool>,
}

/// An argument to `Runtime.callFunctionOn`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallArgument {
    /// The primitive value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// A remote object handle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

impl CallArgument {
    /// Create a by-value argument.
    pub fn value(value: Value) -> Self {
        Self {
            value: Some(value),
            object_id: None,
        }
    }

    /// Create a by-reference argument.
    pub fn object_id(object_id: impl Into<String>) -> Self {
        Self {
            value: None,
            object_id: Some(object_id.into()),
        }
    }
}

/// A console message captured via `Runtime.consoleAPICalled`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleAPICalledParams {
    /// Console method (`log`, `warn`, `error`, ...).
    #[serde(rename = "type")]
    pub call_type: String,
    /// Arguments passed to the console call.
    #[serde(default)]
    pub args: Vec<RemoteObject>,
    /// Monotonic timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: f64,
}

impl ConsoleAPICalledParams {
    /// Join the console arguments into a single string.
    pub fn text(&self) -> String {
        self.args
            .iter()
            .map(remote_object_to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Parameters of `Runtime.exceptionThrown`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionThrownParams {
    /// Monotonic timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: f64,
    /// Details about the exception.
    pub exception_details: ExceptionDetails,
}

/// Auxiliary data attached to an execution context.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextAuxData {
    /// The frame this context belongs to.
    #[serde(default)]
    pub frame_id: Option<String>,
    /// Whether this is the frame's default context.
    #[serde(default)]
    pub is_default: bool,
    /// The context type (`default`, `isolated`, ...).
    #[serde(rename = "type", default)]
    pub context_type: Option<String>,
}

/// An execution context.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextDescription {
    /// The unique context id.
    pub id: i64,
    /// The context origin.
    #[serde(default)]
    pub origin: String,
    /// The context name.
    #[serde(default)]
    pub name: String,
    /// Auxiliary data.
    #[serde(default)]
    pub aux_data: Option<ExecutionContextAuxData>,
}

/// Parameters of the `Runtime.executionContextCreated` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextCreatedParams {
    /// The created context.
    pub context: ExecutionContextDescription,
}

/// Parameters of the `Runtime.executionContextDestroyed` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextDestroyedParams {
    /// The destroyed context id.
    #[serde(default)]
    pub execution_context_id: i64,
}

fn remote_object_to_string(object: &RemoteObject) -> String {
    if let Some(value) = &object.value {
        match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }
    } else {
        object.description.clone().unwrap_or_default()
    }
}
