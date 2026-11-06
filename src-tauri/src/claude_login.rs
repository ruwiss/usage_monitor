//! Browser sign-in through the Claude CLI: `claude auth login` opens the system browser and the CLI
//! writes its own credentials file, which the `claude` source already reads. The app never sees or
//! stores a token here and never logs the CLI output.
use crate::i18n::t;
use crate::platform::no_window;
use crate::sources::claude;
use crate::state::AppState;
use serde_json::json;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const DONE_EVENT: &str = "claude-login://done";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_STEP: Duration = Duration::from_millis(200);

static BUSY: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Outcome {
    Success,
    Failed,
    Cancelled,
    TimedOut,
}

impl Outcome {
    /// Locale key of the user-facing message; `None` on success.
    fn error_key(self) -> Option<&'static str> {
        match self {
            Self::Success => None,
            Self::Failed => Some("claude_cli_failed"),
            Self::Cancelled => Some("claude_cli_cancelled"),
            Self::TimedOut => Some("claude_cli_timeout"),
        }
    }
}

/// Holds the single-login slot; released on drop.
struct Guard(&'static AtomicBool);

impl Guard {
    fn acquire(flag: &'static AtomicBool) -> Option<Guard> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| Guard(flag))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn login_args() -> [&'static str; 2] {
    ["auth", "login"]
}

fn logout_args() -> [&'static str; 2] {
    ["auth", "logout"]
}

pub fn signed_in() -> bool {
    claude::token().is_some()
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        // claude.cmd runs node as a grandchild; kill the whole tree.
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &child.id().to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut cmd);
        let _ = cmd.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Run the CLI with the given args, windowless and without any attached stdio; kill it on
/// timeout or cancel so no process is left behind.
fn run_cli(path: &Path, args: &[&str], timeout: Duration, cancel: &AtomicBool) -> Outcome {
    let mut cmd = Command::new(path);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_window(&mut cmd);
    let Ok(mut child) = cmd.spawn() else { return Outcome::Failed };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return if status.success() { Outcome::Success } else { Outcome::Failed },
            Ok(None) => {}
            Err(_) => {
                kill_tree(&mut child);
                return Outcome::Failed;
            }
        }
        if cancel.load(Ordering::SeqCst) {
            kill_tree(&mut child);
            return Outcome::Cancelled;
        }
        if start.elapsed() >= timeout {
            kill_tree(&mut child);
            return Outcome::TimedOut;
        }
        std::thread::sleep(POLL_STEP);
    }
}

fn cli_path() -> Result<std::path::PathBuf, String> {
    let path = crate::claude_cli::claude_path();
    if path.is_file() {
        Ok(path)
    } else {
        Err(t("claude_cli_missing"))
    }
}

/// Start `claude auth login` in the background. Returns at once; the result arrives as
/// the `claude-login://done` event with `{ok, error}`.
pub fn start(app: &AppHandle, state: Arc<AppState>) -> Result<(), String> {
    let path = cli_path()?;
    let guard = Guard::acquire(&BUSY).ok_or_else(|| t("claude_cli_busy"))?;
    CANCEL.store(false, Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        let _guard = guard;
        let mut outcome = run_cli(&path, &login_args(), LOGIN_TIMEOUT, &CANCEL);
        if outcome == Outcome::Success && !signed_in() {
            outcome = Outcome::Failed;
        }
        if outcome == Outcome::Success {
            crate::sources::invalidate_caches();
            let _ = crate::settings::set_source_visible(&state, "claude".into(), true);
            crate::sources::select_source(&state, "claude".into());
            crate::tray::refresh(&app, &state);
            crate::poll::force_update(app.clone(), state);
        }
        let error = outcome.error_key().map(t);
        let _ = app.emit(DONE_EVENT, json!({ "ok": outcome == Outcome::Success, "error": error }));
    });
    Ok(())
}

/// Abort a running login (kills the CLI process).
pub fn cancel() {
    if BUSY.load(Ordering::SeqCst) {
        CANCEL.store(true, Ordering::SeqCst);
    }
}

/// Run `claude auth logout` (blocking, short timeout).
pub fn sign_out() -> Result<(), String> {
    let path = cli_path()?;
    let _guard = Guard::acquire(&BUSY).ok_or_else(|| t("claude_cli_busy"))?;
    let never = AtomicBool::new(false);
    match run_cli(&path, &logout_args(), LOGOUT_TIMEOUT, &never) {
        Outcome::Success => {
            crate::sources::invalidate_caches();
            Ok(())
        }
        other => Err(t(other.error_key().unwrap_or("claude_cli_failed"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_fixed() {
        assert_eq!(login_args(), ["auth", "login"]);
        assert_eq!(logout_args(), ["auth", "logout"]);
    }

    #[test]
    fn guard_allows_one_holder() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let first = Guard::acquire(&FLAG);
        assert!(first.is_some());
        assert!(Guard::acquire(&FLAG).is_none());
        drop(first);
        assert!(Guard::acquire(&FLAG).is_some());
    }

    #[test]
    fn outcome_error_keys() {
        assert_eq!(Outcome::Success.error_key(), None);
        assert_eq!(Outcome::TimedOut.error_key(), Some("claude_cli_timeout"));
        assert_eq!(Outcome::Cancelled.error_key(), Some("claude_cli_cancelled"));
        assert_eq!(Outcome::Failed.error_key(), Some("claude_cli_failed"));
    }

    #[test]
    fn missing_program_fails_without_hanging() {
        let cancel = AtomicBool::new(false);
        let out = run_cli(Path::new("definitely-not-a-real-program-xyz"), &["auth"], Duration::from_secs(2), &cancel);
        assert_eq!(out, Outcome::Failed);
    }

    #[test]
    fn cancel_flag_stops_a_long_process() {
        let cancel = AtomicBool::new(true);
        #[cfg(windows)]
        let (prog, args): (&str, &[&str]) = ("ping", &["-n", "30", "127.0.0.1"]);
        #[cfg(not(windows))]
        let (prog, args): (&str, &[&str]) = ("sleep", &["30"]);
        let start = Instant::now();
        let out = run_cli(Path::new(prog), args, Duration::from_secs(20), &cancel);
        assert_eq!(out, Outcome::Cancelled);
        assert!(start.elapsed() < Duration::from_secs(10));
    }
}
