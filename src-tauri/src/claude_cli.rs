use crate::platform::no_window;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};


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
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    no_window(&mut cmd);
    let Ok(mut child) = cmd.spawn() else { return String::new() };
    // Read stdout on a helper thread so a hung CLI cannot block the caller past the deadline.
    let mut stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.take(4096).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status_ok = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    };
    if !status_ok {
        return String::new();
    }
    let buf = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    String::from_utf8_lossy(&buf).split_whitespace().next().unwrap_or("").to_string()
}
