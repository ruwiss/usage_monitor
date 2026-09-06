use crate::http::{get_json, headers_from, post_bytes, post_form};
use crate::i18n::t;
use crate::sources::{as_f64_opt, normalize_quotas};
use crate::types::{Account, Organization, Profile};
use chrono::{Duration, Utc};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const USER_URL: &str = "https://cli-chat-proxy.grok.com/v1/user?include=subscription";
const CREDITS_URL: &str = "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

fn auth_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".grok").join("auth.json")
}

fn bundle() -> Option<(Map<String, Value>, Map<String, Value>, String)> {
    let text = fs::read_to_string(auth_path()).ok()?;
    let data: Value = serde_json::from_str(&text).ok()?;
    let obj = data.as_object()?.clone();
    let key = obj.keys().next()?.clone();
    let entry = obj.get(&key)?.as_object()?.clone();
    Some((obj, entry, key))
}

fn entry() -> Option<Map<String, Value>> {
    bundle().map(|(_, e, _)| e)
}

pub fn has_account() -> bool {
    let Some(e) = entry() else { return false };
    ["key", "refresh_token", "email"].iter().any(|k| e.get(*k).and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false))
}

pub fn token() -> Option<String> {
    let e = entry()?;
    e.get("key").or_else(|| e.get("access_token")).and_then(Value::as_str).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

fn expired(e: &Map<String, Value>) -> bool {
    let Some(raw) = e.get("expires_at").and_then(Value::as_str) else { return false };
    let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&raw.replace('Z', "+00:00")) else { return false };
    Utc::now() >= expires.with_timezone(&Utc) - Duration::minutes(2)
}

fn headers(tok: &str, e: &Map<String, Value>) -> reqwest::header::HeaderMap {
    let mut pairs = vec![
        ("Authorization", format!("Bearer {tok}")),
        ("Accept", "application/json".into()),
        ("User-Agent", "grok-shell/0.2.99 (windows; x86_64)".into()),
        ("x-xai-token-auth", "xai-grok-cli".into()),
        ("x-grok-client-identifier", "grok-shell".into()),
        ("x-grok-client-version", "0.2.99".into()),
        ("x-grok-client-mode", "headless".into()),
    ];
    if let Some(email) = e.get("email").and_then(Value::as_str) {
        pairs.push(("x-email", email.into()));
    }
    if let Some(uid) = e.get("user_id").or_else(|| e.get("principal_id")).and_then(Value::as_str) {
        pairs.push(("x-userid", uid.into()));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    headers_from(&refs)
}

pub fn usage() -> Map<String, Value> {
    let e = entry().unwrap_or_default();
    let mut tok = token();
    if tok.is_none() || expired(&e) {
        tok = refresh().or(tok);
    }
    let Some(tok) = tok else { return err(&t("no_token")) };
    let mut data = request(&tok);
    if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
        if let Some(r) = refresh() {
            data = request(&r);
        }
    }
    if data.contains_key("error") {
        return data;
    }
    let mut result = normalize_quotas(data.get("quotas").and_then(Value::as_object).unwrap_or(&Map::new()));
    if let Some(plan) = data.get("plan").and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        result.insert("_plan".into(), json!(plan));
    }
    if result.is_empty() || (result.len() == 1 && result.contains_key("_plan")) {
        if let Some(msg) = data.get("message").and_then(Value::as_str) {
            return err(msg);
        }
    }
    result
}

fn request(tok: &str) -> Map<String, Value> {
    let e = entry().unwrap_or_default();
    let h = headers(tok, &e);
    let billing = get_json(BILLING_URL, &h, 12);
    if billing.contains_key("error") {
        return billing;
    }
    let user = get_json(USER_URL, &h, 12);
    let empty = Map::new();
    let user_ref = if user.contains_key("error") { &empty } else { &user };
    let mut parsed = parse_billing(&billing, user_ref);
    let plan = plan_from_token(tok);
    if !plan.is_empty() {
        parsed.insert("plan".into(), json!(plan));
    }
    if parsed.get("quotas").and_then(Value::as_object).map(|q| q.is_empty()).unwrap_or(true) {
        if let Some(grpc) = grpc_credits(tok) {
            parsed.insert("quotas".into(), json!({"Weekly SuperGrok": grpc}));
        }
    }
    parsed
}

fn refresh() -> Option<String> {
    let (mut data, entry, key) = bundle()?;
    let refresh = entry.get("refresh_token")?.as_str()?.to_string();
    let client_id = entry.get("oidc_client_id").and_then(Value::as_str).unwrap_or(CLIENT_ID);
    let payload = post_form(
        TOKEN_URL,
        &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", client_id)],
        &headers_from(&[("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")]),
    );
    let access = payload.get("access_token")?.as_str()?.to_string();
    let mut stored = entry.clone();
    stored.insert("key".into(), json!(access.clone()));
    if let Some(r) = payload.get("refresh_token") { stored.insert("refresh_token".into(), r.clone()); }
    if let Some(exp) = payload.get("expires_in").and_then(Value::as_i64) {
        stored.insert("expires_at".into(), json!((Utc::now() + Duration::seconds(exp)).to_rfc3339()));
    }
    data.insert(key, Value::Object(stored));
    let _ = fs::write(auth_path(), serde_json::to_string_pretty(&Value::Object(data)).unwrap_or_default() + "\n");
    Some(access)
}

pub fn profile() -> Option<Profile> {
    let e = entry()?;
    let plan = token().map(|t| plan_from_token(&t)).filter(|s| !s.is_empty()).unwrap_or_else(|| "Grok".into());
    Some(Profile {
        account: Account {
            email: e.get("email").and_then(Value::as_str).unwrap_or("").into(),
            uuid: e.get("user_id").and_then(Value::as_str).unwrap_or("grok").into(),
        },
        organization: Organization { organization_type: plan },
    })
}

fn plan_from_token(token: &str) -> String {
    let Some(payload) = token.split('.').nth(1) else { return String::new() };
    let mut b64 = payload.replace('-', "+").replace('_', "/");
    while b64.len() % 4 != 0 { b64.push('='); }
    use base64::Engine;
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return String::new() };
    let Ok(claims) = serde_json::from_slice::<Map<String, Value>>(&bytes) else { return String::new() };
    match claims.get("tier").and_then(Value::as_i64) {
        Some(0) => "Free",
        Some(1) => "SuperGrok",
        Some(2) => "X Basic",
        Some(3) => "X Premium",
        Some(4) => "X Premium Plus",
        Some(5) => "SuperGrok Heavy",
        Some(6) => "SuperGrok Lite",
        _ => "",
    }
    .into()
}

fn unwrap_val(value: Option<&Value>) -> Option<f64> {
    as_f64_opt(value.unwrap_or(&Value::Null))
}

fn parse_billing(billing: &Map<String, Value>, user: &Map<String, Value>) -> Map<String, Value> {
    let config = billing.get("config").and_then(Value::as_object).unwrap_or(billing);
    let period = config
        .get("billingPeriodEnd")
        .or_else(|| config.get("billing_period_end"))
        .or_else(|| config.get("currentPeriod").and_then(|p| p.get("end")))
        .or_else(|| config.get("resetAt"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut quotas = Map::new();
    if let Some(used_pct) = unwrap_val(config.get("creditUsagePercent").or_else(|| config.get("credit_usage_percent"))) {
        quotas.insert("Weekly SuperGrok".into(), json!({"used": used_pct.clamp(0.0, 100.0), "total": 100, "resetAt": period}));
    }
    let monthly = unwrap_val(config.get("monthlyLimit").or_else(|| config.get("monthly_limit")));
    let included = unwrap_val(config.get("includedUsed").or_else(|| config.get("included_used")));
    if let Some(m) = monthly.filter(|v| *v > 0.0) {
        quotas.insert("Monthly included".into(), json!({"used": included.unwrap_or(0.0), "total": m, "resetAt": period}));
    }
    if let Some(cap) = unwrap_val(config.get("onDemandCap")).filter(|v| *v > 0.0) {
        quotas.insert("On-demand".into(), json!({"used": unwrap_val(config.get("onDemandUsed")).unwrap_or(0.0), "total": cap, "resetAt": period}));
    }
    if let Some(prepaid) = unwrap_val(config.get("prepaidBalance")).filter(|v| *v > 0.0) {
        quotas.insert("Prepaid".into(), json!({"used": 0, "total": prepaid, "resetAt": ""}));
    }
    let tier = user.get("subscriptionTier").or_else(|| user.get("subscription_tier")).or_else(|| config.get("subscriptionTier")).and_then(Value::as_str).unwrap_or("");
    let plan = if tier.is_empty() {
        "Grok".into()
    } else {
        title(tier)
    };
    json!({"plan": plan, "quotas": quotas}).as_object().cloned().unwrap()
}

fn title(s: &str) -> String {
    s.replace('_', " ").replace('-', " ").split_whitespace().map(|w| {
        let mut c = w.chars();
        match c.next() { Some(f) => f.to_uppercase().to_string() + c.as_str(), None => String::new() }
    }).collect::<Vec<_>>().join(" ")
}

fn grpc_credits(tok: &str) -> Option<Value> {
    let h = headers_from(&[
        ("Authorization", &format!("Bearer {tok}")),
        ("Content-Type", "application/grpc-web+proto"),
        ("X-Grpc-Web", "1"),
        ("Accept", "application/grpc-web+proto"),
    ]);
    let raw = post_bytes(CREDITS_URL, &h, b"\x00\x00\x00\x00\x00".to_vec()).ok()?;
    let decoded = decode_grok_credits_frame(&raw)?;
    Some(json!({"used": decoded.0, "total": 100, "resetAt": decoded.1}))
}

fn decode_grok_credits_frame(raw: &[u8]) -> Option<(f64, String)> {
    let mut payload = raw;
    if raw.len() >= 5 && matches!(raw[0], 0 | 1 | 0x80 | 0x81) {
        let length = u32::from_be_bytes([raw[1], raw[2], raw[3], raw[4]]) as usize;
        if raw[0] & 0x80 == 0 && 5 + length <= raw.len() {
            payload = &raw[5..5 + length];
        }
    }
    let fields = decode_protobuf(payload)?;
    let (_, credits_bytes) = fields.get(&1).filter(|(w, _)| *w == 2)?;
    let credits = decode_protobuf(credits_bytes.as_slice())?;
    let percent = match credits.get(&1) {
        None => 0.0,
        Some((5, b)) if b.len() == 4 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 * 100.0,
        Some((1, b)) if b.len() == 8 => f64::from_le_bytes(b[..8].try_into().ok()?) * 100.0,
        _ => return None,
    };
    if percent.is_nan() || percent < 0.0 {
        return None;
    }
    let mut reset_at = String::new();
    if let Some((2, ts_buf)) = credits.get(&5) {
        if let Some(ts) = decode_protobuf(ts_buf) {
            let seconds = match ts.get(&1) { Some((0, b)) => varint_value(b).unwrap_or(0), _ => 0 };
            let nanos = match ts.get(&2) { Some((0, b)) => varint_value(b).unwrap_or(0), _ => 0 };
            let millis = seconds * 1000 + (nanos + 500_000) / 1_000_000;
            if let Some(dt) = chrono::DateTime::<Utc>::from_timestamp_millis(millis as i64) {
                reset_at = dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            }
        }
    }
    Some((percent.min(100.0), reset_at))
}



fn decode_protobuf(buf: &[u8]) -> Option<std::collections::HashMap<u32, (i32, Vec<u8>)>> {
    let mut fields = std::collections::HashMap::new();
    let mut offset = 0;
    while offset < buf.len() {
        let (value, next) = read_varint(buf, offset)?;
        offset = next;
        let number = (value >> 3) as u32;
        let wire = (value & 7) as i32;
        if number == 0 { return None; }
        match wire {
            0 => {
                let (v, n) = read_varint(buf, offset)?;
                fields.insert(number, (0, encode_varint(v)));
                offset = n;
            }
            2 => {
                let (size, start) = read_varint(buf, offset)?;
                offset = start;
                let end = offset + size as usize;
                if end > buf.len() { return None; }
                fields.insert(number, (2, buf[offset..end].to_vec()));
                offset = end;
            }
            5 => {
                if offset + 4 > buf.len() { return None; }
                fields.insert(number, (5, buf[offset..offset + 4].to_vec()));
                offset += 4;
            }
            1 => {
                if offset + 8 > buf.len() { return None; }
                fields.insert(number, (1, buf[offset..offset + 8].to_vec()));
                offset += 8;
            }
            _ => return None,
        }
    }
    Some(fields)
}

fn read_varint(buf: &[u8], mut offset: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0;
    while offset < buf.len() {
        let byte = buf[offset];
        offset += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, offset));
        }
        shift += 7;
        if shift > 70 { return None; }
    }
    None
}

fn encode_varint(v: u64) -> Vec<u8> {
    let mut n = v;
    let mut out = Vec::new();
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 { b |= 0x80; }
        out.push(b);
        if n == 0 { break; }
    }
    out
}

fn varint_value(buf: &[u8]) -> Option<u64> {
    read_varint(buf, 0).map(|(v, _)| v)
}

fn err(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn val_unwrap() {
        let v = json!({"val": 59.0});
        assert_eq!(unwrap_val(Some(&v)), Some(59.0));
    }

    #[test]
    fn decodes_credits_frame() {
        let pct = 0.42f32;
        let mut credits = vec![0x0D];
        credits.extend_from_slice(&pct.to_le_bytes());
        let mut ts = encode_varint(8);
        ts.extend(encode_varint(1_700_000_000));
        ts.extend(encode_varint(16));
        ts.extend(encode_varint(0));
        credits.push(0x2A);
        credits.extend(encode_varint(ts.len() as u64));
        credits.extend(ts);
        let mut payload = vec![0x0A];
        payload.extend(encode_varint(credits.len() as u64));
        payload.extend(credits);
        let mut raw = vec![0u8];
        raw.extend((payload.len() as u32).to_be_bytes());
        raw.extend(payload);
        let (percent, reset) = decode_grok_credits_frame(&raw).expect("decode");
        assert!((percent - 42.0).abs() < 0.01);
        assert_eq!(reset, "2023-11-14T22:13:20Z");
    }

    #[test]
    fn bad_frame_is_none() {
        assert_eq!(decode_grok_credits_frame(&[]), None);
        assert_eq!(decode_grok_credits_frame(&[1, 2, 3]), None);
    }
}
