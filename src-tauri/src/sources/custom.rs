use crate::http::{get_json, headers_from};
use crate::i18n::t;
use crate::settings::Settings;
use crate::sources::normalize_quotas;
use crate::types::{Account, Organization, Profile};
use serde_json::{json, Map, Value};
use url::Url;

pub fn usage(settings: &Settings, custom_id: &str) -> Map<String, Value> {
    let Some(custom) = settings.custom_sources.iter().find(|c| c.id == custom_id) else {
        return err(&t("no_token"));
    };
    if Url::parse(&custom.url).ok().map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()) != Some(true) {
        return err(&t_fmt_code());
    }
    let mut pairs = vec![("Accept", "application/json".to_string())];
    if !custom.token.is_empty() {
        let header = if custom.header.is_empty() { "Authorization" } else { &custom.header };
        let token = if custom.token.contains(' ') || header.to_lowercase() != "authorization" {
            custom.token.clone()
        } else {
            format!("Bearer {}", custom.token)
        };
        pairs.push((header, token));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let data = get_json(&custom.url, &headers_from(&refs), 12);
    if data.contains_key("error") {
        return data;
    }
    let quotas = data.get("quotas").and_then(Value::as_object).unwrap_or(&data);
    normalize_quotas(quotas)
}

pub fn profile(settings: &Settings, sid: &str, custom_id: &str) -> Option<Profile> {
    let custom = settings.custom_sources.iter().find(|c| c.id == custom_id)?;
    Some(Profile {
        account: Account { email: custom.name.clone(), uuid: sid.into() },
        organization: Organization { organization_type: "Custom".into() },
    })
}

fn t_fmt_code() -> String {
    crate::i18n::t_fmt("http_error", &[("code", "?")])
}

fn err(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}
