//! Google Antigravity (the `agy` CLI / Antigravity IDE).
//!
//! The signed-in account lives in the Windows Credential Manager under `gemini:antigravity` as a
//! JSON blob: `{"id_token_email": "...", "token": {"access_token", "refresh_token", "expiry"}}`.
//! This module only ever READS that credential; it never writes, switches or deletes anything.
//! Because only the active login exists on disk, a single source `antigravity` is exposed.
//!
//! When the stored access token is expired it is refreshed against Google's token endpoint using
//! the OAuth client found inside the installed Antigravity binaries (nothing is bundled). The
//! refreshed token is kept in memory only.

use crate::http::{headers_from, post_bytes, post_form};
use crate::i18n::t;
use crate::types::{Account, Organization, Profile};
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration as StdDuration, Instant, SystemTime};

type Usage = Map<String, Value>;

const CRED_TARGET: &str = "gemini:antigravity";
const API: &str = "https://cloudcode-pa.googleapis.com/v1internal:";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DASHBOARD: &str = "https://antigravity.google";

#[derive(Debug, Clone, PartialEq)]
pub struct Cred {
    pub email: String,
    pub access: String,
    pub refresh: String,
    pub expiry: Option<DateTime<Utc>>,
}

/// Access tokens refreshed in this process, keyed by refresh token: (access token, expiry).
static REFRESHED: LazyLock<Mutex<HashMap<String, (String, DateTime<Utc>)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// OAuth client (id, secret) that last worked.
static CLIENT: LazyLock<Mutex<Option<(String, String)>>> = LazyLock::new(|| Mutex::new(None));

/// Binaries larger than this are skipped when looking for the OAuth client.
const MAX_SCAN_FILE: u64 = 300 * 1024 * 1024;
/// Read size of one scan step. Bounds memory use regardless of the binary size.
const SCAN_CHUNK: usize = 4 * 1024 * 1024;
/// Bytes carried from one chunk into the next; longer than any match so none is cut in two.
const SCAN_OVERLAP: usize = 256;
/// Upper bound of (id, secret) pairs POSTed to Google per refresh attempt.
const MAX_PAIRS: usize = 12;
/// No new refresh attempt for this long after the stored token was rejected.
const REJECT_BACKOFF: StdDuration = StdDuration::from_secs(10 * 60);
/// Same, after a transient failure (network, 5xx).
const TRANSIENT_BACKOFF: StdDuration = StdDuration::from_secs(60);

type Fingerprint = Vec<(PathBuf, u64, Option<SystemTime>)>;
/// Result of the binary scan, valid while the candidate files keep their size and mtime.
/// An empty list is cached too, so a missing client does not trigger a rescan every poll.
static DISCOVERED: LazyLock<Mutex<Option<(Fingerprint, Vec<(String, String)>)>>> = LazyLock::new(|| Mutex::new(None));
/// Last failed refresh: (refresh token, when, how long to wait).
static REFRESH_FAILED: LazyLock<Mutex<Option<(String, Instant, StdDuration)>>> = LazyLock::new(|| Mutex::new(None));

enum RefreshError {
    Rejected,
    Transient,
}

fn backoff_active(elapsed: StdDuration, wait: StdDuration) -> bool {
    elapsed < wait
}

// ---- credential ---------------------------------------------------------------------------

#[cfg(windows)]
fn read_blob() -> Option<Vec<u8>> {
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let target: Vec<u16> = CRED_TARGET.encode_utf16().chain(std::iter::once(0)).collect();
    let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: CredReadW fills `ptr` with a buffer we free with CredFree; the blob is copied first.
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) == 0 || ptr.is_null() {
            return None;
        }
        let c = &*ptr;
        let bytes = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        CredFree(ptr as *const _);
        Some(bytes)
    }
}

#[cfg(not(windows))]
fn read_blob() -> Option<Vec<u8>> {
    None
}

fn decode_blob(bytes: &[u8]) -> String {
    // Normally UTF-8; tolerate UTF-16LE blobs.
    if bytes.len() >= 2 && bytes[1] == 0 && bytes[0] != 0 {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn parse_cred(text: &str) -> Option<Cred> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let tok = v.get("token")?;
    let s = |x: Option<&Value>| x.and_then(Value::as_str).unwrap_or("").trim().to_string();
    let cred = Cred {
        email: s(v.get("id_token_email")),
        access: s(tok.get("access_token")),
        refresh: s(tok.get("refresh_token")),
        expiry: tok
            .get("expiry")
            .and_then(Value::as_str)
            .and_then(|e| DateTime::parse_from_rfc3339(e).ok())
            .map(|d| d.with_timezone(&Utc)),
    };
    if cred.access.is_empty() && cred.refresh.is_empty() {
        return None;
    }
    Some(cred)
}

pub fn credential() -> Option<Cred> {
    parse_cred(&decode_blob(&read_blob()?))
}

/// Cheap local check used for source discovery (no network).
pub fn has_login() -> bool {
    credential().is_some()
}

pub fn profile() -> Option<Profile> {
    let c = credential()?;
    Some(Profile {
        account: Account { email: c.email.clone(), uuid: "antigravity".into() },
        organization: Organization { organization_type: "Antigravity".into() },
    })
}

// ---- token refresh ------------------------------------------------------------------------

fn valid_access(c: &Cred, force_refresh: bool) -> std::result::Result<String, Usage> {
    let now = Utc::now();
    if !force_refresh {
        if let Some((tok, exp)) = REFRESHED.lock().get(&c.refresh) {
            if *exp > now + Duration::minutes(2) {
                return Ok(tok.clone());
            }
        }
        if !c.access.is_empty() && c.expiry.map(|e| e > now + Duration::minutes(2)).unwrap_or(false) {
            return Ok(c.access.clone());
        }
    }
    if c.refresh.is_empty() {
        return Err(err(&t("auth_expired")));
    }
    if let Some((key, at, wait)) = REFRESH_FAILED.lock().as_ref() {
        if *key == c.refresh && backoff_active(at.elapsed(), *wait) {
            return Err(err(&t("ag_refresh_failed")));
        }
    }
    match refresh(&c.refresh) {
        Ok((tok, secs)) => {
            *REFRESH_FAILED.lock() = None;
            REFRESHED.lock().insert(c.refresh.clone(), (tok.clone(), now + Duration::seconds(secs)));
            Ok(tok)
        }
        Err(e) => {
            let wait = if matches!(e, RefreshError::Rejected) { REJECT_BACKOFF } else { TRANSIENT_BACKOFF };
            *REFRESH_FAILED.lock() = Some((c.refresh.clone(), Instant::now(), wait));
            Err(err(&t("ag_refresh_failed")))
        }
    }
}

fn refresh(refresh_token: &str) -> std::result::Result<(String, i64), RefreshError> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(c) = CLIENT.lock().clone() {
        candidates.push(c);
    }
    for c in discover_clients() {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }
    candidates.truncate(MAX_PAIRS);
    let headers = headers_from(&[("Content-Type", "application/x-www-form-urlencoded")]);
    for (id, secret) in candidates {
        let r = post_form(
            TOKEN_URL,
            &[("client_id", &id), ("client_secret", &secret), ("refresh_token", refresh_token), ("grant_type", "refresh_token")],
            &headers,
        );
        if let Some(tok) = r.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            let secs = r.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
            *CLIENT.lock() = Some((id, secret));
            return Ok((tok.to_string(), secs));
        }
        // Wrong id/secret pairing or revoked token (400/401): try the next pair; stop on others.
        if !matches!(r.get("http_status").and_then(Value::as_u64), Some(400 | 401)) {
            return Err(RefreshError::Transient);
        }
    }
    Err(RefreshError::Rejected)
}

#[cfg(windows)]
fn client_files() -> Vec<std::path::PathBuf> {
    let Some(local) = dirs::data_local_dir() else { return vec![] };
    let mut files = Vec::new();
    let agy = local.join("agy").join("bin").join("agy.exe");
    if agy.is_file() {
        files.push(agy);
    }
    fn walk(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 6 {
                    walk(&p, depth + 1, out);
                }
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("language_server") && n.ends_with(".exe"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    walk(&local.join("Programs").join("Antigravity"), 0, &mut files);
    files
}

#[cfg(not(windows))]
fn client_files() -> Vec<std::path::PathBuf> {
    vec![]
}

fn fingerprint(files: &[PathBuf]) -> Fingerprint {
    files
        .iter()
        .map(|f| match std::fs::metadata(f) {
            Ok(m) => (f.clone(), m.len(), m.modified().ok()),
            Err(_) => (f.clone(), 0, None),
        })
        .collect()
}

fn merge_unique(into: &mut Vec<String>, from: Vec<String>) {
    for x in from {
        if !into.contains(&x) {
            into.push(x);
        }
    }
}

/// Scans the installed Antigravity binaries for the public OAuth client id/secret pair.
/// The result (including "nothing found") is cached until a candidate file changes.
fn discover_clients() -> Vec<(String, String)> {
    let files = client_files();
    let print = fingerprint(&files);
    if let Some((cached_print, pairs)) = DISCOVERED.lock().as_ref() {
        if *cached_print == print {
            return pairs.clone();
        }
    }
    let mut ids: Vec<String> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();
    for (f, size, _) in &print {
        if *size > MAX_SCAN_FILE {
            continue;
        }
        let Ok(file) = std::fs::File::open(f) else { continue };
        let Ok((i, s)) = scan_reader(file, SCAN_CHUNK) else { continue };
        merge_unique(&mut ids, i);
        merge_unique(&mut secrets, s);
    }
    let mut out = Vec::new();
    for id in &ids {
        for s in &secrets {
            out.push((id.clone(), s.clone()));
        }
    }
    out.truncate(MAX_PAIRS);
    *DISCOVERED.lock() = Some((print, out.clone()));
    out
}

/// Streaming scan: reads `chunk` bytes at a time and keeps `SCAN_OVERLAP` bytes of context
/// between reads so a match straddling a chunk boundary is still found (exactly once).
pub fn scan_reader<R: Read>(mut reader: R, chunk: usize) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let chunk = chunk.max(1);
    let mut ids: Vec<String> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut eof = false;
    let mut tmp = vec![0u8; 64 * 1024];
    while !eof {
        let target = buf.len() + chunk;
        while buf.len() < target {
            let want = (target - buf.len()).min(tmp.len());
            let n = reader.read(&mut tmp[..want])?;
            if n == 0 {
                eof = true;
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        // Matches starting in the last SCAN_OVERLAP bytes are left for the next round.
        let limit = if eof { buf.len() } else { buf.len().saturating_sub(SCAN_OVERLAP) };
        let (i, s, end) = scan_client_limited(&buf, limit);
        merge_unique(&mut ids, i);
        merge_unique(&mut secrets, s);
        if !eof {
            // Never cut inside a match just reported: its tail could look like a new, shorter one.
            buf.drain(..limit.max(end));
        }
    }
    Ok((ids, secrets))
}

#[cfg(test)]
pub fn scan_client(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let (i, s, _) = scan_client_limited(bytes, bytes.len());
    (i, s)
}

/// Matches whose start offset is below `limit`, plus the end offset of the last of them.
fn scan_client_limited(bytes: &[u8], limit: usize) -> (Vec<String>, Vec<String>, usize) {
    static ID_RE: LazyLock<regex::bytes::Regex> =
        LazyLock::new(|| regex::bytes::Regex::new(r"[0-9]{6,20}-[a-z0-9]{20,64}\.apps\.googleusercontent\.com").unwrap());
    static SECRET_RE: LazyLock<regex::bytes::Regex> =
        LazyLock::new(|| regex::bytes::Regex::new(r"GOCSPX-[A-Za-z0-9_\-]{28}").unwrap());
    let mut end = 0usize;
    let mut collect = |re: &regex::bytes::Regex| {
        let mut v: Vec<String> = Vec::new();
        for m in re.find_iter(bytes).take_while(|m| m.start() < limit) {
            end = end.max(m.end());
            let s = String::from_utf8_lossy(m.as_bytes()).into_owned();
            if !v.contains(&s) {
                v.push(s);
            }
        }
        v
    };
    let ids = collect(&ID_RE);
    let secrets = collect(&SECRET_RE);
    (ids, secrets, end)
}

// ---- usage --------------------------------------------------------------------------------

pub fn usage() -> Usage {
    let Some(cred) = credential() else { return err(&t("no_token")) };
    let mut force = false;
    loop {
        let token = match valid_access(&cred, force) {
            Ok(t) => t,
            Err(e) => return e,
        };
        match fetch(&token) {
            Ok(out) => return out,
            Err(e) if !force && e.get("auth_error").and_then(Value::as_bool) == Some(true) => force = true,
            Err(e) => return e,
        }
    }
}

fn call(token: &str, method: &str, body: &Value) -> std::result::Result<Value, Usage> {
    let headers = headers_from(&[
        ("Authorization", &format!("Bearer {token}")),
        ("Content-Type", "application/json"),
        ("User-Agent", "antigravity"),
    ]);
    let bytes = post_bytes(&format!("{API}{method}"), &headers, body.to_string().into_bytes())?;
    serde_json::from_slice(&bytes).map_err(|_| err(&t("connection_error")))
}

fn fetch(token: &str) -> std::result::Result<Usage, Usage> {
    let load = call(token, "loadCodeAssist", &json!({ "metadata": { "ideType": "ANTIGRAVITY" } }))?;
    let mut body = json!({});
    if let Some(p) = load.get("cloudaicompanionProject").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        body["project"] = json!(p);
    }
    let models = call(token, "fetchAvailableModels", &body)?;
    let mut out = parse_models(&models);
    if out.is_empty() {
        return Err(err(&t("key_no_data")));
    }
    if let Some(plan) = parse_plan(&load) {
        out.insert("_plan".into(), json!(plan));
    }
    out.insert("_links".into(), json!({ "dashboard_url": DASHBOARD }));
    Ok(out)
}

pub fn parse_plan(load: &Value) -> Option<String> {
    ["paidTier", "currentTier"]
        .iter()
        .filter_map(|k| load.get(*k)?.get("name")?.as_str())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(String::from)
}

const FAMILIES: &[(&str, &str)] = &[
    ("gemini_pro", "Gemini Pro"),
    ("gemini_flash", "Gemini Flash"),
    ("claude", "Claude"),
    ("gpt_oss", "GPT-OSS"),
    ("image", "Image"),
];

fn family(id: &str) -> Option<&'static str> {
    if id.contains("image") {
        Some("image")
    } else if id.starts_with("gemini") && id.contains("pro") {
        Some("gemini_pro")
    } else if id.starts_with("gemini") && id.contains("flash") {
        Some("gemini_flash")
    } else if id.starts_with("claude") {
        Some("claude")
    } else if id.starts_with("gpt") {
        Some("gpt_oss")
    } else {
        None
    }
}

/// `fetchAvailableModels` response -> one entry per model family: the most-consumed model's
/// utilization and the earliest reset among that family's models.
pub fn parse_models(json: &Value) -> Usage {
    let mut buckets: HashMap<&'static str, (f64, Option<DateTime<Utc>>, String)> = HashMap::new();
    if let Some(models) = json.get("models").and_then(Value::as_object) {
        for (name, m) in models {
            if m.get("displayName").and_then(Value::as_str).is_none() {
                continue;
            }
            let Some(q) = m.get("quotaInfo").filter(|q| q.is_object()) else { continue };
            let Some(fam) = family(&name.to_lowercase()) else { continue };
            let remaining = q.get("remainingFraction").and_then(Value::as_f64).unwrap_or(1.0);
            let used = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
            let reset_raw = q.get("resetTime").and_then(Value::as_str).unwrap_or("");
            let reset = DateTime::parse_from_rfc3339(reset_raw).ok().map(|d| d.with_timezone(&Utc));
            let b = buckets.entry(fam).or_insert((0.0, None, String::new()));
            b.0 = b.0.max(used);
            if let Some(r) = reset {
                if b.1.map(|cur| r < cur).unwrap_or(true) {
                    b.1 = Some(r);
                }
            }
        }
    }
    let mut out = Usage::new();
    for (key, label) in FAMILIES {
        if let Some((used, reset, _)) = buckets.get(key) {
            out.insert(
                (*key).into(),
                json!({
                    "utilization": (used * 10.0).round() / 10.0,
                    "resets_at": reset.map(|d| d.to_rfc3339()).unwrap_or_default(),
                    "label": label,
                }),
            );
        }
    }
    out
}

fn err(msg: &str) -> Usage {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_blob() {
        let blob = r#"{"id_token_email":"me@example.com","token":{"access_token":"ya29.a","refresh_token":"1//r","expiry":"2026-11-06T12:30:00.123456+03:00","expires_in":3599}}"#;
        let c = parse_cred(blob).unwrap();
        assert_eq!(c.email, "me@example.com");
        assert_eq!(c.access, "ya29.a");
        assert_eq!(c.refresh, "1//r");
        assert!(c.expiry.is_some());
        assert!(parse_cred("{}").is_none());
        assert!(parse_cred("not json").is_none());
        let utf16: Vec<u8> = blob.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_cred(&decode_blob(&utf16)).unwrap().email, "me@example.com");
    }

    #[test]
    fn model_families() {
        let j = json!({"models": {
            "gemini-3-pro-high": {"displayName": "Gemini 3 Pro (High)", "quotaInfo": {"remainingFraction": 0.6, "resetTime": "2026-11-06T15:00:00Z"}},
            "gemini-3-pro-low": {"displayName": "Gemini 3 Pro (Low)", "quotaInfo": {"remainingFraction": 0.8, "resetTime": "2026-11-06T14:00:00Z"}},
            "gemini-3-flash": {"displayName": "Gemini 3 Flash", "quotaInfo": {"remainingFraction": 1.0, "resetTime": "2026-11-06T16:00:00Z"}},
            "gemini-3-pro-image": {"displayName": "Gemini 3 Pro Image", "quotaInfo": {"remainingFraction": 0.25, "resetTime": "2026-11-07T00:00:00Z"}},
            "claude-sonnet-4-5": {"displayName": "Claude Sonnet 4.5", "quotaInfo": {"remainingFraction": 0.1234, "resetTime": "2026-11-06T15:00:00Z"}},
            "gpt-oss-120b-medium": {"displayName": "GPT-OSS 120B", "quotaInfo": {"remainingFraction": 0.1234, "resetTime": "2026-11-06T15:00:00Z"}},
            "chat_20706": {"quotaInfo": {"remainingFraction": 0.0}},
            "tab_flash_lite": {"displayName": "Tab", "quotaInfo": {"remainingFraction": 0.0}}
        }});
        let u = parse_models(&j);
        assert_eq!(u.len(), 5);
        assert_eq!(u["gemini_pro"]["utilization"], 40.0);
        assert_eq!(u["gemini_pro"]["label"], "Gemini Pro");
        assert_eq!(u["gemini_pro"]["resets_at"], "2026-11-06T14:00:00+00:00");
        assert_eq!(u["gemini_flash"]["utilization"], 0.0);
        assert_eq!(u["image"]["utilization"], 75.0);
        assert_eq!(u["claude"]["utilization"], 87.7);
        assert_eq!(u["gpt_oss"]["label"], "GPT-OSS");
        assert!(parse_models(&json!({})).is_empty());
    }

    #[test]
    fn plan_prefers_paid_tier() {
        let j = json!({"currentTier": {"id": "free-tier", "name": "Antigravity"}, "paidTier": {"id": "g1-pro-tier", "name": "Google AI Pro"}, "cloudaicompanionProject": "p-1"});
        assert_eq!(parse_plan(&j).unwrap(), "Google AI Pro");
        assert_eq!(parse_plan(&json!({"currentTier": {"name": "Free"}})).unwrap(), "Free");
        assert!(parse_plan(&json!({})).is_none());
    }

    #[test]
    fn scans_oauth_client() {
        // Fixtures are assembled at runtime so the source contains no literal
        // client ID/secret shape (GitHub push protection blocks those).
        let id = format!("{}-{}.apps.googleusercontent.com", "1234567890", "fakeclientidfakeclientidfakeid");
        let secret = format!("GOCSPX-{}", "FAKEFAKEFAKEFAKEFAKEFAKE0000");
        let mut bin = b"\0\0junk".to_vec();
        bin.extend_from_slice(id.as_bytes());
        bin.push(0);
        bin.extend_from_slice(secret.as_bytes());
        bin.push(0);
        let (ids, secrets) = scan_client(&bin);
        assert_eq!(ids, vec![id]);
        assert_eq!(secrets, vec![secret]);
    }

    #[test]
    fn scan_reader_finds_matches_across_chunk_boundaries() {
        let id = format!("{}-{}.apps.googleusercontent.com", "1234567890", "fakeclientidfakeclientidfakeid");
        let secret = format!("GOCSPX-{}", "FAKEFAKEFAKEFAKEFAKEFAKE0000");
        let mut data = vec![b'x'; 5000];
        data.extend_from_slice(id.as_bytes());
        data.extend_from_slice(&[0u8; 700]);
        data.extend_from_slice(secret.as_bytes());
        data.extend_from_slice(&[b'y'; 3000]);
        let total = data.len();
        // Many chunk sizes so each match lands on a boundary at least once.
        for chunk in (1..=total).step_by(37).chain([total, total + 1, 4096, 5000, 5100]) {
            let (ids, secrets) = scan_reader(std::io::Cursor::new(&data), chunk).unwrap();
            assert_eq!(ids, vec![id.to_string()], "chunk {chunk}");
            assert_eq!(secrets, vec![secret.to_string()], "chunk {chunk}");
        }
        let (ids, secrets) = scan_reader(std::io::Cursor::new(Vec::<u8>::new()), 1024).unwrap();
        assert!(ids.is_empty() && secrets.is_empty());
    }

    #[test]
    fn refresh_backoff_window() {
        assert!(backoff_active(StdDuration::from_secs(5), REJECT_BACKOFF));
        assert!(backoff_active(StdDuration::from_secs(599), REJECT_BACKOFF));
        assert!(!backoff_active(StdDuration::from_secs(601), REJECT_BACKOFF));
        assert!(backoff_active(StdDuration::from_secs(30), TRANSIENT_BACKOFF));
        assert!(!backoff_active(StdDuration::from_secs(61), TRANSIENT_BACKOFF));
    }
}
