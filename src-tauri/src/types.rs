use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceToggle {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub visible: bool,
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Account {
    pub email: String,
    pub uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Organization {
    pub organization_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    pub account: Account,
    pub organization: Organization,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RefreshResult {
    pub success: bool,
    pub updated: bool,
    pub old_version: String,
    pub new_version: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomField {
    pub path: String,
    pub key: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSource {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_header")]
    pub header: String,
    #[serde(default)]
    pub fields: Vec<CustomField>,
}

fn default_header() -> String {
    "Authorization".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCandidate {
    pub path: String,
    pub key: String,
    pub label: String,
    pub preview: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTestResult {
    pub fields: Vec<CustomCandidate>,
    #[serde(default)]
    pub keys: Vec<CustomCandidate>,
    pub raw: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub ninerouter_url: String,
    pub custom_sources: Vec<CustomSource>,
    pub show_remaining: bool,
    pub sources: Vec<SourceToggle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupProfile {
    pub email: String,
    pub plan: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupBar {
    pub key: String,
    pub label: String,
    pub pct_text: String,
    pub fill_pct: f64,
    pub warn: bool,
    pub reset_text: String,
    pub dividers: Vec<f64>,
    pub marker_rel: Option<f64>,
    #[serde(default = "bar_kind")]
    pub kind: String,
}

fn bar_kind() -> String {
    "bar".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraUsageView {
    pub has_limit: bool,
    pub pct_text: String,
    pub fill_pct: f64,
    pub spent_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupSnapshot {
    pub profile: Option<PopupProfile>,
    pub usage: Vec<PopupBar>,
    pub extra: Option<ExtraUsageView>,
    pub installations: Vec<Value>,
    pub status: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupInit {
    pub colors: Value,
    pub t: Value,
    pub app_version: String,
    pub compact_hide: Vec<String>,
    pub data: PopupSnapshot,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomPayload {
    #[serde(default)]
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub fields: Vec<CustomField>,
}
