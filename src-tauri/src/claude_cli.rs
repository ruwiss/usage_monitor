use crate::platform::no_window;
use std::process::Command;
use std::path::PathBuf;


pub fn claude_path() -> PathBuf {
    if let Ok(found) = which::which("claude") {
        let ext = found.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ext == "ps1" {
            for alt_ext in ["cmd", "exe"] {
                let alt = found.with_extension(alt_ext);
                if alt.is_file() {
                    return alt;
                }
            }
        }
        return found;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        for name in ["claude.cmd", "claude.exe"] {
            let candidate = PathBuf::from(&appdata).join("npm").join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    for candidate in [
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
        home.join(".local").join("bin").join("claude"),
        home.join(".local").join("bin").join("claude.exe"),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    home.join(".local").join("bin").join(if cfg!(windows) { "claude.exe" } else { "claude" })
}

pub fn cli_version(path: &PathBuf) -> String {
    if !path.is_file() {
        return String::new();
    }
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    no_window(&mut cmd);
    match cmd.output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.split_whitespace().next().unwrap_or("").to_string()
        }
        _ => String::new(),
    }
}
