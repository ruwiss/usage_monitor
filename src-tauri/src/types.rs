use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type UsageMap = Map<String, Value>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    pub utilization: f64,
    pub resets_at: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSource {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_header")]
    pub header: String,
}

fn default_header() -> String {
    "Authorization".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub ninerouter_url: String,
    pub custom_sources: Vec<CustomSource>,
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
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub token: String,
}
