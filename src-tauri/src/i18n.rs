use serde_json::{Map, Value};
use std::fs;
use std::path::{PathBuf};
use std::sync::LazyLock;

static TRANSLATIONS: LazyLock<Map<String, Value>> = LazyLock::new(load);



pub fn load() -> Map<String, Value> {
    load_from(&locale_dirs())
}

pub fn t(key: &str) -> String {
    match TRANSLATIONS.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => key.to_string(),
    }
}

pub fn t_fmt(key: &str, pairs: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (k, v) in pairs {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locale"));
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("locale"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("locale"));
        }
    }
    dirs
}

fn load_from(dirs: &[PathBuf]) -> Map<String, Value> {
    let language = crate::settings::language_override();
    if !language.is_empty() {
        if let Some(map) = read_locale(dirs, &language) {
            return map;
        }
    }
    let sys = sys_locale();
    let code = detect_lang_code(&sys, dirs);
    read_locale(dirs, &code).or_else(|| read_locale(dirs, "en")).unwrap_or_default()
}

fn read_locale(dirs: &[PathBuf], code: &str) -> Option<Map<String, Value>> {
    for dir in dirs {
        let path = dir.join(format!("{code}.json"));
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(Value::Object(map)) = serde_json::from_str(&text) {
                return Some(map);
            }
        }
    }
    None
}

fn locale_exists(dirs: &[PathBuf], code: &str) -> bool {
    dirs.iter().any(|d| d.join(format!("{code}.json")).is_file())
}

pub fn detect_lang_code(lang: &str, dirs: &[PathBuf]) -> String {
    let normalized = normalize_locale(lang);
    let parts: Vec<&str> = normalized.split('_').collect();
    let mut base = parts.first().copied().unwrap_or("en").to_lowercase();
    let rest = parts.get(1).copied().unwrap_or("").to_string();
    let last = parts.last().copied().unwrap_or("").to_string();

    if base.len() > 3 {
        base = normalize_locale(&base).split(['.', '_']).next().unwrap_or("en").to_lowercase();
    }

    let mut region_override = String::new();
    if base == "ukrainian" {
        base = "uk".into();
    } else if base == "hindi" {
        base = "hi".into();
    } else if base == "indonesian" {
        base = "id".into();
    } else if base.starts_with("chinese") {
        let original_region = parts.get(1).copied().unwrap_or("");
        let traditional = base.contains("traditional")
            || matches!(original_region, "Taiwan" | "Hong Kong SAR" | "Macao SAR");
        base = "zh".into();
        region_override = if traditional { "TW".into() } else { "CN".into() };
    } else if let Some(iso) = windows_language_iso(&base) {
        base = iso.into();
    }

    if region_override.is_empty() {
        match rest.to_ascii_lowercase().as_str() {
            "hant" => {
                base = "zh".into();
                region_override = "TW".into();
            }
            "hans" => {
                base = "zh".into();
                region_override = "CN".into();
            }
            _ => {}
        }
    }

    let region = if !region_override.is_empty() {
        region_override
    } else if base.len() <= 3 {
        if last.len() >= 2 && last.len() <= 3 {
            last
        } else {
            rest
        }
    } else {
        String::new()
    };

    if !region.is_empty() {
        let tagged = format!("{base}-{region}");
        if locale_exists(dirs, &tagged) {
            return tagged;
        }
    }
    if locale_exists(dirs, &base) {
        return base;
    }
    "en".into()
}

fn windows_language_iso(name: &str) -> Option<&'static str> {
    Some(match name {
        "german" => "de",
        "spanish" => "es",
        "french" => "fr",
        "italian" => "it",
        "japanese" => "ja",
        "korean" => "ko",
        "turkish" => "tr",
        "portuguese" => "pt",
        "english" => "en",
        "dutch" => "nl",
        "russian" => "ru",
        "polish" => "pl",
        "ukrainian" => "uk",
        "hindi" => "hi",
        "indonesian" => "id",
        _ => return None,
    })
}

fn normalize_locale(lang: &str) -> String {
    lang.split('.').next().unwrap_or(lang).replace('-', "_")
}

fn sys_locale() -> String {
    let os = crate::platform::os_locale();
    if !os.is_empty() {
        return os;
    }
    std::env::var("LANG")
        .ok()
        .or_else(|| std::env::var("LC_ALL").ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_dirs() -> (std::path::PathBuf, [std::path::PathBuf; 1]) {
        let tmp = std::env::temp_dir().join(format!(
            "um-locale-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&tmp);
        for name in [
            "en.json", "de.json", "es.json", "tr.json", "zh-CN.json", "zh-TW.json", "pt-BR.json",
        ] {
            fs::write(tmp.join(name), "{}").unwrap();
        }
        let dirs = [tmp.clone()];
        (tmp, dirs)
    }

    #[test]
    fn detect_zh_variants() {
        let (tmp, dirs) = fixture_dirs();
        assert_eq!(detect_lang_code("Chinese (Simplified)_China", &dirs), "zh-CN");
        assert_eq!(detect_lang_code("Chinese (Traditional)_Taiwan", &dirs), "zh-TW");
        assert_eq!(detect_lang_code("zh-Hant-TW", &dirs), "zh-TW");
        assert_eq!(detect_lang_code("zh-Hans-CN", &dirs), "zh-CN");
        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    fn detect_windows_english_names() {
        let (tmp, dirs) = fixture_dirs();
        assert_eq!(detect_lang_code("German_Germany", &dirs), "de");
        assert_eq!(detect_lang_code("de-DE", &dirs), "de");
        assert_eq!(detect_lang_code("Turkish_Turkey", &dirs), "tr");
        assert_eq!(detect_lang_code("tr-TR", &dirs), "tr");
        assert_eq!(detect_lang_code("Spanish_Mexico", &dirs), "es");
        let _ = fs::remove_dir_all(tmp);
    }

    #[cfg(windows)]
    #[test]
    fn windows_os_locale_maps_to_shipped_file() {
        let loc = crate::platform::os_locale();
        assert!(!loc.is_empty());
        let dirs = [PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locale")];
        let code = detect_lang_code(&loc, &dirs);
        assert!(
            locale_exists(&dirs, &code),
            "locale {loc:?} mapped to {code} but file missing"
        );
    }
}
