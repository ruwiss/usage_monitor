use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use std::time::Duration;

/// Redirect policy shared by every client: a redirect is followed only when it stays on the
/// same scheme, host and port, so credentials and custom headers are never forwarded to
/// another origin.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        let follow = attempt.previous().last().map_or(false, |prev| same_origin(prev, attempt.url()));
        if follow {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// True for hosts on the local machine or a private network, where a self-signed
/// certificate is normal (9Router, OmniRoute, ...): localhost, `*.localhost`, `*.local`,
/// 127.0.0.0/8, ::1, RFC 1918 IPv4 ranges and IPv6 unique-local addresses.
pub fn is_local_host(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local")
        }
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        None => false,
    }
}

/// Shared client for the secure (certificate-validating) path: connection pool and TLS
/// config are built once. Per-request timeouts are applied on the request builder.
fn secure_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| Client::builder().redirect(redirect_policy()).build().expect("reqwest client"))
}

/// Client that skips certificate verification. Only ever used for local/private hosts.
fn local_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .redirect(redirect_policy())
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest client")
    })
}

fn request_timeout(timeout_secs: u64) -> Duration {
    Duration::from_secs(timeout_secs.max(1))
}

pub fn get_json(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Map<String, Value> {
    match get_json_value(url, headers, timeout_secs) {
        Ok(Value::Object(map)) => map,
        Ok(_) => error_map(&crate::i18n::t("connection_error")),
        Err(err) => err,
    }
}

pub fn get_json_value(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Result<Value, Map<String, Value>> {
    fetch_json_value(url, headers, timeout_secs)
}

/// GET for user-configured URLs. Certificate verification is relaxed only for local or
/// private-network hosts; every other host goes through the verifying client.
pub fn get_json_value_insecure(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Result<Value, Map<String, Value>> {
    let client = if is_local_host(url) { local_client() } else { secure_client() };
    match client.get(url).timeout(request_timeout(timeout_secs)).headers(headers.clone()).send() {
        Ok(resp) => success_or_status(resp),
        Err(err) => Err(map_error(err)),
    }
}

fn fetch_json_value(url: &str, headers: &HeaderMap, timeout_secs: u64) -> Result<Value, Map<String, Value>> {
    match secure_client().get(url).timeout(request_timeout(timeout_secs)).headers(headers.clone()).send() {
        Ok(resp) => success_or_status(resp),
        Err(err) => Err(map_error(err)),
    }
}

pub fn post_form(url: &str, form: &[(&str, &str)], headers: &HeaderMap) -> Map<String, Value> {
    match secure_client().post(url).timeout(request_timeout(12)).headers(headers.clone()).form(form).send() {
        Ok(resp) => map_response(resp),
        Err(err) => map_error(err),
    }
}

pub fn post_bytes(url: &str, headers: &HeaderMap, body: Vec<u8>) -> Result<Vec<u8>, Map<String, Value>> {
    match secure_client().post(url).timeout(request_timeout(12)).headers(headers.clone()).body(body).send() {
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
    map.insert("http_status".into(), json!(code));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts_are_detected() {
        for u in [
            "http://localhost:20128/api",
            "https://LOCALHOST/x",
            "https://localhost./x",
            "https://router.local/x",
            "https://app.localhost/x",
            "https://127.0.0.1:8443/",
            "https://127.5.6.7/",
            "https://[::1]:8443/",
            "https://10.0.0.1/",
            "https://10.255.255.255/",
            "https://192.168.1.10/",
            "https://172.16.0.1/",
            "https://172.31.255.255/",
            "https://[fd00::1]/",
        ] {
            assert!(is_local_host(u), "{u}");
        }
    }

    #[test]
    fn public_and_lookalike_hosts_are_not_local() {
        for u in [
            "https://api.openai.com/",
            "https://localhost.evil.com/",
            "https://evil.com/localhost",
            "https://notlocal/",
            "https://foo.local.evil.com/",
            "https://172.15.255.255/",
            "https://172.32.0.1/",
            "https://11.0.0.1/",
            "https://192.169.0.1/",
            "https://8.8.8.8/",
            "https://[2001:db8::1]/",
            "https://localhost@evil.com/",
            "not a url",
            "",
        ] {
            assert!(!is_local_host(u), "{u}");
        }
    }

    #[test]
    fn same_origin_requires_scheme_host_and_port() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(same_origin(&u("https://a.com/x"), &u("https://A.com/y")));
        assert!(same_origin(&u("https://a.com/x"), &u("https://a.com:443/y")));
        assert!(!same_origin(&u("https://a.com/x"), &u("https://b.com/x")));
        assert!(!same_origin(&u("https://a.com/x"), &u("http://a.com/x")));
        assert!(!same_origin(&u("https://a.com/x"), &u("https://a.com:8443/x")));
        assert!(!same_origin(&u("https://a.com/x"), &u("https://a.com.evil.com/x")));
    }
}
