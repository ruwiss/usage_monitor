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
/// A manually added provider API key (see `sources::keyed`). The key is stored in settings.json
/// as plain text, like custom source tokens, and is never sent back to the UI.
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyedSource {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub key: String,
}

impl std::fmt::Debug for KeyedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyedSource")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("name", &self.name)
            .field("key", &"<redacted>")
            .finish()
    }
}

/// Catalog entry shown in the "Add provider key" dropdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyProviderInfo {
    pub kind: String,
    pub name: String,
    pub hint: String,
    pub key_url: String,
    pub key_optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub ninerouter_url: String,
    pub key_providers: Vec<KeyProviderInfo>,
    pub custom_sources: Vec<CustomSource>,
    pub show_remaining: bool,
    pub provider_auto_update: bool,
    pub sources: Vec<SourceToggle>,
    #[serde(default)]
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUpdatePreference {
    pub enabled: bool,
    pub error: Option<String>,
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
    /// The quota currently shown on the tray icon's top slot (chosen automatically).
    #[serde(default)]
    pub tray_active: bool,
    /// Run-out forecast text ("At this pace, limit reached in 2d 4h"); empty when usage is not ahead of pace.
    #[serde(default)]
    pub runout_text: String,
    /// True when the projected run-out is imminent (shown in the warning colour).
    #[serde(default)]
    pub runout_urgent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoRow {
    pub label: String,
    pub value: String,
}

/// Generic credits / info section built from the optional `_credits`, `_info` and `_links` usage keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupInfo {
    pub rows: Vec<InfoRow>,
    #[serde(default)]
    pub dashboard_url: Option<String>,
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
pub struct PopupGroup {
    pub provider: String,
    pub index: i32,
    pub count: i32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupSnapshot {
    pub profile: Option<PopupProfile>,
    pub usage: Vec<PopupBar>,
    pub extra: Option<ExtraUsageView>,
    #[serde(default)]
    pub info: Option<PopupInfo>,
    #[serde(default)]
    pub group: Option<PopupGroup>,
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
