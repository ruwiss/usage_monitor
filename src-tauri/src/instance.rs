use std::env;
use std::path::{Path, PathBuf};

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
    fn default_dir_without_env() {
        env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(is_default_config_dir());
    }
}
