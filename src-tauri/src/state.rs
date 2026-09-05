use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use crate::cache::UsageCache;
use crate::settings::Settings;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub cache: UsageCache,
    pub current_id: Mutex<Option<String>>,
    pub last_response: Mutex<Map<String, Value>>,
    pub prev_utilization: Mutex<HashMap<String, f64>>,
    pub prev_account_uuid: Mutex<Option<String>>,
    pub first_update_done: Mutex<bool>,
    pub notified_thresholds: Mutex<HashMap<String, f64>>,
    pub fast_polls_remaining: Mutex<i32>,
    pub idle_reset_pending: Mutex<bool>,
    pub deferred_notifications: Mutex<HashMap<String, (String, String)>>,
    pub next_poll_time: Mutex<Option<f64>>,
    pub popup_pinned: Mutex<bool>,
    pub popup_moved: Mutex<bool>,
    pub popup_shown: Mutex<bool>,
    pub popup_closed_at: Mutex<Option<Instant>>,
    pub light_taskbar: Mutex<bool>,
    pub running: Mutex<bool>,
}

impl AppState {
    pub fn new(settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            settings: Mutex::new(settings),
            cache: UsageCache::new(),
            current_id: Mutex::new(None),
            last_response: Mutex::new(Map::new()),
            prev_utilization: Mutex::new(HashMap::new()),
            prev_account_uuid: Mutex::new(None),
            first_update_done: Mutex::new(false),
            notified_thresholds: Mutex::new(HashMap::new()),
            fast_polls_remaining: Mutex::new(0),
            idle_reset_pending: Mutex::new(false),
            deferred_notifications: Mutex::new(HashMap::new()),
            next_poll_time: Mutex::new(None),
            popup_pinned: Mutex::new(false),
            popup_moved: Mutex::new(false),
            popup_shown: Mutex::new(false),
            popup_closed_at: Mutex::new(None),
            light_taskbar: Mutex::new(false),
            running: Mutex::new(true),
        })
    }
}
