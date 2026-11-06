use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;
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

/// The language code the app should use for `code`: the code itself when set,
/// otherwise the OS locale mapped to a shipped file (defaulting to `en`).
pub fn resolved_language(code: &str) -> String {
    if !code.trim().is_empty() {
        return code.trim().to_string();
    }
    let dirs = locale_dirs();
    detect_lang_code(&sys_locale(), &dirs)
}

pub fn t_fmt(key: &str, pairs: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (k, v) in pairs {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Language codes that have a shipped locale file, paired with their English name.
/// Sorted by name so the UI list is stable.
pub fn available_languages() -> Vec<(String, String)> {
    let dirs = locale_dirs();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for dir in &dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(code) = name.strip_suffix(".json") else { continue };
            if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                continue;
            }
            seen.insert(code.to_string());
        }
    }
    let mut out: Vec<(String, String)> =
        seen.into_iter().map(|code| (code.clone(), language_name(&code).to_string())).collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// True when `code` is a valid setting: empty means "follow the system", and any
/// other value must have a shipped locale file.
pub fn language_available(code: &str) -> bool {
    let code = code.trim();
    code.is_empty() || code == "en" || locale_exists(&locale_dirs(), code)
}

/// The full translation map for one language code. An empty code follows the OS
/// locale; an unknown code falls back to the built-in (startup) map.
pub fn translations_for(code: &str) -> Map<String, Value> {
    let resolved = resolved_language(code);
    read_locale(&locale_dirs(), &resolved).unwrap_or_else(|| TRANSLATIONS.clone())
}

/// English display name for a shipped language code.
fn language_name(code: &str) -> &'static str {
    match code {
        "en" => "English",
        "de" => "Deutsch",
        "es" => "Español",
        "fr" => "Français",
        "it" => "Italiano",
        "ja" => "日本語",
        "ko" => "한국어",
        "pt-BR" => "Português (Brasil)",
        "tr" => "Türkçe",
        "uk" => "Українська",
        "hi" => "हिन्दी",
        "id" => "Bahasa Indonesia",
        "zh-CN" => "简体中文",
        "zh-TW" => "繁體中文",
        _ => "Other",
    }
}

fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locale"));
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("locale"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("locale"));
            // macOS app bundle: Contents/MacOS/../Resources/locale
            dirs.push(parent.join("../Resources/locale"));
        }
    }
    if let Some(bundle) = crate::platform::macos_bundle_dir() {
        dirs.push(bundle.join("Contents/Resources/locale"));
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

    #[test]
    fn shipped_languages_are_listed_and_named() {
        let langs = available_languages();
        let codes: Vec<&str> = langs.iter().map(|(c, _)| c.as_str()).collect();
        for code in ["en", "de", "es", "fr", "it", "ja", "ko", "pt-BR", "tr", "uk", "hi", "id", "zh-CN", "zh-TW"] {
            assert!(codes.contains(&code), "missing language {code}");
        }
        assert_eq!(langs.iter().find(|(c, _)| c == "en").unwrap().1, "English");
        // Sorted by display name.
        let mut sorted = langs.clone();
        sorted.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(langs, sorted);
    }

    #[test]
    fn language_availability_and_translations() {
        // Empty means "follow the system" and is always allowed.
        assert!(language_available(""));
        assert!(language_available("en"));
        assert!(language_available(" de "));
        assert!(!language_available("xx-nope"));
        assert!(!language_available(".."));
        // A specific known code reads that file.
        let de = translations_for("de");
        assert_eq!(de.get("email").and_then(Value::as_str), Some("E-Mail"));
        // An empty code resolves to something shipped (never empty), and always
        // exposes the popup title.
        let auto = translations_for("");
        assert!(auto.contains_key("popup_title"));
        // Unknown codes still yield the built-in map with the popup title.
        let fallback = translations_for("xx-nope");
        assert!(fallback.contains_key("popup_title"));
    }

    #[test]
    fn resolved_language_prefers_explicit_code() {
        assert_eq!(resolved_language("de"), "de");
        assert_eq!(resolved_language(" tr "), "tr");
        // Empty resolves to a shipped code, never an empty string.
        assert!(!resolved_language("").is_empty());
    }
}
