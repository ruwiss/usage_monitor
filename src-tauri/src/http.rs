use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Map, Value};
use std::time::Duration;

fn client(timeout_secs: u64) -> Client {
    client_with(timeout_secs, false)
}

fn client_with(timeout_secs: u64, accept_invalid: bool) -> Client {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .danger_accept_invalid_certs(accept_invalid)
        .build()
        .expect("reqwest client")
}

pub fn get_json(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Map<String, Value> {
    match get_json_value(url, headers, timeout_secs) {
        Ok(Value::Object(map)) => map,
        Ok(_) => error_map(&crate::i18n::t("connection_error")),
        Err(err) => err,
    }
}

pub fn get_json_value(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Result<Value, Map<String, Value>> {
    fetch_json_value(url, headers, timeout_secs, false)
}

pub fn get_json_value_insecure(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Result<Value, Map<String, Value>> {
    fetch_json_value(url, headers, timeout_secs, true)
}

fn fetch_json_value(url: &str, headers: &HeaderMap, timeout_secs: u64, accept_invalid: bool) -> Result<Value, Map<String, Value>> {
    match client_with(timeout_secs, accept_invalid).get(url).headers(headers.clone()).send() {
        Ok(resp) => success_or_status(resp),
        Err(err) => Err(map_error(err)),
    }
}

pub fn post_form(url: &str, form: &[(&str, &str)], headers: &HeaderMap) -> Map<String, Value> {
    match client(12).post(url).headers(headers.clone()).form(form).send() {
        Ok(resp) => map_response(resp),
        Err(err) => map_error(err),
    }
}

pub fn post_bytes(url: &str, headers: &HeaderMap, body: Vec<u8>) -> Result<Vec<u8>, Map<String, Value>> {
    match client(12).post(url).headers(headers.clone()).body(body).send() {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(map_response(resp));
            }
            match resp.bytes() {
                Ok(b) => Ok(b.to_vec()),
                Err(e) => {
                    let mut m = Map::new();
                    m.insert("error".into(), json!(e.to_string()));
                    Err(m)
                }
            }
        }
        Err(err) => Err(map_error(err)),
    }
}

pub fn headers_from(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
            map.insert(name, val);
        }
    }
    map
}

fn success_or_status(resp: reqwest::blocking::Response) -> Result<Value, Map<String, Value>> {
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok());
    let body = resp.text().unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str::<Value>(&body).map_err(|_| error_map(&crate::i18n::t("connection_error")));
    }
    Err(status_map(status.as_u16(), &body, retry_after))
}

fn map_response(resp: reqwest::blocking::Response) -> Map<String, Value> {
    match success_or_status(resp) {
        Ok(Value::Object(map)) => map,
        Ok(_) => error_map(&crate::i18n::t("connection_error")),
        Err(err) => err,
    }
}

fn status_map(code: u16, body: &str, retry_after: Option<i64>) -> Map<String, Value> {
    let extra_msg = server_message(body);
    let mut map = Map::new();
    if let Some(msg) = extra_msg {
        map.insert("server_message".into(), json!(msg));
    }
    if code == 401 {
        map.insert("error".into(), json!(crate::i18n::t("auth_expired")));
        map.insert("auth_error".into(), json!(true));
        return map;
    }
    if code == 429 {
        if let Some(ra) = retry_after.filter(|v| *v >= 0) {
            map.insert("retry_after".into(), json!(ra));
        }
        map.insert("error".into(), json!(crate::i18n::t_fmt("http_error", &[("code", "429")])));
        map.insert("rate_limited".into(), json!(true));
        return map;
    }
    if (500..600).contains(&code) {
        map.insert(
            "error".into(),
            json!(crate::i18n::t_fmt("server_error", &[("code", &code.to_string())])),
        );
        return map;
    }
    let code_s = if code == 0 { "?".into() } else { code.to_string() };
    map.insert("error".into(), json!(crate::i18n::t_fmt("http_error", &[("code", &code_s)])));
    map
}

fn map_error(err: reqwest::Error) -> Map<String, Value> {
    let msg = err.to_string().to_lowercase();
    if msg.contains("certificate") || msg.contains("tls") || msg.contains("ssl") || msg.contains("unknownissuer") {
        return error_map(&crate::i18n::t("certificate_error"));
    }
    if err.is_timeout() || err.is_connect() {
        return error_map(&crate::i18n::t("connection_error"));
    }
    if let Some(status) = err.status() {
        return status_map(status.as_u16(), "", None);
    }
    error_map(&crate::i18n::t("connection_error"))
}

fn error_map(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

fn server_message(body: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(body).ok()?;
    let obj = payload.as_object()?;
    match obj.get("error") {
        Some(Value::Object(e)) => e
            .get("message")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        Some(Value::String(s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        _ => obj
            .get("message")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}
