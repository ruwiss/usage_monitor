use sha1::{Digest, Sha1};
use std::env;
use std::path::{Path, PathBuf};

pub fn parse_config_dir(argv: &[String]) -> Option<String> {
    let mut value = None;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if let Some(rest) = arg.strip_prefix("--config-dir=") {
            value = Some(rest.to_string());
        } else if arg == "--config-dir" && i + 1 < argv.len() {
            value = Some(argv[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    let mut value = value?;
    value = value.trim().trim_matches('"').trim_end_matches(['\\', '/']).to_string();
    if value.is_empty() {
        return None;
    }
    if value.len() == 2 && value.as_bytes()[0].is_ascii_alphabetic() && value.as_bytes()[1] == b':' {
        value.push('\\');
    }
    let expanded = expand_path(&value);
    Some(expanded)
}

fn expand_path(value: &str) -> String {
    let mut s = value.to_string();
    if s.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            s = s.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    // Expand %VAR% on Windows and $VAR elsewhere in a simple pass.
    if let Ok(profile) = env::var("USERPROFILE") {
        s = s.replace("%USERPROFILE%", &profile);
    }
    if let Ok(home) = env::var("HOME") {
        s = s.replace("$HOME", &home);
    }
    PathBuf::from(s).to_string_lossy().into_owned()
}

pub fn effective_config_dir() -> PathBuf {
    if let Ok(custom) = env::var("CLAUDE_CONFIG_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claude")
}

pub fn is_default_config_dir() -> bool {
    let default = dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claude");
    normcase(&effective_config_dir()) == normcase(&default)
}

pub fn config_dir_suffix() -> String {
    if is_default_config_dir() {
        return String::new();
    }
    let normalized = normcase(&effective_config_dir());
    let mut hasher = Sha1::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let hex = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("_{}", &hex[..12])
}

fn normcase(path: &Path) -> String {
    let s = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_suffix_empty() {
        env::remove_var("CLAUDE_CONFIG_DIR");
        assert_eq!(config_dir_suffix(), "");
    }

    #[test]
    fn parse_equals_and_space() {
        assert_eq!(
            parse_config_dir(&["--config-dir=/tmp/x".into()]),
            Some("/tmp/x".into())
        );
        assert_eq!(
            parse_config_dir(&["--config-dir".into(), "/tmp/y".into()]),
            Some("/tmp/y".into())
        );
    }
}
