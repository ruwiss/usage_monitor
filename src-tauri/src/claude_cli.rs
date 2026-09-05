use crate::platform::no_window;
use crate::types::RefreshResult;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(60);

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
pub fn refresh_token() -> RefreshResult {
    let path = claude_path();
    if !path.is_file() {
        return RefreshResult {
            success: false,
            error: "claude CLI not found".into(),
            ..Default::default()
        };
    }
    let old = cli_version(&path);
    let mut cmd = Command::new(&path);
    cmd.arg("update").stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RefreshResult { success: false, error: e.to_string(), ..Default::default() },
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let new = cli_version(&path);
                return RefreshResult {
                    success: true,
                    updated: !old.is_empty() && !new.is_empty() && old != new,
                    old_version: old,
                    new_version: new,
                    error: String::new(),
                };
            }
            Ok(Some(_)) => {
                return RefreshResult { success: false, error: "claude update failed".into(), ..Default::default() };
            }
            Ok(None) if start.elapsed() < UPDATE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return RefreshResult { success: false, error: "Timeout".into(), ..Default::default() };
            }
            Err(e) => return RefreshResult { success: false, error: e.to_string(), ..Default::default() },
        }
    }
}
