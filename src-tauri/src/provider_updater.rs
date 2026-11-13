//! Updates local providers independently of usage polling and OAuth failures.
//! Unknown installations are skipped rather than installing a second copy.
use crate::{alerts, i18n, platform, state::AppState};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod installers;

const INTERVAL: i64 = 24 * 60 * 60;
const MAX_OUTPUT: u64 = 4 * 1024 * 1024;

pub fn spawn(app: tauri::AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || {
        let mut last_attempt = 0;
        while *state.running.lock() {
            check_once(&app, &state, &mut last_attempt);
            for _ in 0..60 {
                if !*state.running.lock() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    });
}

fn check_once(app: &tauri::AppHandle, state: &Arc<AppState>, last_attempt: &mut i64) {
    let settings = state.settings.lock().clone();
    let now = chrono::Utc::now().timestamp();
    if settings.provider_auto_update
        && due(now, settings.last_provider_update_check)
        && due(now, *last_attempt)
    {
        *last_attempt = now;
        // Persist before network work, including failures, to avoid restart retry loops.
        let saved = state
            .settings
            .lock()
            .save_setting("last_provider_update_check", json!(now));
        if let Err(err) = saved {
            eprintln!("provider updater: could not save check time: {err}");
        } else {
            let sources = crate::sources::list_sources(&settings);
            for (id, bin, label) in [
                ("claude", "claude", "Claude Code"),
                ("codex", "codex", "Codex CLI"),
                ("grok", "grok", "Grok CLI"),
                ("omp", "omp", "OMP"),
                ("antigravity", "antigravity", "Antigravity IDE"),
                ("antigravity", "agy", "Antigravity CLI"),
            ] {
                if !*state.running.lock() || !state.settings.lock().provider_auto_update {
                    break;
                }
                if !sources
                    .iter()
                    .any(|s| s.id == id || (id == "omp" && s.id.starts_with("omp:")))
                {
                    continue;
                }
                let path = match id {
                    "claude" => Some(crate::claude_cli::claude_path()),
                    "omp" => crate::sources::omp::omp_bin(),
                    "antigravity" => installers::antigravity_path(bin),
                    _ => which::which(bin).ok(),
                };
                let Some(path) = path.filter(|p| p.is_file()) else {
                    continue;
                };
                let result = update(&path, bin, state);
                let notify = state.settings.lock().notify_provider_update;
                match result {
                    Ok(Some((old, new))) if notify => {
                        let message = i18n::t_fmt(
                            "notify_provider_update",
                            &[("provider", label), ("old", &old), ("new", &new)],
                        );
                        alerts::notify_or_defer(
                            &app,
                            &state,
                            &format!("provider_update_{id}"),
                            &message,
                            &i18n::t("notify_update_title"),
                        );
                    }
                    Err(err) => eprintln!("provider updater ({label}): {err}"),
                    _ => {}
                }
            }
        }
    }
}

fn due(now: i64, last: i64) -> bool {
    last <= 0 || now < last || now.saturating_sub(last) >= INTERVAL
}

struct Npm {
    node: PathBuf,
    cli: PathBuf,
    prefix: PathBuf,
    root: PathBuf,
}

impl Npm {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.node);
        cmd.arg(&self.cli).arg("--prefix").arg(&self.prefix);
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(home);
        }
        cmd
    }

    fn discover() -> Option<Self> {
        let npm = which::which("npm").ok()?;
        let dir = npm.parent()?;
        let cli = if cfg!(windows) {
            dir.join("node_modules/npm/bin/npm-cli.js")
        } else {
            fs::canonicalize(&npm).ok()?
        };
        if !cli.is_file() {
            return None;
        }
        let adjacent = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
        let node = if adjacent.is_file() {
            adjacent
        } else {
            which::which("node").ok()?
        };
        let mut cmd = Command::new(&node);
        cmd.arg(&cli).args(["prefix", "--global"]);
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(home);
        }
        let prefix = PathBuf::from(run(cmd, Duration::from_secs(15)).ok()?.trim());
        if !prefix.is_absolute() {
            return None;
        }
        let root = if cfg!(windows) {
            prefix.join("node_modules")
        } else {
            prefix.join("lib/node_modules")
        };
        Some(Self {
            node,
            cli,
            prefix,
            root,
        })
    }

    fn package_for(&self, executable: &Path, bin: &str) -> Option<PathBuf> {
        // Only packages owned by the active npm prefix are eligible. pnpm/Bun,
        // project-local node_modules and another Node installation do not match.
        let bin_dir = if cfg!(windows) {
            self.prefix.clone()
        } else {
            self.prefix.join("bin")
        };
        if executable.parent()?.canonicalize().ok()? != bin_dir.canonicalize().ok()? {
            return None;
        }
        for entry in fs::read_dir(&self.root).ok()?.flatten() {
            let path = entry.path();
            let candidates = if entry.file_name().to_string_lossy().starts_with('@') {
                fs::read_dir(path)
                    .ok()?
                    .flatten()
                    .map(|e| e.path())
                    .collect::<Vec<_>>()
            } else {
                vec![path]
            };
            for package in candidates {
                if package.is_symlink() {
                    continue;
                } // npm link / external checkout
                let manifest: Value = match read_manifest(&package) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let Some(name) = manifest["name"].as_str().filter(|n| valid_package_name(n)) else {
                    continue;
                };
                if self.root.join(name).canonicalize().ok() != package.canonicalize().ok() {
                    continue;
                }
                let Some(target) = package_target(&package, &manifest, bin) else {
                    continue;
                };
                let matches = if cfg!(windows) {
                    // npm's cmd/PowerShell shims contain this exact package-relative target.
                    let relative = target
                        .strip_prefix(&self.prefix)
                        .ok()?
                        .to_string_lossy()
                        .replace('\\', "/");
                    read_limited(executable)
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .is_some_and(|s| s.replace('\\', "/").contains(&format!("/{relative}")))
                } else {
                    target
                        .canonicalize()
                        .ok()
                        .zip(executable.canonicalize().ok())
                        .is_some_and(|(a, b)| a == b)
                };
                if matches {
                    return Some(package);
                }
            }
        }
        None
    }
}

fn read_manifest(package: &Path) -> Result<Value, String> {
    serde_json::from_slice(&read_limited(&package.join("package.json"))?).map_err(|e| e.to_string())
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err("command output or metadata exceeds size limit".into());
    }
    Ok(bytes)
}

fn bin_target<'a>(manifest: &'a Value, bin: &str) -> Option<&'a str> {
    match manifest.get("bin")? {
        Value::Object(bins) => bins.get(bin)?.as_str(),
        Value::String(target) if manifest.get("name")?.as_str()?.rsplit('/').next()? == bin => {
            Some(target)
        }
        _ => None,
    }
}

fn stable_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let result = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() {
        None
    } else {
        Some(result)
    }
}

fn valid_package_name(name: &str) -> bool {
    fn part(part: &str) -> bool {
        part.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    }
    if let Some(scoped) = name.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, name)| part(scope) && part(name))
    } else {
        part(name)
    }
}

fn package_target(package: &Path, manifest: &Value, bin: &str) -> Option<PathBuf> {
    let target = Path::new(bin_target(manifest, bin)?);
    if target.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return None;
    }
    let target: PathBuf = package.join(target).components().collect();
    if !target.is_file()
        || !target
            .canonicalize()
            .ok()?
            .starts_with(package.canonicalize().ok()?)
    {
        return None;
    }
    Some(target)
}

fn update(path: &Path, bin: &str, state: &AppState) -> Result<Option<(String, String)>, String> {
    if let Some(npm) = Npm::discover() {
        if let Some(package) = npm.package_for(path, bin) {
            let before = read_manifest(&package)?;
            let name = before["name"].as_str().ok_or("package name missing")?;
            let old = before["version"]
                .as_str()
                .ok_or("package version missing")?
                .to_string();
            let mut cmd = npm.command();
            cmd.args(["view", name, "version", "--json"]);
            let latest: String = serde_json::from_str(&run(cmd, Duration::from_secs(30))?)
                .map_err(|e| e.to_string())?;
            // Never downgrade, or migrate a prerelease/custom version to stable.
            if !stable_version(&old)
                .zip(stable_version(&latest))
                .is_some_and(|(a, b)| b > a)
            {
                return Ok(None);
            }
            let mut cmd = npm.command();
            cmd.args([
                "install",
                "--global",
                "--no-audit",
                "--no-fund",
                &format!("{name}@{latest}"),
            ]);
            run_install(cmd, state)?;
            let after = read_manifest(&package)?;
            let new = after["version"]
                .as_str()
                .ok_or("updated version missing")?
                .to_string();
            return Ok((new == latest && new != old).then_some((old, new)));
        }
    }
    match installers::update(path, bin, state)? {
        installers::Outcome::Checked(result) => return Ok(result),
        installers::Outcome::Unmanaged => {}
    }
    // The native Claude installer owns this exact location. Package-manager
    // installations (including winget, brew, Scoop) are deliberately not guessed.
    if bin == "claude" && native_claude(path) {
        let old = version(path)?;
        let mut cmd = Command::new(path);
        cmd.arg("update");
        run_install(cmd, state)?;
        let new = version(path)?;
        return Ok((old != new).then_some((old, new)));
    }
    Ok(None)
}

fn native_claude(path: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let native = home.join(".local/bin").join(if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    });
    if path != native {
        return false;
    }
    if cfg!(windows) {
        !path.is_symlink()
            && !path.with_extension("bunx").exists()
            && !path.with_extension("shim").exists()
    } else {
        path.canonicalize()
            .ok()
            .is_some_and(|p| p.starts_with(home.join(".local/share/claude/versions")))
    }
}

fn version(path: &Path) -> Result<String, String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    let text = run(cmd, Duration::from_secs(15))?;
    text.split_whitespace()
        .next()
        .filter(|v| stable_version(v).is_some())
        .map(str::to_string)
        .ok_or("unrecognized CLI version".into())
}

/// File output avoids pipe-buffer deadlocks; child input is always noninteractive.
fn run(cmd: Command, timeout: Duration) -> Result<String, String> {
    run_command(cmd, timeout, None)
}

fn run_install(cmd: Command, state: &AppState) -> Result<String, String> {
    if !*state.running.lock() {
        return Err("application is stopping".into());
    }
    run_command(cmd, Duration::from_secs(600), Some(&state.settings))
}

fn run_command(
    mut cmd: Command,
    timeout: Duration,
    gate: Option<&parking_lot::Mutex<crate::settings::Settings>>,
) -> Result<String, String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let file_path = std::env::temp_dir().join(format!(
        "usage-monitor-update-{:032x}.log",
        u128::from_le_bytes(random)
    ));
    let mut created = false;
    let result = (|| {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .map_err(|e| e.to_string())?;
        created = true;
        cmd.stdin(Stdio::null()).stderr(Stdio::null());
        if gate.is_some() {
            cmd.stdout(Stdio::null());
        } else {
            cmd.stdout(file);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        platform::no_window(&mut cmd);
        let mut child = {
            // This is the same lock used by the toggle: once disabling returns,
            // no additional installer can start. Never hold it while waiting.
            let settings = gate.map(|lock| lock.lock());
            if settings.as_ref().is_some_and(|s| !s.provider_auto_update) {
                return Err("provider updates are disabled".into());
            }
            cmd.spawn().map_err(|e| e.to_string())?
        };
        let start = Instant::now();
        loop {
            if gate.is_none() && fs::metadata(&file_path).is_ok_and(|m| m.len() > MAX_OUTPUT) {
                terminate(&mut child);
                return Err("command output or metadata exceeds size limit".into());
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    if gate.is_some() {
                        return Ok(String::new());
                    }
                    return String::from_utf8(read_limited(&file_path)?).map_err(|e| e.to_string());
                }
                Ok(Some(status)) => return Err(format!("command exited with {status}")),
                Ok(None) if start.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                result => {
                    terminate(&mut child);
                    return Err(match result {
                        Err(e) => e.to_string(),
                        _ => "command timed out".into(),
                    });
                }
            }
        }
    })();
    if created {
        let _ = fs::remove_file(file_path);
    }
    result
}

fn terminate(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let program = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| root.join("System32/taskkill.exe"))
            .unwrap_or_else(|| PathBuf::from("taskkill.exe"));
        let mut kill = Command::new(program);
        kill.args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        platform::no_window(&mut kill);
        if let Ok(mut killer) = kill.spawn() {
            let start = Instant::now();
            while matches!(killer.try_wait(), Ok(None)) && start.elapsed() < Duration::from_secs(5)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = killer.kill();
            let _ = killer.wait();
        }
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_command(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "provider_updater::tests::runner_child_fixture",
            "--nocapture",
        ])
        .env("USAGE_MONITOR_UPDATER_TEST_CHILD", mode);
        cmd
    }

    // Only subprocesses of the test binary run here; no installed provider or
    // package manager is ever launched by these failure-path tests.
    #[test]
    fn runner_child_fixture() {
        match std::env::var("USAGE_MONITOR_UPDATER_TEST_CHILD").as_deref() {
            Ok("sleep") => std::thread::sleep(Duration::from_secs(15)),
            Ok("fail") => std::process::exit(7),
            Ok("output") => {
                print!("{}", "x".repeat(MAX_OUTPUT as usize + 512));
            }
            Ok("short") => {
                fs::write(
                    std::env::var_os("USAGE_MONITOR_UPDATER_TEST_READY").unwrap(),
                    "ready",
                )
                .unwrap();
                std::thread::sleep(Duration::from_millis(300));
            }
            Ok("ok") => println!("ready"),
            _ => {}
        }
    }

    #[test]
    fn runner_bounds_failures_timeouts_and_output_without_hanging() {
        assert!(run(fixture_command("ok"), Duration::from_secs(5))
            .unwrap()
            .contains("ready"));
        assert!(run(fixture_command("fail"), Duration::from_secs(5))
            .unwrap_err()
            .contains("command exited"));
        assert!(run(fixture_command("sleep"), Duration::from_millis(100))
            .unwrap_err()
            .contains("timed out"));
        assert!(run(fixture_command("output"), Duration::from_secs(5))
            .unwrap_err()
            .contains("size limit"));
        assert!(run(
            Command::new("usage-monitor-nonexistent-test-helper"),
            Duration::from_secs(1)
        )
        .is_err());
    }

    #[test]
    fn disabled_preference_is_checked_at_spawn_even_after_detection() {
        let settings = parking_lot::Mutex::new(crate::settings::Settings::default());
        settings.lock().provider_auto_update = false;
        let result = run_command(
            Command::new("usage-monitor-nonexistent-test-helper"),
            Duration::from_secs(1),
            Some(&settings),
        );
        assert_eq!(result.unwrap_err(), "provider updates are disabled");
    }

    #[test]
    fn in_flight_install_releases_the_settings_lock_and_finishes_after_disable() {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let dir =
            std::env::temp_dir().join(format!("um-runner-{:032x}", u128::from_le_bytes(random)));
        fs::create_dir(&dir).unwrap();
        let ready = dir.join("ready");
        let settings = Arc::new(parking_lot::Mutex::new(crate::settings::Settings::default()));
        let worker_settings = settings.clone();
        let mut cmd = fixture_command("short");
        cmd.env("USAGE_MONITOR_UPDATER_TEST_READY", &ready);
        let worker = std::thread::spawn(move || {
            run_command(cmd, Duration::from_secs(5), Some(&worker_settings))
        });
        let started = Instant::now();
        while !ready.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists());
        settings
            .try_lock()
            .expect("installer must not block the settings toggle")
            .provider_auto_update = false;
        assert!(worker.join().unwrap().is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn updates_are_daily_even_after_restart_and_clock_rollback() {
        assert!(due(100, 0));
        assert!(!due(101, 100));
        assert!(due(100 + INTERVAL, 100));
        assert!(due(99, 100));
    }

    #[test]
    fn prereleases_and_unknown_versions_are_not_migrated() {
        assert_eq!(stable_version("1.20.3"), Some((1, 20, 3)));
        for v in ["1.2.3-beta.1", "v1.2.3", "1.2", "1.2.3.4", ""] {
            assert_eq!(stable_version(v), None);
        }
        assert!(stable_version("1.10.0") > stable_version("1.9.9"));
    }

    #[test]
    fn malformed_package_identity_or_bin_cannot_target_another_installation() {
        for name in [
            "--help",
            "../codex",
            "https://example.com/tool",
            "@scope/tool/other",
            "@/tool",
            "",
        ] {
            assert!(!valid_package_name(name));
        }
        assert!(valid_package_name("@openai/codex"));
        assert!(valid_package_name("grok-cli"));
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "um-bin-boundary-{:032x}",
            u128::from_le_bytes(random)
        ));
        let package = dir.join("package");
        fs::create_dir_all(&package).unwrap();
        fs::write(dir.join("outside.js"), "// external binary").unwrap();
        let manifest = json!({"bin":{"codex":"../outside.js"}});
        assert!(package_target(&package, &manifest, "codex").is_none());
        let manifest = json!({"bin":{"codex":dir.join("outside.js")}});
        assert!(package_target(&package, &manifest, "codex").is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn package_must_expose_the_provider_binary() {
        let manifest = json!({"name":"@openai/codex", "bin":{"codex":"bin/codex.js"}});
        assert_eq!(bin_target(&manifest, "codex"), Some("bin/codex.js"));
        assert_eq!(bin_target(&manifest, "grok"), None);
        let manifest = json!({"name":"@scope/omp", "bin":"cli.js"});
        assert_eq!(bin_target(&manifest, "omp"), Some("cli.js"));
        assert_eq!(bin_target(&manifest, "codex"), None);
    }

    #[test]
    fn npm_only_matches_its_own_global_binary_not_another_installation() {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "provider-detection-{:032x}",
            u128::from_le_bytes(random)
        ));
        let root = if cfg!(windows) {
            dir.join("node_modules")
        } else {
            dir.join("lib/node_modules")
        };
        let package = root.join("@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@openai/codex","version":"1.0.0","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();
        fs::write(package.join("bin/codex.js"), "// test fixture").unwrap();
        let bin_dir = if cfg!(windows) {
            dir.clone()
        } else {
            dir.join("bin")
        };
        fs::create_dir_all(&bin_dir).unwrap();
        let executable = bin_dir.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
        #[cfg(windows)]
        fs::write(
            &executable,
            "@node \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(package.join("bin/codex.js"), &executable).unwrap();
        let npm = Npm {
            node: PathBuf::new(),
            cli: PathBuf::new(),
            prefix: dir.clone(),
            root,
        };
        assert_eq!(npm.package_for(&executable, "codex"), Some(package));
        assert_eq!(npm.package_for(&executable, "grok"), None);
        let other = dir.join("another-install");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("codex.cmd"), "another installation").unwrap();
        assert_eq!(npm.package_for(&other.join("codex.cmd"), "codex"), None);
        #[cfg(windows)]
        {
            fs::write(&executable, "@node elsewhere\\codex.js %*").unwrap();
            assert_eq!(npm.package_for(&executable, "codex"), None);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
