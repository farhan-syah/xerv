//! Visual metadata for the web UI.
//!
//! These types store UI state (node positions, zoom level, etc.) and are
//! embedded in YAML but ignored by the executor.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

/// Visual metadata for the web UI.
///
/// This is embedded in the YAML and ignored by the executor.
/// It stores node positions, zoom level, and other UI state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct FlowMetadata {
    /// UI-specific metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiMetadata>,

    /// Additional custom metadata fields.
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// UI-specific visual metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct UiMetadata {
    /// Canvas viewport state.
    #[serde(default)]
    pub canvas: CanvasState,

    /// Node-specific UI state, keyed by node ID.
    #[serde(default)]
    pub nodes: HashMap<String, NodeUiState>,
}

/// Canvas viewport state.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct CanvasState {
    /// Zoom level (1.0 = 100%).
    #[serde(default = "default_zoom")]
    pub zoom: f64,

    /// Viewport offset.
    #[serde(default)]
    pub viewport: ViewportOffset,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            viewport: ViewportOffset::default(),
        }
    }
}

fn default_zoom() -> f64 {
    1.0
}

/// Viewport offset coordinates.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct ViewportOffset {
    /// X offset in pixels.
    #[serde(default)]
    pub x: f64,

    /// Y offset in pixels.
    #[serde(default)]
    pub y: f64,
}

/// Node-specific UI state.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct NodeUiState {
    /// Node position on canvas.
    pub position: NodePosition,
}

/// Node position on canvas.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct NodePosition {
    /// X coordinate in pixels.
    pub x: f64,

    /// Y coordinate in pixels.
    pub y: f64,
}
