//! `DOM` domain.

use serde::{Deserialize, Serialize};

/// Parameters of `DOM.getDocument`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDocumentParams {
    /// Depth of the returned tree; `-1` for the entire tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
    /// Traverse through iframes and shadow roots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pierce: Option<bool>,
}

/// Response of `DOM.getDocument`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDocumentResult {
    /// The root node.
    pub root: Node,
}

/// A DOM node as described by the `DOM` domain.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// Node id.
    pub node_id: i64,
    /// Backend node id.
    #[serde(default)]
    pub backend_node_id: i64,
    /// Node type.
    #[serde(default)]
    pub node_type: i64,
    /// Node name.
    #[serde(default)]
    pub node_name: String,
    /// Local name.
    #[serde(default)]
    pub local_name: String,
    /// Node value.
    #[serde(default)]
    pub node_value: String,
    /// The frame this node owns, for `<iframe>` / `<frame>` elements.
    #[serde(default)]
    pub frame_id: Option<String>,
}

/// Parameters of `DOM.getBoxModel`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBoxModelParams {
    /// Node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    /// Backend node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    /// Remote object id of a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

/// Response of `DOM.getBoxModel`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBoxModelResult {
    /// The box model.
    pub model: BoxModel,
}

/// A DOM box model.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxModel {
    /// Content quad.
    pub content: Quad,
    /// Padding quad.
    #[serde(default)]
    pub padding: Quad,
    /// Border quad.
    #[serde(default)]
    pub border: Quad,
    /// Margin quad.
    #[serde(default)]
    pub margin: Quad,
    /// Element width.
    #[serde(default)]
    pub width: f64,
    /// Element height.
    #[serde(default)]
    pub height: f64,
}

impl BoxModel {
    /// The center point of the content quad, in CSS pixels.
    pub fn content_center(&self) -> (f64, f64) {
        self.content.center()
    }
}

/// A quad expressed as `[x1, y1, x2, y2, x3, y3, x4, y4]`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Quad(pub Vec<f64>);

impl Quad {
    /// The center of the quad.
    pub fn center(&self) -> (f64, f64) {
        let points = self.0.chunks_exact(2);
        let count = self.0.len() / 2;
        if count == 0 {
            return (0.0, 0.0);
        }
        let mut x = 0.0;
        let mut y = 0.0;
        for point in points {
            x += point[0];
            y += point[1];
        }
        (x / count as f64, y / count as f64)
    }
}

/// Parameters of `DOM.getOuterHTML`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOuterHTMLParams {
    /// Node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    /// Backend node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    /// Remote object id of a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

/// Response of `DOM.getOuterHTML`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOuterHTMLResult {
    /// Serialized HTML.
    pub outer_html: String,
}

/// Parameters of `DOM.focus`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusParams {
    /// Node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    /// Backend node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    /// Remote object id of a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

/// Parameters of `DOM.setFileInputFiles`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFileInputFilesParams {
    /// Absolute paths of the files to assign.
    pub files: Vec<String>,
    /// Node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    /// Backend node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    /// Remote object id of a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

/// Parameters of `DOM.describeNode`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeNodeParams {
    /// Remote object id of a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    /// Backend node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    /// Node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
}

/// Response of `DOM.describeNode`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeNodeResult {
    /// The described node.
    pub node: Node,
}

/// Parameters of `DOM.getFrameOwner`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameOwnerParams {
    /// The frame whose owner node is requested.
    pub frame_id: String,
}

/// Response of `DOM.getFrameOwner`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameOwnerResult {
    /// Backend node id of the owning `<iframe>`/`<frame>` element.
    #[serde(default)]
    pub backend_node_id: i64,
    /// Node id of the owning element, when available.
    #[serde(default)]
    pub node_id: Option<i64>,
}
