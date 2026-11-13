//! Package-manager adapters. Detection only reads metadata; mutations target a
//! single package after its executable has been matched to that installation.
use super::{
    package_target, read_limited, read_manifest, run, run_install, stable_version,
    valid_package_name,
};
use crate::state::AppState;
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(super) enum Outcome {
    Unmanaged,
    Checked(Option<(String, String)>),
}

struct Tool {
    program: PathBuf,
    args: Vec<OsString>,
}

impl Tool {
    fn find(name: &str) -> Option<Self> {
        let path = which::which(name).ok().or_else(|| {
            if name == "brew" {
                [
                    "/opt/homebrew/bin/brew",
                    "/usr/local/bin/brew",
                    "/home/linuxbrew/.linuxbrew/bin/brew",
                ]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
            } else {
                None
            }
        })?;
        Self::at(path)
    }

    fn at(mut path: PathBuf) -> Option<Self> {
        if cfg!(windows) {
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if ext == "cmd" || ext == "bat" {
                path.set_extension("ps1");
                if !path.is_file() {
                    return None;
                }
            }
            if path.extension()?.eq_ignore_ascii_case("ps1") {
                let program = which::which("pwsh").ok().or_else(|| {
                    Some(
                        PathBuf::from(std::env::var_os("SystemRoot")?)
                            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
                    )
                })?;
                // -File preserves argument boundaries; no interpolated shell code.
                return Some(Self {
                    program,
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-ExecutionPolicy".into(),
                        "Bypass".into(),
                        "-File".into(),
                        path.into_os_string(),
                    ],
                });
            }
        }
        Some(Self {
            program: path,
            args: vec![],
        })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .args(args)
            .env("NO_COLOR", "1")
            .env("CI", "true");
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(home);
        }
        cmd
    }

    fn read(&self, args: &[&str]) -> Result<String, String> {
        run(self.command(args), Duration::from_secs(30))
    }
}

pub(super) fn antigravity_path(bin: &str) -> Option<PathBuf> {
    which::which(bin).ok().or_else(|| {
        let local = dirs::data_local_dir()?;
        let candidate = if bin == "agy" {
            local.join("agy/bin/agy.exe")
        } else {
            local.join("Programs/Antigravity/Antigravity.exe")
        };
        candidate.is_file().then_some(candidate)
    })
}

pub(super) fn update(path: &Path, bin: &str, state: &AppState) -> Result<Outcome, String> {
    #[cfg(windows)]
    {
        match scoop(path, bin, state)? {
            Outcome::Unmanaged => {}
            result => return Ok(result),
        }
        match winget(path, bin, state)? {
            Outcome::Unmanaged => {}
            result => return Ok(result),
        }
    }
    #[cfg(unix)]
    match brew(path, bin, state)? {
        Outcome::Unmanaged => {}
        result => return Ok(result),
    }
    for manager in ["pnpm", "bun"] {
        match javascript(path, bin, manager, state)? {
            Outcome::Unmanaged => {}
            result => return Ok(result),
        }
    }
    Ok(Outcome::Unmanaged)
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.canonicalize()
        .ok()
        .zip(b.canonicalize().ok())
        .is_some_and(|(a, b)| a == b)
}

fn package_dirs(root: &Path) -> Vec<PathBuf> {
    let mut result = vec![];
    let Ok(entries) = fs::read_dir(root) else {
        return result;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('@') {
            if let Ok(scoped) = fs::read_dir(entry.path()) {
                result.extend(scoped.flatten().map(|e| e.path()));
            }
        } else if !name.to_string_lossy().starts_with('.') {
            result.push(entry.path());
        }
    }
    result
}

fn relative_path(target: &Path, base: &Path) -> Option<PathBuf> {
    let target: Vec<_> = target.components().collect();
    let base: Vec<_> = base.components().collect();
    if target.first() != base.first() {
        return None;
    }
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut result = PathBuf::new();
    for component in &base[common..] {
        if matches!(component, Component::Normal(_)) {
            result.push("..");
        }
    }
    for component in &target[common..] {
        result.push(component.as_os_str());
    }
    Some(result)
}

fn references(file: &Path, target: &Path, base: &Path) -> bool {
    let Ok(bytes) = read_limited(file) else {
        return false;
    };
    if bytes.len() > 1024 * 1024 {
        return false;
    }
    let mut candidates = vec![target.to_path_buf()];
    if let Some(relative) = relative_path(target, base) {
        candidates.push(relative);
    }
    for path in candidates {
        let text = path.to_string_lossy();
        for text in [text.replace('\\', "/"), text.replace('/', "\\")] {
            let utf16: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            if !text.is_empty()
                && (bytes.windows(text.len()).any(|w| w == text.as_bytes())
                    || bytes.windows(utf16.len()).any(|w| w == utf16))
            {
                return true;
            }
        }
    }
    false
}

fn owns_bin(package: &Path, manifest: &Value, path: &Path, bin_dir: &Path, bin: &str) -> bool {
    let Some(target) = package_target(package, manifest, bin) else {
        return false;
    };
    if !target.is_file() {
        return false;
    }
    if same_path(path, &target) {
        return true;
    }
    if !path.parent().is_some_and(|p| same_path(p, bin_dir)) {
        return false;
    }
    references(path, &target, bin_dir) || references(&path.with_extension("bunx"), &target, bin_dir)
}

/// pnpm 10 uses one global root; pnpm 11+ lists isolated install groups.
fn pnpm_roots(tool: &Tool) -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Ok(root) = tool.read(&["root", "--global"]) {
        let root = PathBuf::from(root.trim());
        if root.is_absolute() && root.is_dir() {
            roots.push(root);
        }
    }
    if let Ok(text) = tool.read(&["list", "--global", "--depth=0", "--json"]) {
        if let Ok(Value::Array(groups)) = serde_json::from_str(&text) {
            for group in groups {
                if let Some(path) = group["path"].as_str() {
                    let root = PathBuf::from(path).join("node_modules");
                    if root.is_absolute() && root.is_dir() && !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            }
        }
    }
    roots
}

fn bun_root(output: &str) -> Option<PathBuf> {
    let header = output.lines().find_map(|line| {
        line.split_once(" node_modules")
            .map(|(path, _)| path.trim())
    })?;
    let root = PathBuf::from(header).join("node_modules");
    root.is_absolute().then_some(root)
}

fn registry_spec(spec: &str) -> bool {
    !spec.contains([':', '/', '\\'])
        && (spec == "latest"
            || spec
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || "^~*<>= ".contains(c)))
}

#[cfg(any(unix, test))]
fn brew_owns(files: &str, path: &Path) -> bool {
    files.lines().any(|line| {
        // Directory counts and symlink annotations are not part of the path.
        let line = line.trim().split(" -> ").next().unwrap_or("");
        let line = line.split(" (").next().unwrap_or("");
        let file = PathBuf::from(line);
        file.is_absolute()
            && file
                .canonicalize()
                .ok()
                .zip(path.canonicalize().ok())
                .is_some_and(|(file, path)| {
                    path == file || (file.is_dir() && path.starts_with(file))
                })
    })
}

fn javascript(path: &Path, bin: &str, manager: &str, state: &AppState) -> Result<Outcome, String> {
    let Some(tool) = Tool::find(manager) else {
        return Ok(Outcome::Unmanaged);
    };
    let bin_args: &[&str] = if manager == "bun" {
        &["pm", "bin", "--global"]
    } else {
        &["bin", "--global"]
    };
    let Ok(dir) = tool.read(bin_args) else {
        return Ok(Outcome::Unmanaged);
    };
    let bin_dir = PathBuf::from(dir.trim());
    if !path.parent().is_some_and(|p| same_path(p, &bin_dir)) {
        return Ok(Outcome::Unmanaged);
    }
    let roots = if manager == "pnpm" {
        pnpm_roots(&tool)
    } else {
        tool.read(&["pm", "ls", "--global"])
            .ok()
            .and_then(|s| bun_root(&s))
            .into_iter()
            .collect()
    };
    for root in roots {
        let global_manifest = root.parent().and_then(|p| read_manifest(p).ok());
        for package in package_dirs(&root) {
            let Ok(manifest) = read_manifest(&package) else {
                continue;
            };
            let Some(name) = manifest["name"].as_str() else {
                continue;
            };
            if !valid_package_name(name) {
                continue;
            }
            if root.join(name).canonicalize().ok() != package.canonicalize().ok() {
                continue;
            }
            // Require a top-level registry dependency, excluding linked checkouts.
            let Some(spec) = global_manifest
                .as_ref()
                .and_then(|m| m["dependencies"][name].as_str())
            else {
                continue;
            };
            if !registry_spec(spec) {
                continue;
            }
            if !owns_bin(&package, &manifest, path, &bin_dir, bin) {
                continue;
            }
            let old = manifest["version"]
                .as_str()
                .ok_or("package version missing")?
                .to_string();
            if stable_version(&old).is_none() {
                return Ok(Outcome::Checked(None));
            }
            // Both commands update only an existing named global package and
            // preserve the manager's global root, registry and script policy.
            let package_spec;
            let target = if manager == "pnpm" {
                let latest: String =
                    serde_json::from_str(&tool.read(&["view", name, "version", "--json"])?)
                        .map_err(|e| e.to_string())?;
                if !stable_version(&old)
                    .zip(stable_version(&latest))
                    .is_some_and(|(a, b)| b > a)
                {
                    return Ok(Outcome::Checked(None));
                }
                package_spec = format!("{name}@{latest}");
                package_spec.as_str()
            } else {
                name
            };
            run_install(
                tool.command(&["update", "--global", "--latest", target]),
                state,
            )?;
            let roots = if manager == "pnpm" {
                pnpm_roots(&tool)
            } else {
                vec![root]
            };
            for root in roots {
                let candidate = root.join(name);
                if let Ok(after) = read_manifest(&candidate) {
                    if let Some(new) = after["version"].as_str() {
                        if stable_version(&new)
                            .zip(stable_version(&old))
                            .is_some_and(|(a, b)| a > b)
                        {
                            return Ok(Outcome::Checked(Some((old, new.to_string()))));
                        }
                    }
                }
            }
            return Ok(Outcome::Checked(None));
        }
    }
    Ok(Outcome::Unmanaged)
}

#[cfg(any(unix, test))]
#[cfg_attr(windows, allow(dead_code))]
fn brew(path: &Path, bin: &str, state: &AppState) -> Result<Outcome, String> {
    let Some(tool) = Tool::find("brew") else {
        return Ok(Outcome::Unmanaged);
    };
    let Ok(text) = tool.read(&["info", "--json=v2", "--installed"]) else {
        return Ok(Outcome::Unmanaged);
    };
    let inventory: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for (key, kind) in [("formulae", "--formula"), ("casks", "--cask")] {
        let Some(entries) = inventory[key].as_array() else {
            continue;
        };
        for entry in entries {
            let name = if key == "casks" {
                entry["token"].as_str()
            } else {
                entry["full_name"]
                    .as_str()
                    .or_else(|| entry["name"].as_str())
            };
            let Some(name) = name else { continue };
            let short_name = name
                .rsplit('/')
                .next()
                .unwrap_or(name)
                .split('@')
                .next()
                .unwrap_or(name);
            let known = match bin {
                "claude" => short_name == "claude-code",
                "codex" => short_name == "codex",
                "grok" => ["grok", "grok-cli", "grok-shell"].contains(&short_name),
                "omp" => ["omp", "oh-my-pi"].contains(&short_name),
                "antigravity" => short_name == "antigravity",
                "agy" => ["agy", "antigravity-cli"].contains(&short_name),
                _ => false,
            };
            if !known {
                continue;
            }
            let installed = &entry["installed"];
            let old = if key == "casks" {
                installed.as_str()
            } else {
                installed
                    .as_array()
                    .and_then(|a| a.last())
                    .and_then(|v| v["version"].as_str())
            };
            let Some(old) = old else { continue };
            let Ok(files) = tool.read(&["list", kind, name]) else {
                continue;
            };
            let owned = brew_owns(&files, path);
            if !owned {
                continue;
            }
            if entry["pinned"].as_bool() == Some(true) {
                return Ok(Outcome::Checked(None));
            }
            let mut cmd = tool.command(&["upgrade", kind, name]);
            // Do not upgrade unrelated installed dependents or invoke sudo.
            cmd.env("HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK", "1")
                .env("SUDO_ASKPASS", "/usr/bin/false")
                .env("HOMEBREW_SUDO_ASKPASS", "/usr/bin/false");
            run_install(cmd, state)?;
            let after: Value =
                serde_json::from_str(&tool.read(&["info", "--json=v2", kind, name])?)
                    .map_err(|e| e.to_string())?;
            let installed = &after[key][0]["installed"];
            let new = if key == "casks" {
                installed.as_str()
            } else {
                installed
                    .as_array()
                    .and_then(|a| a.last())
                    .and_then(|v| v["version"].as_str())
            };
            return Ok(Outcome::Checked(
                new.filter(|new| *new != old)
                    .map(|new| (old.to_string(), new.to_string())),
            ));
        }
    }
    Ok(Outcome::Unmanaged)
}

#[cfg(windows)]
fn scoop_target(path: &Path) -> Option<PathBuf> {
    let shim = path.with_extension("shim");
    let text = fs::read_to_string(shim).ok()?;
    let value = text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "path").then(|| value.trim().trim_matches('"'))
    })?;
    let target = PathBuf::from(value);
    target.is_absolute().then_some(target)
}

#[cfg(windows)]
fn scoop(path: &Path, bin: &str, state: &AppState) -> Result<Outcome, String> {
    let Some(root) = std::env::var_os("SCOOP")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join("scoop")))
    else {
        return Ok(Outcome::Unmanaged);
    };
    let script = root.join("apps/scoop/current/bin/scoop.ps1");
    let Some(tool) = script.is_file().then(|| Tool::at(script)).flatten() else {
        return Ok(Outcome::Unmanaged);
    };
    let target = scoop_target(path).unwrap_or_else(|| path.to_path_buf());
    let Ok(entries) = fs::read_dir(root.join("apps")) else {
        return Ok(Outcome::Unmanaged);
    };
    for entry in entries.flatten() {
        let current = entry.path().join("current");
        let direct = target
            .canonicalize()
            .ok()
            .zip(current.canonicalize().ok())
            .is_some_and(|(target, current)| target.starts_with(current));
        let manifest = match scoop_manifest(&current) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let owned = direct
            || scoop_bins(&manifest, bin).iter().any(|bin_target| {
                let target = current.join(bin_target);
                target.is_file() && references(path, &target, &root.join("shims"))
            });
        if !owned {
            continue;
        }
        let info: Value = serde_json::from_slice(
            &fs::read(current.join("install.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if info["hold"].as_bool() == Some(true) {
            return Ok(Outcome::Checked(None));
        }
        let old = manifest["version"]
            .as_str()
            .ok_or("Scoop version missing")?
            .to_string();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_package_name(&name) {
            return Ok(Outcome::Unmanaged);
        }
        run_install(tool.command(&["update", &name]), state)?;
        let after = scoop_manifest(&current)?;
        let new = after["version"]
            .as_str()
            .ok_or("Scoop updated version missing")?
            .to_string();
        return Ok(Outcome::Checked((new != old).then_some((old, new))));
    }
    Ok(Outcome::Unmanaged)
}

#[cfg(windows)]
fn scoop_bins<'a>(manifest: &'a Value, bin: &str) -> Vec<&'a str> {
    fn entry<'a>(value: &'a Value, bin: &str) -> Option<&'a str> {
        let (target, alias) = match value {
            Value::String(target) => (target.as_str(), None),
            Value::Array(values) => (
                values.first()?.as_str()?,
                values.get(1).and_then(Value::as_str),
            ),
            _ => return None,
        };
        let stem = Path::new(target).file_stem()?.to_str()?;
        (alias.unwrap_or(stem) == bin).then_some(target)
    }
    match &manifest["bin"] {
        Value::String(_) => entry(&manifest["bin"], bin).into_iter().collect(),
        Value::Array(values) => values.iter().filter_map(|v| entry(v, bin)).collect(),
        _ => vec![],
    }
}

#[cfg(windows)]
fn scoop_manifest(current: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(current.join("manifest.json")).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn winget_id(bin: &str) -> Option<&'static str> {
    match bin {
        "claude" => Some("Anthropic.ClaudeCode"),
        "codex" => Some("OpenAI.Codex"),
        "antigravity" => Some("Google.Antigravity"),
        _ => None,
    }
}

#[cfg(windows)]
fn winget(path: &Path, bin: &str, state: &AppState) -> Result<Outcome, String> {
    let Some(id) = winget_id(bin) else {
        return Ok(Outcome::Unmanaged);
    };
    let Some(tool) = Tool::find("winget") else {
        return Ok(Outcome::Unmanaged);
    };
    let Some(local) = dirs::data_local_dir() else {
        return Ok(Outcome::Unmanaged);
    };
    let target = path.canonicalize().map_err(|e| e.to_string())?;
    let packages = local.join("Microsoft/WinGet/Packages");
    let portable = fs::read_dir(packages).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}_"))
                && entry
                    .path()
                    .canonicalize()
                    .ok()
                    .is_some_and(|dir| target.starts_with(dir))
        })
    });
    // Installer-based packages must match a per-user uninstall registration and
    // its install directory. Merely appearing in `winget list` is insufficient.
    let registered = if portable {
        None
    } else {
        winget_registration(path, bin)?
    };
    if !portable && registered.is_none() {
        return Ok(Outcome::Unmanaged);
    }
    let list = tool.read(&[
        "list",
        "--id",
        id,
        "--exact",
        "--scope",
        "user",
        "--source",
        "winget",
        "--disable-interactivity",
    ])?;
    let old = list.lines().find_map(|line| {
        let words: Vec<_> = line.split_whitespace().collect();
        let pos = words.iter().position(|w| *w == id)?;
        words.get(pos + 1).map(|v| v.to_string())
    });
    let Some(old) = old else {
        return Ok(Outcome::Checked(None));
    };
    let available = tool.read(&[
        "list",
        "--upgrade-available",
        "--id",
        id,
        "--exact",
        "--scope",
        "user",
        "--source",
        "winget",
        "--disable-interactivity",
    ])?;
    if !available
        .lines()
        .any(|line| line.split_whitespace().any(|word| word == id))
    {
        return Ok(Outcome::Checked(None));
    }
    let updates = run_install(
        tool.command(&[
            "upgrade",
            "--id",
            id,
            "--exact",
            "--scope",
            "user",
            "--source",
            "winget",
            "--silent",
            "--skip-dependencies",
            "--disable-interactivity",
        ]),
        state,
    );
    let after = tool.read(&[
        "list",
        "--id",
        id,
        "--exact",
        "--scope",
        "user",
        "--source",
        "winget",
        "--disable-interactivity",
    ])?;
    let new = after.lines().find_map(|line| {
        let words: Vec<_> = line.split_whitespace().collect();
        let pos = words.iter().position(|w| *w == id)?;
        words.get(pos + 1).map(|v| v.to_string())
    });
    if let Some(new) = new.filter(|v| *v != old) {
        return Ok(Outcome::Checked(Some((old, new))));
    }
    updates?;
    Ok(Outcome::Checked(None))
}

#[cfg(windows)]
fn winget_registration(path: &Path, bin: &str) -> Result<Option<String>, String> {
    let Some(root) = std::env::var_os("SystemRoot") else {
        return Ok(None);
    };
    let mut cmd =
        Command::new(PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    // Inputs are passed via environment variables, never inserted in script text.
    const SCRIPT: &str = r#"[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$names = @{claude='Claude Code';codex='Codex';antigravity='Antigravity'}
$p = [IO.Path]::GetFullPath($env:USAGE_MONITOR_PROVIDER_PATH)
$rows = @(Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object {
  $_.DisplayName -eq $names[$env:USAGE_MONITOR_PROVIDER_BIN] -and $_.InstallLocation -and
  $p.StartsWith(([IO.Path]::GetFullPath($_.InstallLocation).TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)
} | Select-Object -ExpandProperty DisplayVersion)
ConvertTo-Json -InputObject $rows -Compress"#;
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("USAGE_MONITOR_PROVIDER_PATH", path)
        .env("USAGE_MONITOR_PROVIDER_BIN", bin);
    let output = run(cmd, Duration::from_secs(15))?;
    let versions: Vec<String> = serde_json::from_str(output.trim_start_matches('\u{feff}').trim())
        .map_err(|e| e.to_string())?;
    Ok((versions.len() == 1).then(|| versions[0].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_git_and_custom_package_sources_are_not_switched_to_registry() {
        for spec in [
            "link:../checkout",
            "file:../tool",
            "workspace:*",
            "github:org/repo",
            "org/repo",
            "https://example.com/tool.tgz",
            "next",
        ] {
            assert!(!registry_spec(spec));
        }
        for spec in ["1.2.3", "^1.2.3", "~1.0", "latest", "*"] {
            assert!(registry_spec(spec));
        }
    }

    #[test]
    fn bun_custom_global_root_is_read_from_inventory() {
        let dir = std::env::temp_dir().join("custom bun global");
        let output = format!(
            "{} node_modules (2 installed)\n├── @openai/codex@1.0.0",
            dir.display()
        );
        assert_eq!(bun_root(&output), Some(dir.join("node_modules")));
        assert_eq!(bun_root("unexpected output"), None);
    }

    #[test]
    fn relative_shim_target_keeps_package_and_installation_identity() {
        let base = std::env::temp_dir().join("pnpm/bin");
        let target =
            std::env::temp_dir().join("pnpm/global/5/node_modules/@openai/codex/bin/codex.js");
        assert_eq!(
            relative_path(&target, &base),
            Some(PathBuf::from(
                "../global/5/node_modules/@openai/codex/bin/codex.js"
            ))
        );
    }

    #[test]
    fn shims_must_reference_the_exact_global_package_even_with_spaces_and_utf16() {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "manager fixture {:032x}",
            u128::from_le_bytes(random)
        ));
        let bin_dir = dir.join("bin");
        let package = dir.join("global/node_modules/@openai/codex");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("bin/codex.js"), "// fixture").unwrap();
        let executable = bin_dir.join("codex.exe");
        fs::write(&executable, "binary shim fixture").unwrap();
        let manifest = serde_json::json!({"name":"@openai/codex","bin":{"codex":"bin/codex.js"}});
        let target = package.join("bin/codex.js");
        let relative = relative_path(&target, &bin_dir).unwrap();
        let reference: Vec<_> = relative
            .to_string_lossy()
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(executable.with_extension("bunx"), &reference).unwrap();
        assert!(owns_bin(
            &package,
            &manifest,
            &executable,
            &bin_dir,
            "codex"
        ));
        assert!(!owns_bin(
            &package,
            &manifest,
            &executable,
            &bin_dir,
            "grok"
        ));
        let another = dir.join("another-global/node_modules/@openai/codex");
        fs::create_dir_all(another.join("bin")).unwrap();
        fs::write(another.join("bin/codex.js"), "// another installation").unwrap();
        assert!(!owns_bin(
            &another,
            &manifest,
            &executable,
            &bin_dir,
            "codex"
        ));
        fs::write(executable.with_extension("bunx"), "unknown shim layout").unwrap();
        assert!(!owns_bin(
            &package,
            &manifest,
            &executable,
            &bin_dir,
            "codex"
        ));
        let listing = format!("==> App\n{} (3 files)\n", package.display());
        assert!(brew_owns(&listing, &target));
        assert!(!brew_owns(&listing, &another.join("bin/codex.js")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn scoop_script_aliases_match_the_declared_provider_command() {
        let manifest = serde_json::json!({"bin":[["dist/grok.js", "grok", "node"], "codex.exe"]});
        assert_eq!(scoop_bins(&manifest, "grok"), vec!["dist/grok.js"]);
        assert_eq!(scoop_bins(&manifest, "codex"), vec!["codex.exe"]);
        assert!(scoop_bins(&manifest, "claude").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn winget_ids_are_exact_cli_ids_not_desktop_app_names() {
        assert_eq!(winget_id("claude"), Some("Anthropic.ClaudeCode"));
        assert_eq!(winget_id("codex"), Some("OpenAI.Codex"));
        assert_eq!(winget_id("grok"), None);
        assert_eq!(winget_id("unknown"), None);
    }
}
