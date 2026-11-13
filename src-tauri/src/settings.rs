use crate::error::{Error, Result};
use crate::instance::{effective_config_dir, is_default_config_dir};
use crate::state::AppState;
use crate::types::{CustomPayload, CustomSource, KeyedSource, SettingsView};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use url::Url;

const SETTINGS_FILENAME: &str = "usage-monitor-settings.json";

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| Error::from(e.to_string()))?;
    let temp = path.with_file_name(format!(".usage-monitor-settings-{:032x}.tmp", u128::from_le_bytes(random)));
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(fs::metadata(path).map(|m| m.permissions().mode()).unwrap_or(0o600));
        }
        let mut file = options.open(&temp)?;
        created = true;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if created { let _ = fs::remove_file(temp); }
    result
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub poll_interval: i64,
    pub poll_fast: i64,
    pub poll_fast_extra: i64,
    pub poll_error: i64,
    pub max_backoff: i64,
    pub idle_pause: i64,
    pub bg: String,
    pub fg: String,
    pub fg_dim: String,
    pub fg_heading: String,
    pub fg_link: String,
    pub bar_bg: String,
    pub bar_fg: String,
    pub bar_fg_warn: String,
    pub bar_divider: String,
    pub bar_marker: String,
    pub icon_light: HashMap<String, [u8; 4]>,
    pub icon_dark: HashMap<String, [u8; 4]>,
    pub icon_fields: Vec<String>,
    pub icon_style: String,
    pub tooltip_fields: Vec<String>,
    pub popup_fields: Vec<String>,
    pub compact_hide: Vec<String>,
    pub alert_time_aware: bool,
    pub alert_time_aware_below: f64,
    pub auto_update: bool,
    pub provider_auto_update: bool,
    pub notify_provider_update: bool,
    pub last_provider_update_check: i64,
    pub last_update_check: i64,
    pub currency_symbol: Option<String>,
    pub language: String,
    pub ninerouter_url: String,
    pub connection_id: String,
    pub source_id: String,
    pub custom_sources: Vec<CustomSource>,
    pub keyed_sources: Vec<KeyedSource>,
    pub hidden_sources: Vec<String>,
    pub show_remaining: bool,
    pub time_format: String,
    pub cli_command: HashMap<String, Vec<String>>,
    pub quick_action_command: Vec<String>,
    pub on_reset_command: Vec<String>,
    pub on_startup_command: Vec<String>,
    pub on_threshold_command: Vec<String>,
    pub alert_thresholds: HashMap<String, Vec<f64>>,
    pub alert_extra_usage_spent: Vec<f64>,
    pub bar_labels: HashMap<String, String>,
    /// Provider (9Router group) -> chosen connection id, for grouped sources.
    pub group_selections: HashMap<String, String>,
    raw: Map<String, Value>,
    path: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval: 180,
            poll_fast: 120,
            poll_fast_extra: 2,
            poll_error: 30,
            max_backoff: 900,
            idle_pause: 300,
            bg: "#1e1e1e".into(),
            fg: "#cccccc".into(),
            fg_dim: "#888888".into(),
            fg_heading: "#ffffff".into(),
            fg_link: "#4a9eff".into(),
            bar_bg: "#333333".into(),
            bar_fg: "#4a9eff".into(),
            bar_fg_warn: "#e05050".into(),
            bar_divider: "#000c".into(),
            bar_marker: "#fffc".into(),
            icon_light: default_icon_light(),
            icon_dark: default_icon_dark(),
            icon_fields: vec!["five_hour".into(), "seven_day".into()],
            icon_style: "number+bars".into(),
            tooltip_fields: vec!["five_hour".into(), "seven_day".into()],
            popup_fields: vec!["*".into()],
            compact_hide: vec![],
            alert_time_aware: true,
            alert_time_aware_below: 90.0,
            auto_update: true,
            provider_auto_update: true,
            notify_provider_update: true,
            last_provider_update_check: 0,
            last_update_check: 0,
            currency_symbol: None,
            language: String::new(),
            ninerouter_url: "http://localhost:20128".into(),
            connection_id: String::new(),
            source_id: String::new(),
            custom_sources: vec![],
            keyed_sources: vec![],
            hidden_sources: vec![],
            time_format: crate::platform::system_time_format(),
            cli_command: HashMap::new(),
            quick_action_command: vec![],
            on_reset_command: vec![],
            on_startup_command: vec![],
            on_threshold_command: vec![],
            alert_thresholds: default_thresholds(),
            alert_extra_usage_spent: vec![],
            bar_labels: HashMap::new(),
            group_selections: HashMap::new(),
            show_remaining: false,
            raw: Map::new(),
            path: write_path(),
        }
    }
}

fn default_icon_light() -> HashMap<String, [u8; 4]> {
    HashMap::from([
        ("fg".into(), [255, 255, 255, 255]),
        ("fg_half".into(), [255, 255, 255, 80]),
        ("fg_dim".into(), [255, 255, 255, 140]),
        ("fg_warn".into(), [224, 80, 80, 255]),
    ])
}

fn default_icon_dark() -> HashMap<String, [u8; 4]> {
    HashMap::from([
        ("fg".into(), [0, 0, 0, 255]),
        ("fg_half".into(), [0, 0, 0, 80]),
        ("fg_dim".into(), [0, 0, 0, 140]),
        ("fg_warn".into(), [224, 80, 80, 255]),
    ])
}

fn default_thresholds() -> HashMap<String, Vec<f64>> {
    HashMap::from([
        ("five_hour".into(), vec![50.0, 80.0, 95.0]),
        ("seven_day".into(), vec![95.0]),
        ("extra_usage".into(), vec![50.0, 80.0, 95.0]),
    ])
}

pub fn language_override() -> String {
    // Used during i18n load before Settings::load finishes; re-read file cheaply.
    Settings::load().language
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        if let Some((path, data)) = read_first() {
            s.path = path;
            s.apply(data);
        }
        if let Ok(url) = std::env::var("NINEROUTER_URL") {
            if !url.is_empty() {
                s.ninerouter_url = url.trim_end_matches('/').into();
            }
        }
        if let Ok(id) = std::env::var("USAGE_MONITOR_CONNECTION") {
            if !id.is_empty() {
                s.source_id = id;
            }
        }
        s
    }

    fn apply(&mut self, mut data: Map<String, Value>) {
        validate(&mut data);
        self.raw = data.clone();
        if let Some(v) = data.get("poll_interval").and_then(Value::as_i64) { self.poll_interval = v; }
        if let Some(v) = data.get("poll_fast").and_then(Value::as_i64) { self.poll_fast = v; }
        if let Some(v) = data.get("poll_fast_extra").and_then(Value::as_i64) { self.poll_fast_extra = v; }
        if let Some(v) = data.get("poll_error").and_then(Value::as_i64) { self.poll_error = v; }
        if let Some(v) = data.get("max_backoff").and_then(Value::as_i64) { self.max_backoff = v; }
        if let Some(v) = data.get("idle_pause").and_then(Value::as_i64) { self.idle_pause = v; }
        take_str(&data, "bg", &mut self.bg);
        take_str(&data, "fg", &mut self.fg);
        take_str(&data, "fg_dim", &mut self.fg_dim);
        take_str(&data, "fg_heading", &mut self.fg_heading);
        take_str(&data, "fg_link", &mut self.fg_link);
        take_str(&data, "bar_bg", &mut self.bar_bg);
        take_str(&data, "bar_fg", &mut self.bar_fg);
        take_str(&data, "bar_fg_warn", &mut self.bar_fg_warn);
        take_str(&data, "bar_divider", &mut self.bar_divider);
        take_str(&data, "bar_marker", &mut self.bar_marker);
        if let Some(v) = data.get("icon_fields").and_then(Value::as_array) {
            self.icon_fields = str_list(v);
        }
        take_str(&data, "icon_style", &mut self.icon_style);
        if let Some(v) = data.get("tooltip_fields").and_then(Value::as_array) {
            self.tooltip_fields = str_list(v);
        }
        if let Some(v) = data.get("popup_fields").and_then(Value::as_array) {
            self.popup_fields = str_list(v);
        }
        if let Some(v) = data.get("compact_hide").and_then(Value::as_array) {
            self.compact_hide = str_list(v);
        }
        if let Some(v) = data.get("alert_time_aware").and_then(Value::as_bool) { self.alert_time_aware = v; }
        if let Some(v) = data.get("alert_time_aware_below").and_then(Value::as_f64) { self.alert_time_aware_below = v; }
        if let Some(v) = data.get("provider_auto_update").and_then(Value::as_bool) { self.provider_auto_update = v; }
        self.notify_provider_update = data.get("notify_provider_update").or_else(|| data.get("notify_claude_update")).and_then(Value::as_bool).unwrap_or(true);
        if let Some(v) = data.get("last_provider_update_check").and_then(Value::as_i64) { self.last_provider_update_check = v; }
        if let Some(v) = data.get("auto_update").and_then(Value::as_bool) { self.auto_update = v; }
        if let Some(v) = data.get("last_update_check").and_then(Value::as_i64) { self.last_update_check = v; }
        if data.contains_key("currency_symbol") {
            self.currency_symbol = data.get("currency_symbol").and_then(Value::as_str).map(|s| s.to_string());
        }
        if let Some(v) = data.get("show_remaining").and_then(Value::as_bool) { self.show_remaining = v; }
        take_str(&data, "language", &mut self.language);
        take_str(&data, "ninerouter_url", &mut self.ninerouter_url);
        take_str(&data, "connection_id", &mut self.connection_id);
        take_str(&data, "source_id", &mut self.source_id);
        take_str(&data, "time_format", &mut self.time_format);
        if let Some(Value::Object(obj)) = data.get("cli_command") {
            self.cli_command.clear();
            for (k, v) in obj {
                let cmd = command_list(v);
                if !cmd.is_empty() {
                    self.cli_command.insert(k.clone(), cmd);
                }
            }
        }
        if let Some(v) = data.get("custom_sources").and_then(Value::as_array) {
            self.custom_sources = v.iter().filter_map(|item| serde_json::from_value(item.clone()).ok()).collect();
        }
        if let Some(v) = data.get("keyed_sources").and_then(Value::as_array) {
            self.keyed_sources = v.iter().filter_map(|item| serde_json::from_value(item.clone()).ok()).collect();
        }
        if let Some(v) = data.get("hidden_sources").and_then(Value::as_array) {
            self.hidden_sources = v.iter().filter_map(Value::as_str).map(str::to_string).collect();
        }
        if let Some(Value::String(cmd)) = data.get("quick_action_command") {
            self.quick_action_command = if cmd.trim().is_empty() { vec![] } else { vec![cmd.clone()] };
        } else if let Some(v) = data.get("quick_action_command").and_then(Value::as_array) {
            self.quick_action_command = str_list(v);
        } else if let Some(v) = data.get("on_double_click_command") {
            self.quick_action_command = command_list(v);
        }
        self.on_reset_command = command_list(data.get("on_reset_command").unwrap_or(&Value::Null));
        self.on_startup_command = command_list(data.get("on_startup_command").unwrap_or(&Value::Null));
        self.on_threshold_command = command_list(data.get("on_threshold_command").unwrap_or(&Value::Null));
        if let Some(v) = data.get("alert_extra_usage_spent").and_then(Value::as_array) {
            self.alert_extra_usage_spent = v.iter().filter_map(Value::as_f64).collect();
        }
        for (k, v) in &data {
            if let Some(rest) = k.strip_prefix("alert_thresholds_") {
                if let Some(arr) = v.as_array() {
                    self.alert_thresholds.insert(rest.to_string(), arr.iter().filter_map(Value::as_f64).collect());
                }
            }
        }
        merge_icon(&data, "icon_light", &mut self.icon_light);
        merge_icon(&data, "icon_dark", &mut self.icon_dark);
        if let Some(Value::Object(obj)) = data.get("bar_labels") {
            self.bar_labels = obj
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
        }
        if let Some(Value::Object(obj)) = data.get("group_selections") {
            self.group_selections = obj
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
        }
        self.ninerouter_url = self.ninerouter_url.trim_end_matches('/').into();
    }

    pub fn view(&self) -> SettingsView {
        SettingsView {
            ninerouter_url: self.ninerouter_url.clone(),
            key_providers: crate::sources::keyed::provider_infos(),
            custom_sources: self.custom_sources.clone(),
            show_remaining: self.show_remaining,
            provider_auto_update: self.provider_auto_update,
            sources: crate::sources::source_toggles(self),
            language: self.language.clone(),
        }
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.hidden_sources.iter().any(|hidden| hidden == id)
    }

    pub fn set_source_visible(&mut self, id: &str, visible: bool) {
        if visible {
            self.hidden_sources.retain(|hidden| hidden != id);
        } else if !self.is_hidden(id) {
            self.hidden_sources.push(id.to_string());
        }
    }

    pub fn save_setting(&mut self, key: &str, value: Value) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            self.path = write_path();
        }
        let mut data = match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str::<Value>(&text)?.as_object().cloned().ok_or_else(|| Error::from("Settings must be a JSON object"))?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(err) => return Err(err.into()),
        };
        data.insert(key.to_string(), value.clone());
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&self.path, &(serde_json::to_string_pretty(&Value::Object(data.clone()))? + "\n"))?;
        self.raw = data;
        match key {
            "source_id" => self.source_id = value.as_str().unwrap_or("").into(),
            "language" => self.language = value.as_str().unwrap_or("").into(),
            "show_remaining" => self.show_remaining = value.as_bool().unwrap_or(false),
            "ninerouter_url" => self.ninerouter_url = value.as_str().unwrap_or("http://localhost:20128").trim_end_matches('/').into(),
            "connection_id" => self.connection_id = value.as_str().unwrap_or("").into(),
            "custom_sources" => {
                if let Some(arr) = value.as_array() {
                    self.custom_sources = arr.iter().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect();
                }
            }
            "keyed_sources" => {
                if let Some(arr) = value.as_array() {
                    self.keyed_sources = arr.iter().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect();
                }
            }
            "hidden_sources" => {
                if let Some(arr) = value.as_array() {
                    self.hidden_sources = arr.iter().filter_map(Value::as_str).map(str::to_string).collect();
                }
            }
            "provider_auto_update" => self.provider_auto_update = value.as_bool().unwrap_or(true),
            "notify_provider_update" => self.notify_provider_update = value.as_bool().unwrap_or(true),
            "last_provider_update_check" => self.last_provider_update_check = value.as_i64().unwrap_or(0),
            "auto_update" => self.auto_update = value.as_bool().unwrap_or(true),
            "last_update_check" => self.last_update_check = value.as_i64().unwrap_or(0),
            "bar_labels" => {
                if let Some(obj) = value.as_object() {
                    self.bar_labels = obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect();
                }
            }
            "group_selections" => {
                if let Some(obj) = value.as_object() {
                    self.group_selections = obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Disabling always takes effect for this session, even on a read-only or
    /// corrupt settings file. Enabling requires a successful persistent save.
    pub fn set_provider_auto_update(&mut self, enabled: bool) -> Result<()> {
        if !enabled { self.provider_auto_update = false; }
        self.save_setting("provider_auto_update", json!(enabled))
    }

    /// Persist a custom display name for a usage bar. Empty name clears it.
    pub fn set_bar_label(&mut self, key: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            self.bar_labels.remove(key);
        } else {
            self.bar_labels.insert(key.to_string(), name.to_string());
        }
        let value = serde_json::to_value(&self.bar_labels)?;
        self.save_setting("bar_labels", value)
    }

    pub fn bar_label<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.bar_labels.get(key).map(String::as_str).unwrap_or(fallback)
    }

    /// Remember which connection was last picked inside a 9Router group.
    pub fn set_group_selection(&mut self, provider: &str, conn_id: &str) -> Result<()> {
        self.group_selections.insert(provider.to_string(), conn_id.to_string());
        let value = serde_json::to_value(&self.group_selections)?;
        self.save_setting("group_selections", value)
    }

    pub fn get_alert_thresholds(&self, variant_key: &str) -> Vec<f64> {
        if let Some(v) = self.alert_thresholds.get(variant_key) {
            return v.clone();
        }
        let parts: Vec<&str> = variant_key.splitn(3, '_').collect();
        if parts.len() >= 3 {
            let base = format!("{}_{}", parts[0], parts[1]);
            if let Some(v) = self.alert_thresholds.get(&base) {
                return v.clone();
            }
        }
        vec![]
    }
}

/// Snapshot the settings, release the global lock and only then build the view: building it
/// runs source discovery (child process, network) which must never happen under the lock.
/// Any mutation can change which sources exist, so the discovery caches are dropped first.
fn view_unlocked(settings: parking_lot::MutexGuard<'_, Settings>) -> SettingsView {
    let snapshot = settings.clone();
    drop(settings);
    crate::sources::invalidate_caches();
    snapshot.view()
}

pub fn save_ninerouter(state: &AppState, url: String) -> Result<SettingsView> {
    let cleaned = url.trim().trim_end_matches('/').to_string();
    let parsed = Url::parse(&cleaned).map_err(|_| Error::from("URL must be http(s)"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(Error::from("URL must be http(s)"));
    }
    let mut settings = state.settings.lock();
    settings.save_setting("ninerouter_url", json!(cleaned))?;
    Ok(view_unlocked(settings))
}

pub fn add_custom(state: &AppState, payload: CustomPayload) -> Result<SettingsView> {
    let name = payload.name.trim().to_string();
    let url = payload.url.trim().to_string();
    let parsed = Url::parse(&url).map_err(|_| Error::from("Name and http(s) URL required"))?;
    if name.is_empty() || (parsed.scheme() != "http" && parsed.scheme() != "https") || parsed.host_str().is_none() {
        return Err(Error::from("Name and http(s) URL required"));
    }
    let slug = slug(&name);
    let fields = payload
        .fields
        .into_iter()
        .filter(|f| !f.path.trim().is_empty())
        .collect();
    let mut settings = state.settings.lock();
    settings.custom_sources.retain(|s| s.id != slug);
    settings.custom_sources.push(CustomSource {
        id: slug.clone(),
        name,
        url,
        token: payload.token,
        header: if payload.header.trim().is_empty() { "Authorization".into() } else { payload.header },
        fields,
    });
    settings.set_source_visible(&format!("custom:{slug}"), true);
    let value = serde_json::to_value(&settings.custom_sources)?;
    settings.save_setting("custom_sources", value)?;
    let hidden = serde_json::to_value(&settings.hidden_sources)?;
    settings.save_setting("hidden_sources", hidden)?;
    Ok(view_unlocked(settings))
}

pub fn remove_custom(state: &AppState, id: &str) -> Result<SettingsView> {
    let slug = id.strip_prefix("custom:").unwrap_or(id).to_string();
    let source_id = format!("custom:{slug}");
    let mut settings = state.settings.lock();
    settings.custom_sources.retain(|s| s.id != slug);
    settings.hidden_sources.retain(|hidden| hidden != &source_id && hidden != &slug);
    let value = serde_json::to_value(&settings.custom_sources)?;
    settings.save_setting("custom_sources", value)?;
    let hidden = serde_json::to_value(&settings.hidden_sources)?;
    settings.save_setting("hidden_sources", hidden)?;
    Ok(view_unlocked(settings))
}

/// Saves a provider key that already passed `sources::keyed::test`.
pub fn add_keyed(state: &AppState, provider: &str, key: &str, name: &str) -> Result<SettingsView> {
    let def = crate::sources::keyed::find(provider).ok_or_else(|| Error::from(crate::i18n::t("key_unknown_provider")))?;
    let mut settings = state.settings.lock();
    let id = crate::sources::keyed::new_entry_id(&settings.keyed_sources, def.kind, name.trim());
    settings.keyed_sources.push(KeyedSource {
        id: id.clone(),
        provider: def.kind.into(),
        name: name.trim().into(),
        key: key.trim().into(),
    });
    settings.set_source_visible(&crate::sources::keyed::source_id(def.kind, &id), true);
    let value = serde_json::to_value(&settings.keyed_sources)?;
    settings.save_setting("keyed_sources", value)?;
    let hidden = serde_json::to_value(&settings.hidden_sources)?;
    settings.save_setting("hidden_sources", hidden)?;
    Ok(view_unlocked(settings))
}

pub fn remove_keyed(state: &AppState, source_id: &str) -> Result<SettingsView> {
    let mut settings = state.settings.lock();
    if let Some((provider, id)) = source_id.strip_prefix("key:").and_then(|p| p.split_once(':')) {
        settings.keyed_sources.retain(|e| !(e.provider == provider && e.id == id));
    }
    settings.hidden_sources.retain(|hidden| hidden != source_id);
    let value = serde_json::to_value(&settings.keyed_sources)?;
    settings.save_setting("keyed_sources", value)?;
    let hidden = serde_json::to_value(&settings.hidden_sources)?;
    settings.save_setting("hidden_sources", hidden)?;
    Ok(view_unlocked(settings))
}

pub fn set_show_remaining(state: &AppState, remaining: bool) -> Result<SettingsView> {
    let mut settings = state.settings.lock();
    settings.save_setting("show_remaining", json!(remaining))?;
    Ok(view_unlocked(settings))
}

pub fn set_language(state: &AppState, code: String) -> Result<SettingsView> {
    let code = code.trim().to_string();
    if !crate::i18n::language_available(&code) {
        return Err(Error::from("Unknown language"));
    }
    let mut settings = state.settings.lock();
    settings.save_setting("language", json!(code))?;
    Ok(view_unlocked(settings))
}

pub fn set_source_visible(state: &AppState, id: String, visible: bool) -> Result<SettingsView> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err(Error::from("Source id required"));
    }
    let mut settings = state.settings.lock();
    settings.set_source_visible(&id, visible);
    let value = serde_json::to_value(&settings.hidden_sources)?;
    settings.save_setting("hidden_sources", value)?;
    Ok(view_unlocked(settings))
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "custom".into() } else { s }
}

fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !is_default_config_dir() {
        paths.push(effective_config_dir().join(SETTINGS_FILENAME));
    }
    paths.push(app_dir().join(SETTINGS_FILENAME));
    if let Some(bundle) = crate::platform::macos_bundle_dir() {
        if let Some(parent) = bundle.parent() {
            paths.push(parent.join(SETTINGS_FILENAME));
        }
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude").join(SETTINGS_FILENAME));
    }
    paths
}

fn app_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
    } else if let Ok(exe) = std::env::current_exe() {
        exe.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        PathBuf::from(".")
    }
}

/// Directory holding the settings file; other small app data files live next to it.
pub fn data_dir() -> PathBuf {
    write_path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn write_path() -> PathBuf {
    for path in search_paths() {
        if path.is_file() {
            return path;
        }
    }
    if !dir_writable(&app_dir()) {
        if let Some(home) = dirs::home_dir() {
            let dir = home.join(".claude");
            let _ = fs::create_dir_all(&dir);
            return dir.join(SETTINGS_FILENAME);
        }
    }
    app_dir().join(SETTINGS_FILENAME)
}

fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(".usage-monitor-write-test");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn read_first() -> Option<(PathBuf, Map<String, Value>)> {
    for path in search_paths() {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).ok()?.trim().to_string();
        if text.is_empty() {
            return Some((path, Map::new()));
        }
        let value: Value = serde_json::from_str(&text).ok()?;
        return Some((path, value.as_object().cloned().unwrap_or_default()));
    }
    None
}

fn take_str(data: &Map<String, Value>, key: &str, dest: &mut String) {
    if let Some(s) = data.get(key).and_then(Value::as_str) {
        *dest = s.to_string();
    }
}

fn str_list(arr: &[Value]) -> Vec<String> {
    arr.iter().filter_map(Value::as_str).map(|s| s.to_string()).collect()
}

fn command_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        Value::Array(arr) => str_list(arr),
        _ => vec![],
    }
}

fn merge_icon(data: &Map<String, Value>, key: &str, dest: &mut HashMap<String, [u8; 4]>) {
    let Some(Value::Object(obj)) = data.get(key) else { return };
    for (k, v) in obj {
        if let Some(arr) = v.as_array() {
            if arr.len() == 4 {
                if let (Some(r), Some(g), Some(b), Some(a)) = (
                    arr[0].as_u64(), arr[1].as_u64(), arr[2].as_u64(), arr[3].as_u64(),
                ) {
                    dest.insert(k.clone(), [r as u8, g as u8, b as u8, a as u8]);
                }
            }
        }
    }
}

fn validate(data: &mut Map<String, Value>) {
    let numeric = ["poll_interval", "poll_fast", "poll_fast_extra", "poll_error", "max_backoff"];
    let mut drop = Vec::new();
    for key in numeric {
        if let Some(v) = data.get(key) {
            if !v.is_i64() || v.as_i64().unwrap_or(0) < 1 {
                drop.push(key.to_string());
            }
        }
    }
    if let Some(v) = data.get("idle_pause") {
        if !v.is_i64() || v.as_i64().unwrap_or(-1) < 0 {
            drop.push("idle_pause".into());
        }
    }
    if let Some(v) = data.get("time_format") {
        if !matches!(v.as_str(), Some("24h" | "12h")) {
            drop.push("time_format".into());
        }
    }
    if let Some(v) = data.get("icon_style") {
        if !matches!(v.as_str(), Some("number+bars" | "numbers")) {
            drop.push("icon_style".into());
        }
    }
    for key in drop {
        data.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preference_fixture() -> (PathBuf, Settings) {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!("um-provider-pref-{:032x}", u128::from_le_bytes(random)));
        fs::create_dir(&dir).unwrap();
        let mut settings = Settings::default();
        settings.path = dir.join(SETTINGS_FILENAME);
        (dir, settings)
    }

    #[test]
    fn provider_preference_persists_and_preserves_other_settings() {
        let (dir, mut settings) = preference_fixture();
        fs::write(&settings.path, r#"{"provider_auto_update":true,"source_id":"codex","auto_update":false}"#).unwrap();
        settings.set_provider_auto_update(false).unwrap();
        assert!(!settings.provider_auto_update);
        let stored: Value = serde_json::from_str(&fs::read_to_string(&settings.path).unwrap()).unwrap();
        assert_eq!(stored["provider_auto_update"], false);
        assert_eq!(stored["source_id"], "codex");
        assert_eq!(stored["auto_update"], false);
        settings.set_provider_auto_update(true).unwrap();
        assert!(settings.provider_auto_update);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1); // No secret-bearing temp files remain.
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_settings_are_preserved_and_disabling_still_takes_effect() {
        let (dir, mut settings) = preference_fixture();
        for original in ["{broken json", "[]", "null"] {
            fs::write(&settings.path, original).unwrap();
            settings.provider_auto_update = true;
            assert!(settings.set_provider_auto_update(false).is_err());
            assert!(!settings.provider_auto_update);
            assert!(settings.set_provider_auto_update(true).is_err());
            assert!(!settings.provider_auto_update);
            assert_eq!(fs::read_to_string(&settings.path).unwrap(), original);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_atomic_replace_leaves_target_untouched_and_cleans_up() {
        let (dir, _) = preference_fixture();
        let target = dir.join("occupied");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("original"), "keep me").unwrap();
        assert!(atomic_write(&target, "replacement").is_err());
        assert_eq!(fs::read_to_string(target.join("original")).unwrap(), "keep me");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn provider_updates_have_separate_switches_and_preserve_legacy_notification_choice() {
        let mut settings = Settings::default();
        settings.apply(json!({"auto_update": false, "notify_claude_update": false}).as_object().unwrap().clone());
        assert!(!settings.auto_update);
        assert!(settings.provider_auto_update);
        assert!(!settings.notify_provider_update);
        settings.apply(json!({"provider_auto_update": false, "notify_claude_update": false, "notify_provider_update": true}).as_object().unwrap().clone());
        assert!(!settings.provider_auto_update);
        assert!(settings.notify_provider_update);
    }

    #[test]
    fn drops_invalid_poll() {
        let mut data = Map::from_iter([("poll_interval".into(), json!(0)), ("poll_fast".into(), json!(30))]);
        validate(&mut data);
        assert!(!data.contains_key("poll_interval"));
        assert_eq!(data.get("poll_fast").and_then(Value::as_i64), Some(30));
    }

    #[test]
    fn save_setting_merges( ) {
        let dir = std::env::temp_dir().join(format!("um-set-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(SETTINGS_FILENAME);
        fs::write(&path, "{\n  \"source_id\": \"grok\"\n}\n").unwrap();
        let mut s = Settings::default();
        s.path = path.clone();
        s.save_setting("ninerouter_url", json!("http://localhost:9")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("grok"));
        assert!(text.contains("localhost:9"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hides_and_shows_source() {
        let mut s = Settings::default();
        s.set_source_visible("claude", false);
        assert!(s.is_hidden("claude"));
        s.set_source_visible("claude", true);
        assert!(!s.is_hidden("claude"));
    }

    #[test]
    fn save_setting_updates_language_in_memory() {
        let dir = std::env::temp_dir().join(format!("um-lang-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(SETTINGS_FILENAME);
        let mut s = Settings::default();
        s.path = path.clone();
        s.save_setting("language", json!("de")).unwrap();
        assert_eq!(s.language, "de");
        assert_eq!(s.view().language, "de");
        // Persisted for the next launch.
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"language\": \"de\""));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bar_label_roundtrip_and_clear() {
        let dir = std::env::temp_dir().join(format!("um-label-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut s = Settings::default();
        s.path = dir.join(SETTINGS_FILENAME);
        s.set_bar_label("five_hour", "Oturum").unwrap();
        assert_eq!(s.bar_label("five_hour", "x"), "Oturum");
        assert_eq!(s.bar_label("other", "x"), "x");
        s.set_bar_label("five_hour", "").unwrap();
        assert_eq!(s.bar_label("five_hour", "x"), "x");
        let _ = fs::remove_dir_all(dir);
    }
}
