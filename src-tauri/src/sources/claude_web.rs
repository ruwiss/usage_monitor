//! Claude usage through a claude.ai web session.
//!
//! The user signs in inside an app-owned webview window (`claude-login`). The session lives only in
//! that webview's own profile directory. Usage is read by running `fetch()` inside a hidden webview
//! (`claude-bridge`) that shares the profile, so requests carry the real browser fingerprint and
//! cookies (claude.ai sits behind Cloudflare, which rejects non-browser TLS clients). The page
//! hands the result back through its URL fragment, so no Tauri IPC is exposed to claude.ai and the
//! cookies never enter this process (they are only checked for presence to detect a finished login).

use crate::i18n::t;
use crate::sources::claude::merge_limits;
use crate::state::AppState;
use crate::types::{Account, Organization, Profile};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const SOURCE_ID: &str = "claude-web";
const LOGIN_LABEL: &str = "claude-login";
const BRIDGE_LABEL: &str = "claude-bridge";
const ORIGIN_PAGE: &str = "https://claude.ai/robots.txt";
const ORGS_URL: &str = "https://claude.ai/api/organizations";
const ACCOUNT_URL: &str = "https://claude.ai/api/account";
const DASHBOARD: &str = "https://claude.ai/settings/usage";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const LOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// Largest result fragment read back from the bridge window URL.
const MAX_FRAGMENT: usize = 1024 * 1024;
/// Hosts (and their subdomains) the visible login window may navigate to: Claude and Anthropic.
const LOGIN_DOMAINS: &[&str] = &["claude.ai", "claude.com", "anthropic.com"];
/// Exact extra hosts needed for "Continue with Google" (no other google.com / googleusercontent.com host).
const LOGIN_HOSTS: &[&str] = &["accounts.google.com", "ssl.gstatic.com", "www.gstatic.com"];
/// The hidden bridge window only ever talks to claude.ai itself.
const BRIDGE_HOSTS: &[&str] = &["claude.ai"];
#[cfg(target_os = "macos")]
const STORE_ID: [u8; 16] = *b"usage-monitor-cw";

static APP: OnceLock<AppHandle> = OnceLock::new();
static GATE: Mutex<()> = Mutex::new(());
static LOADED: AtomicBool = AtomicBool::new(false);
static ACCOUNT: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

// ---- storage ------------------------------------------------------------------------------------

fn root_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.9router.usage-monitor")
        .join("claude-web")
}

fn profile_dir() -> PathBuf {
    root_dir().join("profile")
}

fn marker() -> PathBuf {
    root_dir().join("signed-in")
}

/// True once a login completed in this app (marker only; the session itself stays in the webview profile).
pub fn has_login() -> bool {
    marker().is_file()
}

// ---- navigation filter ----------------------------------------------------------------------------

/// Common checks: https, a real host, no userinfo, default port. Returns the normalized host.
fn plain_https_host(url: &Url) -> Option<String> {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    Some(url.host_str()?.trim_end_matches('.').to_ascii_lowercase())
}

fn is_blank(url: &Url) -> bool {
    url.scheme() == "about" && url.as_str() == "about:blank"
}

/// Navigation filter of the visible login window.
pub fn allowed_url(url: &Url) -> bool {
    if is_blank(url) {
        return true;
    }
    let Some(host) = plain_https_host(url) else { return false };
    LOGIN_HOSTS.contains(&host.as_str()) || LOGIN_DOMAINS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Navigation filter of the hidden bridge window.
pub fn bridge_allowed_url(url: &Url) -> bool {
    if is_blank(url) {
        return true;
    }
    plain_https_host(url).is_some_and(|h| BRIDGE_HOSTS.contains(&h.as_str()))
}

/// The user is past the sign-in pages on claude.ai.
fn login_finished(url: &Url) -> bool {
    if url.scheme() != "https" || url.host_str() != Some("claude.ai") {
        return false;
    }
    let path = url.path();
    !["/login", "/magic-link", "/onboarding", "/logout", "/oauth"].iter().any(|p| path.starts_with(p))
}

fn builder<'a>(
    app: &'a AppHandle,
    label: &str,
    url: &str,
    allow: fn(&Url) -> bool,
) -> Result<WebviewWindowBuilder<'a, tauri::Wry, AppHandle>, String> {
    let url: Url = url.parse().map_err(|_| "bad url".to_string())?;
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url)).on_navigation(filtered(allow));
    Ok(with_profile(b))
}

/// Navigation filter that logs (scheme and host only, never the query) what it blocks.
fn filtered(allow: fn(&Url) -> bool) -> impl Fn(&Url) -> bool + Send + 'static {
    move |url| {
        let ok = allow(url);
        if !ok {
            eprintln!("claude-web: blocked navigation to {}://{}", url.scheme(), url.host_str().unwrap_or(""));
        }
        ok
    }
}

fn with_profile<'a>(b: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    #[cfg(not(target_os = "macos"))]
    let b = b.data_directory(profile_dir());
    #[cfg(target_os = "macos")]
    let b = b.data_store_identifier(STORE_ID);
    b
}

static POPUP_SEQ: AtomicU32 = AtomicU32::new(0);

/// "Continue with Google" opens a popup via window.open() and reports back with postMessage, so the popup
/// must be created from the opener (keeps the opener link) in the same profile, under the same filter.
fn login_popup(app: &AppHandle, url: Url, features: tauri::webview::NewWindowFeatures) -> NewWindowResponse<tauri::Wry> {
    if !allowed_url(&url) {
        eprintln!("claude-web: blocked popup to {}://{}", url.scheme(), url.host_str().unwrap_or(""));
        return NewWindowResponse::Deny;
    }
    let label = format!("claude-login-popup-{}", POPUP_SEQ.fetch_add(1, Ordering::SeqCst));
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External("about:blank".parse().expect("url")))
        .on_navigation(filtered(allowed_url))
        .window_features(features)
        .title(t("claude_web_window_title"));
    match with_profile(builder).build() {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}

// ---- sign in / out --------------------------------------------------------------------------------

pub fn open_login(app: &AppHandle, state: Arc<AppState>) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let _ = std::fs::create_dir_all(profile_dir());
    let popup_app = app.clone();
    let window = builder(app, LOGIN_LABEL, "https://claude.ai/login", allowed_url)?
        .on_new_window(move |url, features| login_popup(&popup_app, url, features))
        .title(t("claude_web_window_title"))
        .inner_size(520.0, 760.0)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    let _ = window.set_focus();
    let app = app.clone();
    std::thread::spawn(move || watch_login(app, state));
    Ok(())
}

fn watch_login(app: AppHandle, state: Arc<AppState>) {
    let started = Instant::now();
    let claude: Url = "https://claude.ai/".parse().expect("url");
    loop {
        std::thread::sleep(Duration::from_millis(1500));
        let Some(window) = app.get_webview_window(LOGIN_LABEL) else { return };
        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.destroy();
            return;
        }
        let Ok(url) = window.url() else { continue };
        if !login_finished(&url) {
            continue;
        }
        // Presence check only; the value is never read into a variable that outlives this closure.
        let signed_in = window
            .cookies_for_url(claude.clone())
            .map(|c| c.iter().any(|c| c.name() == "sessionKey"))
            .unwrap_or(false);
        if !signed_in {
            continue;
        }
        let _ = std::fs::create_dir_all(root_dir());
        let _ = std::fs::write(marker(), b"1");
        let _ = window.destroy();
        if let Some(bridge) = app.get_webview_window(BRIDGE_LABEL) {
            let _ = bridge.destroy();
        }
        *ACCOUNT.lock() = None;
        crate::sources::invalidate_caches();
        let _ = crate::settings::set_source_visible(&state, SOURCE_ID.into(), true);
        crate::sources::select_source(&state, SOURCE_ID.into());
        crate::tray::refresh(&app, &state);
        crate::poll::force_update(app.clone(), state);
        return;
    }
}

/// Forget the session: drop the marker, wipe the webview data and delete the profile directory.
pub fn sign_out(app: &AppHandle) {
    let _ = std::fs::remove_file(marker());
    *ACCOUNT.lock() = None;
    crate::sources::invalidate_caches();
    #[cfg(target_os = "macos")]
    if app.get_webview_window(BRIDGE_LABEL).is_none() && app.get_webview_window(LOGIN_LABEL).is_none() {
        // The macOS data store is only reachable through a live webview.
        if let Ok(b) = builder(app, BRIDGE_LABEL, "about:blank", bridge_allowed_url) {
            let _ = b.visible(false).build();
        }
    }
    for label in [LOGIN_LABEL, BRIDGE_LABEL] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.clear_all_browsing_data();
            let _ = w.destroy();
        }
    }
    std::thread::spawn(|| {
        // WebView2 releases its files shortly after the window is gone.
        for _ in 0..40 {
            if has_login() {
                return; // signed in again meanwhile
            }
            if !profile_dir().exists() || std::fs::remove_dir_all(profile_dir()).is_ok() {
                let _ = std::fs::remove_dir(root_dir());
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

// ---- in-page fetch ----------------------------------------------------------------------------------

fn ensure_bridge(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(BRIDGE_LABEL) {
        return Ok(w);
    }
    LOADED.store(false, Ordering::SeqCst);
    let window = builder(app, BRIDGE_LABEL, ORIGIN_PAGE, bridge_allowed_url)?
        .title("Usage Monitor")
        .visible(false)
        .skip_taskbar(true)
        .focused(false)
        .inner_size(400.0, 300.0)
        .on_page_load(|_, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                LOADED.store(true, Ordering::SeqCst);
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + LOAD_TIMEOUT;
    while !LOADED.load(Ordering::SeqCst) {
        if Instant::now() >= deadline {
            let _ = window.destroy();
            return Err("claude.ai did not load".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Ok(window)
}

fn fetch_script(url: &str, prefix: &str) -> String {
    format!(
        "(async()=>{{let r;try{{const x=await fetch({url},{{credentials:'include',headers:{{'accept':'application/json'}}}});\
         const t=await x.text();r={{s:x.status,b:t.slice(0,200000)}};}}catch(e){{r={{s:0,b:String(e)}}}}\
         location.hash={prefix}+encodeURIComponent(JSON.stringify(r));}})()",
        url = json!(url),
        prefix = json!(prefix),
    )
}

/// Unpredictable per-request nonce (128 random bits, hex). The page being driven cannot guess
/// it, so it cannot forge a result for a request it has not seen.
fn new_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Decode `um:<nonce>:<percent-encoded json {s,b}>` from a URL fragment. Oversized or malformed
/// fragments yield `None`.
fn parse_fragment(fragment: &str, prefix: &str) -> Option<(i64, String)> {
    if fragment.len() > MAX_FRAGMENT {
        return None;
    }
    let rest = fragment.strip_prefix(prefix)?;
    let decoded = url::form_urlencoded::parse(format!("v={rest}").as_bytes()).next()?.1.into_owned();
    let v: Value = serde_json::from_str(&decoded).ok()?;
    Some((v.get("s")?.as_i64()?, v.get("b")?.as_str()?.to_string()))
}

/// GET `url` from inside the signed-in claude.ai page. Returns (status, body).
fn page_fetch(url: &str) -> Result<(i64, String), String> {
    let app = APP.get().ok_or("app not ready")?;
    let _gate = GATE.lock();
    let window = ensure_bridge(app)?;
    let prefix = format!("um:{}:", new_nonce()?);
    window.eval(fetch_script(url, &prefix)).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + FETCH_TIMEOUT;
    loop {
        if let Ok(u) = window.url() {
            if u.fragment().is_some_and(|f| f.len() > MAX_FRAGMENT && f.starts_with(&prefix)) {
                let _ = window.destroy();
                return Err("claude.ai response too large".into());
            }
            if let Some((status, body)) = u.fragment().and_then(|f| parse_fragment(f, &prefix)) {
                if status == 403 && body.contains("Just a moment") {
                    let _ = window.destroy(); // challenge page: start from a clean page next time
                }
                return Ok((status, body));
            }
        }
        if Instant::now() >= deadline {
            let _ = window.destroy();
            return Err("claude.ai did not respond".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn err(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

fn status_error(status: i64, body: &str) -> Map<String, Value> {
    if status == 403 && body.contains("Just a moment") {
        return err(&t("claude_web_blocked"));
    }
    let mut m = match status {
        0 => err(&t("connection_error")),
        401 | 403 => err(&t("claude_web_signin_needed")),
        429 => {
            let mut m = err(&crate::i18n::t_fmt("http_error", &[("code", "429")]));
            m.insert("rate_limited".into(), json!(true));
            m
        }
        500..=599 => err(&crate::i18n::t_fmt("server_error", &[("code", &status.to_string())])),
        _ => err(&crate::i18n::t_fmt("http_error", &[("code", &status.to_string())])),
    };
    m.insert("http_status".into(), json!(status));
    m
}

fn get_json(url: &str) -> Result<Value, Map<String, Value>> {
    let (status, body) = page_fetch(url).map_err(|e| err(&format!("{}\n{e}", t("connection_error"))))?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &body));
    }
    serde_json::from_str(&body).map_err(|_| err(&t("connection_error")))
}

// ---- usage / profile ------------------------------------------------------------------------------------

pub fn usage() -> Map<String, Value> {
    if !has_login() {
        return err(&t("claude_web_signin_needed"));
    }
    match fetch_usage() {
        Ok(m) | Err(m) => m,
    }
}

fn fetch_usage() -> Result<Map<String, Value>, Map<String, Value>> {
    let orgs = get_json(ORGS_URL)?;
    let org = pick_org(&orgs).ok_or_else(|| err(&t("claude_web_no_org")))?;
    let id = org.get("uuid").and_then(Value::as_str).unwrap_or_default();
    let Value::Object(data) = get_json(&format!("{ORGS_URL}/{id}/usage"))? else {
        return Err(err(&t("connection_error")));
    };
    let mut out = parse_usage(data, org);
    if ACCOUNT.lock().is_none() {
        if let Ok(acc) = get_json(ACCOUNT_URL) {
            *ACCOUNT.lock() = Some(parse_account(&acc));
        }
    }
    out.insert("_links".into(), json!({ "dashboard_url": DASHBOARD }));
    Ok(out)
}

pub fn profile() -> Option<Profile> {
    if !has_login() {
        return None;
    }
    if ACCOUNT.lock().is_none() {
        if let Ok(acc) = get_json(ACCOUNT_URL) {
            *ACCOUNT.lock() = Some(parse_account(&acc));
        }
    }
    let (email, uuid) = ACCOUNT.lock().clone()?;
    Some(Profile {
        account: Account { email, uuid: if uuid.is_empty() { SOURCE_ID.into() } else { uuid } },
        organization: Organization::default(),
    })
}

/// (email, uuid) from `/api/account`.
pub fn parse_account(v: &Value) -> (String, String) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let email = if s("email_address").is_empty() { s("email") } else { s("email_address") };
    (email, s("uuid"))
}

/// The organization with the best subscription (max > pro > any chat-capable), valid uuid only.
pub fn pick_org(orgs: &Value) -> Option<&Value> {
    let mut best: Option<(&Value, i32)> = None;
    for org in orgs.as_array()? {
        let uuid = org.get("uuid").and_then(Value::as_str).unwrap_or("");
        if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let caps = org.get("capabilities").map(|c| c.to_string()).unwrap_or_default();
        let score = if caps.contains("claude_max") {
            3
        } else if caps.contains("claude_pro") {
            2
        } else if caps.contains("chat") {
            1
        } else {
            0
        };
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((org, score));
        }
    }
    best.map(|(o, _)| o)
}

pub fn plan_name(org: &Value) -> Option<String> {
    let tier = org.get("rate_limit_tier").and_then(Value::as_str).unwrap_or("");
    let caps = org.get("capabilities").map(|c| c.to_string()).unwrap_or_default();
    let text = format!("{tier} {caps}").to_lowercase();
    let name = if text.contains("max_20x") {
        "Max 20x"
    } else if text.contains("max_5x") {
        "Max 5x"
    } else if text.contains("max") {
        "Max"
    } else if text.contains("pro") {
        "Pro"
    } else if text.contains("team") {
        "Team"
    } else if text.contains("enterprise") {
        "Enterprise"
    } else if text.trim().is_empty() {
        return None;
    } else {
        "Free"
    };
    Some(name.into())
}

/// The web usage payload has the same shape as the OAuth one (five_hour, seven_day, seven_day_sonnet,
/// extra_usage, ...), so it flows through the same normalization as the CLI source.
pub fn parse_usage(data: Map<String, Value>, org: &Value) -> Map<String, Value> {
    let mut out = merge_limits(data);
    if let Some(plan) = plan_name(org) {
        out.insert("_plan".into(), json!(plan));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orgs() -> Value {
        serde_json::from_str(
            r#"[
              {"uuid":"11111111-aaaa","name":"api org","capabilities":["api"],"rate_limit_tier":"default"},
              {"uuid":"22222222-bbbb","name":"omer's Organization","capabilities":["chat","claude_max"],"rate_limit_tier":"default_claude_max_5x"},
              {"uuid":"33333333-cccc","name":"team","capabilities":["chat","claude_pro"],"rate_limit_tier":"default"}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn picks_best_org_and_plan() {
        let v = orgs();
        let org = pick_org(&v).unwrap();
        assert_eq!(org["uuid"], "22222222-bbbb");
        assert_eq!(plan_name(org).as_deref(), Some("Max 5x"));
    }

    #[test]
    fn rejects_unsafe_org_uuid() {
        let v: Value = serde_json::from_str(r#"[{"uuid":"a/../b","capabilities":["chat"]}]"#).unwrap();
        assert!(pick_org(&v).is_none());
    }

    #[test]
    fn plan_names() {
        let free: Value = serde_json::from_str(r#"{"capabilities":["chat"],"rate_limit_tier":"default"}"#).unwrap();
        assert_eq!(plan_name(&free).as_deref(), Some("Free"));
        let pro: Value = serde_json::from_str(r#"{"capabilities":["claude_pro"]}"#).unwrap();
        assert_eq!(plan_name(&pro).as_deref(), Some("Pro"));
    }

    #[test]
    fn parses_usage_like_oauth_payload() {
        let usage: Map<String, Value> = serde_json::from_str(
            r#"{
              "five_hour":{"utilization":37.0,"resets_at":"2026-11-06T18:00:00+00:00"},
              "seven_day":{"utilization":12.5,"resets_at":"2026-11-10T00:00:00+00:00"},
              "seven_day_sonnet":{"utilization":4.0,"resets_at":"2026-11-10T00:00:00+00:00"},
              "seven_day_opus":null,
              "extra_usage":{"is_enabled":true,"monthly_limit":2000,"used_credits":350.0,"utilization":17.5}
            }"#,
        )
        .unwrap();
        let orgs = orgs();
        let out = parse_usage(usage, pick_org(&orgs).unwrap());
        assert_eq!(out["five_hour"]["utilization"], 37.0);
        assert_eq!(out["seven_day_sonnet"]["utilization"], 4.0);
        assert_eq!(out["extra_usage"]["used_credits"], 350.0);
        assert_eq!(out["_plan"], "Max 5x");
    }

    #[test]
    fn parses_account() {
        let v: Value = serde_json::from_str(r#"{"uuid":"u-1","email_address":"a@b.c"}"#).unwrap();
        assert_eq!(parse_account(&v), ("a@b.c".into(), "u-1".into()));
    }

    #[test]
    fn navigation_filter() {
        let ok = |s: &str| allowed_url(&s.parse::<Url>().unwrap());
        assert!(ok("https://claude.ai/login"));
        assert!(ok("https://accounts.google.com/o/oauth2/auth"));
        assert!(ok("https://www.gstatic.com/x"));
        assert!(ok("https://console.anthropic.com/x"));
        assert!(ok("https://CLAUDE.AI/login"));
        assert!(ok("about:blank"));
        assert!(!ok("https://mail.google.com/"));
        assert!(!ok("https://lh3.googleusercontent.com/x"));
        assert!(!ok("https://accounts.google.com.evil.com/"));
        assert!(!ok("https://evil.com/accounts.google.com"));
        assert!(!ok("https://claude.ai@evil.com/"));
        assert!(!ok("https://user:pw@claude.ai/"));
        assert!(!ok("https://claude.ai:8443/"));
        assert!(!ok("about:srcdoc"));
        assert!(!ok("data:text/html,x"));
        assert!(!ok("http://claude.ai/login"));
        assert!(!ok("https://evilclaude.ai/"));
        assert!(!ok("https://claude.ai.evil.com/"));
        assert!(!ok("file:///c:/x"));
        assert!(!ok("https://example.com/"));
    }

    #[test]
    fn bridge_navigation_filter() {
        let ok = |s: &str| bridge_allowed_url(&s.parse::<Url>().unwrap());
        assert!(ok("https://claude.ai/robots.txt"));
        assert!(ok("about:blank"));
        assert!(!ok("https://accounts.google.com/"));
        assert!(!ok("https://console.anthropic.com/"));
        assert!(!ok("https://www.claude.ai/"));
        assert!(!ok("https://claude.ai.evil.com/"));
        assert!(!ok("https://evilclaude.ai/"));
        assert!(!ok("https://claude.ai@evil.com/"));
        assert!(!ok("http://claude.ai/"));
    }

    #[test]
    fn nonces_are_random_hex() {
        let a = new_nonce().unwrap();
        let b = new_nonce().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn oversized_or_garbage_fragments_are_rejected() {
        let big = format!("um:7:{}", "a".repeat(MAX_FRAGMENT + 1));
        assert!(parse_fragment(&big, "um:7:").is_none());
        for junk in ["um:7:", "um:7:%", "um:7:%zz", "um:7:%00%ff", "um:7:{}", "um:7:null", "", "um:7:%7B%22s%22%3A%22x%22%7D"] {
            assert!(parse_fragment(junk, "um:7:").is_none(), "{junk}");
        }
    }

    #[test]
    fn login_finished_detection() {
        let f = |s: &str| login_finished(&s.parse::<Url>().unwrap());
        assert!(f("https://claude.ai/new"));
        assert!(!f("https://claude.ai/login?returnTo=%2F"));
        assert!(!f("https://claude.ai/magic-link"));
        assert!(!f("https://accounts.google.com/"));
    }

    #[test]
    fn decodes_fragment_result() {
        let body = r#"{"s":200,"b":"{\"a\":1} é"}"#;
        let enc: String = url::form_urlencoded::byte_serialize(body.as_bytes()).collect::<String>().replace('+', "%20");
        let (s, b) = parse_fragment(&format!("um:7:{enc}"), "um:7:").unwrap();
        assert_eq!(s, 200);
        assert_eq!(b, "{\"a\":1} é");
        assert!(parse_fragment(&format!("um:8:{enc}"), "um:7:").is_none());
    }
}
