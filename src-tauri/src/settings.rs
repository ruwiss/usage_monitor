use crate::error::{Error, Result};
use crate::instance::{effective_config_dir, is_default_config_dir};
use crate::state::AppState;
use crate::types::{CustomPayload, CustomSource, SettingsView};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

const SETTINGS_FILENAME: &str = "usage-monitor-settings.json";

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
    pub notify_claude_update: bool,
    pub currency_symbol: Option<String>,
    pub language: String,
    pub ninerouter_url: String,
    pub connection_id: String,
    pub source_id: String,
    pub custom_sources: Vec<CustomSource>,
    pub show_remaining: bool,
    pub time_format: String,
    pub cli_command: HashMap<String, Vec<String>>,
    pub quick_action_command: Vec<String>,
    pub on_reset_command: Vec<String>,
    pub on_startup_command: Vec<String>,
    pub on_threshold_command: Vec<String>,
    pub alert_thresholds: HashMap<String, Vec<f64>>,
    pub alert_extra_usage_spent: Vec<f64>,
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
            notify_claude_update: true,
            currency_symbol: None,
            language: String::new(),
            ninerouter_url: "http://localhost:20128".into(),
            connection_id: String::new(),
            source_id: String::new(),
            custom_sources: vec![],
            time_format: crate::platform::system_time_format(),
            cli_command: HashMap::new(),
            quick_action_command: vec![],
            on_reset_command: vec![],
            on_startup_command: vec![],
            on_threshold_command: vec![],
            alert_thresholds: default_thresholds(),
            alert_extra_usage_spent: vec![],
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
        if let Some(v) = data.get("notify_claude_update").and_then(Value::as_bool) { self.notify_claude_update = v; }
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
        self.ninerouter_url = self.ninerouter_url.trim_end_matches('/').into();
    }

    pub fn view(&self) -> SettingsView {
        SettingsView {
            ninerouter_url: self.ninerouter_url.clone(),
            custom_sources: self.custom_sources.clone(),
            show_remaining: self.show_remaining,
        }
    }

    pub fn save_setting(&mut self, key: &str, value: Value) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            self.path = write_path();
        }
        let mut data = if self.path.is_file() {
            fs::read_to_string(&self.path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        } else {
            Map::new()
        };
        data.insert(key.to_string(), value.clone());
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, serde_json::to_string_pretty(&Value::Object(data.clone()))? + "\n")?;
        self.raw = data;
        match key {
            "source_id" => self.source_id = value.as_str().unwrap_or("").into(),
            "show_remaining" => self.show_remaining = value.as_bool().unwrap_or(false),
            "ninerouter_url" => self.ninerouter_url = value.as_str().unwrap_or("http://localhost:20128").trim_end_matches('/').into(),
            "connection_id" => self.connection_id = value.as_str().unwrap_or("").into(),
            "custom_sources" => {
                if let Some(arr) = value.as_array() {
                    self.custom_sources = arr.iter().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect();
                }
            }
            _ => {}
        }
        Ok(())
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

pub fn save_ninerouter(state: &AppState, url: String) -> Result<SettingsView> {
    let cleaned = url.trim().trim_end_matches('/').to_string();
    let parsed = Url::parse(&cleaned).map_err(|_| Error::from("URL must be http(s)"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(Error::from("URL must be http(s)"));
    }
    let mut settings = state.settings.lock();
    settings.save_setting("ninerouter_url", json!(cleaned))?;
    Ok(settings.view())
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
        id: slug,
        name,
        url,
        token: payload.token,
        header: if payload.header.trim().is_empty() { "Authorization".into() } else { payload.header },
        fields,
    });
    let value = serde_json::to_value(&settings.custom_sources)?;
    settings.save_setting("custom_sources", value)?;
    Ok(settings.view())
}

pub fn remove_custom(state: &AppState, id: &str) -> Result<SettingsView> {
    let mut settings = state.settings.lock();
    settings.custom_sources.retain(|s| s.id != id);
    let value = serde_json::to_value(&settings.custom_sources)?;
    settings.save_setting("custom_sources", value)?;
    Ok(settings.view())
}


pub fn set_show_remaining(state: &AppState, remaining: bool) -> Result<SettingsView> {
    let mut settings = state.settings.lock();
    settings.save_setting("show_remaining", json!(remaining))?;
    Ok(settings.view())
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
}
