# Usage Monitor

Windows / Linux / macOS tray app for live quota of **Claude**, **Codex**, **Grok**, **Antigravity**, **OMP**, **9Router**, and key-based providers (OpenRouter, GitHub Copilot, OpenCode Go, z.ai GLM, Kimi Code, MiniMax, DeepSeek, Moonshot). CLI sources need no key entry: the app reads the login you already have. Provider API keys are optional and added in Settings.

## What it tracks

| Source | When it appears | Credentials |
|---|---|---|
| **Claude** | Claude CLI logged in | `~/.claude/.credentials.json` |
| **Claude (web)** | Signed in via Settings > Sources > "Sign in to Claude (web)" (no Claude CLI needed) | claude.ai session in the app's own browser profile; see [PRIVACY.md](PRIVACY.md) |
| **Codex** | Codex CLI logged in | `~/.codex/auth.json` |
| **Grok** | Grok CLI logged in | `~/.grok/auth.json` |
| **Antigravity** (Windows) | `agy` CLI / Antigravity IDE logged in | Windows Credential Manager entry `gemini:antigravity` (read-only); quotas per model family: Gemini Pro, Gemini Flash, Claude, GPT-OSS, Image |
| **OMP** | `omp` on PATH (or `%LOCALAPPDATA%\omp\omp.exe`) | `omp usage --json` — one source per logged-in OMP provider |
| **9Router** | Local 9Router running | `http://localhost:20128` (quota-capable providers only) |
| **Custom** | HTTP source in Settings (test URL, pick fields) | `usage-monitor-settings.json` |
| **GitHub Copilot** | `gh` signed in, or `GH_TOKEN` / `GITHUB_TOKEN` set (auto), or a token added in Settings | `gh auth token`, env, or settings JSON |
| **OpenCode Go** | OpenCode `auth.json` has an `opencode-go` key (auto), or a key added in Settings | `~/.local/share/opencode/auth.json` or settings JSON |
| **OpenRouter**, **z.ai GLM**, **Kimi Code**, **MiniMax**, **DeepSeek**, **Moonshot** | Key added in Settings > Sources > *Add provider key* (tested once before it is saved) | `usage-monitor-settings.json` |

A provider with **no account** stays off the list. Native CLI, OMP, and 9Router of the same provider are separate entries.

## Features

- **Live tray icon** — session + weekly bars, or stacked percentages via `icon_style`. Theme-aware.
- **Detail popup** — left-click (Windows and macOS). Account, reset countdown, extra usage, quota bars. Pin it. Compact view via `compact_hide`.
- **Smart alerts** — per-quota thresholds, time-aware mode, reset toasts.
- **[Event commands](docs/event-commands.md)** — shell command on reset, threshold, startup, or tray quick action.
- **Adaptive polling** — faster while usage climbs, pauses when idle/locked, aligns to the next reset.
- **Start at login** — tray menu, Windows, Linux, and macOS.
- **13 languages** — auto-detected, override with `language`.

## Security

Credentials stay on disk where the CLI put them. Used only as HTTP `Authorization` headers. Never logged.

Network: Anthropic (`api.anthropic.com`), OpenAI Codex (`chatgpt.com`), xAI Grok (`cli-chat-proxy.grok.com`, `auth.x.ai`, `grok.com`), Google Antigravity (`cloudcode-pa.googleapis.com`, `oauth2.googleapis.com` for token refresh), optional local 9Router (`localhost:20128`) plus any custom URL you add. Key-based providers contact only their own API host, and only after you add a key or sign in with `gh` / OpenCode: `openrouter.ai`, `api.github.com`, `opencode.ai`, `api.z.ai` (falls back to `open.bigmodel.cn`), `api.kimi.com`, `api.minimax.io` (falls back to `api.minimaxi.com`), `api.deepseek.com`, `api.moonshot.ai` (falls back to `api.moonshot.cn`). Keys you type are stored in plain text in `usage-monitor-settings.json`, like custom source tokens. **OMP** stays local: the app runs `omp usage --json` and does not read OMP credential files.

No analytics, tracking, or telemetry. See [PRIVACY.md](PRIVACY.md).

## Requirements

- **Windows 10/11** (64-bit), **Linux** (freedesktop tray), or **macOS**.
- At least one logged-in CLI (Claude, Codex, Grok, or OMP) **or** a running 9Router.

Missing token: tray shows `!`. Log in to that CLI; the next poll picks it up. Claude 401 runs `claude update`. Codex/Grok refresh their own OAuth tokens.

## Install

**[Download the latest release](https://github.com/ruwiss/usage_monitor/releases/latest)** — pick **one** file:

| You have | Download |
|---|---|
| Windows | `x64-setup.exe` (MSI is optional) |
| macOS Apple Silicon | `aarch64.dmg` |
| macOS Intel | `x64.dmg` |
| Debian / Ubuntu | `.deb` (`amd64` or `arm64`) |
| Fedora / RHEL | `.rpm` (`x86_64` or `aarch64`) |
| Any Linux | `.AppImage` |
| Arch | [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD) |

Skip `.sig`, `.app.tar.gz`, and `latest.json` — those are for auto-update.

### Linux

```bash
# Debian / Ubuntu
sudo apt install ./Usage.Monitor_*_amd64.deb

# Fedora / RHEL
sudo dnf install ./Usage.Monitor-*-1.x86_64.rpm

# AppImage
chmod +x Usage.Monitor_*.AppImage
./Usage.Monitor_*.AppImage

# Arch (from this repo)
cd packaging/arch
makepkg -si
```

### macOS

Open the `.dmg`, drag **Usage Monitor** to Applications. Then open it from Applications (not from the DMG).

CI builds are **ad-hoc signed** (no Apple Developer ID). First launch may ask you to confirm an unidentified developer: right-click the app → **Open**. If Gatekeeper still says the app is damaged:

```bash
xattr -cr "/Applications/Usage Monitor.app"
```

Later GitHub Releases auto-updates do this themselves (no `sudo`, no extra notification). The first `.dmg` install can still need the command once.

The app is a menu-bar extra (no Dock icon). Left-click the status item for the detail popup; right-click for the menu (providers, start at login, quit). GUI apps do not inherit your shell `PATH`; the app prepends Homebrew and user bin dirs (`/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, …) so `claude` / `omp` resolve.

## Quick Start (from source)

Rust + Node 20+.

```bash
npm install
npm run tauri
```

Or:

```bash
cd src-tauri
cargo run
```

### Linux deps

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
npm install
npm run tauri
```

On Linux the popup opens from the tray **menu** (the panel eats the left-click).

## How to Use

| Action | What happens |
|---|---|
| **Hover** | Tooltip: usage % and reset times |
| **Left-click** (Windows / macOS) | Detail popup (toggle on macOS) |
| **Double-click** (Windows) | [Quick action](docs/event-commands.md) if configured |
| **Right-click** / Linux left-click | Source picker, Start at login, restart, quit |
| **Escape** / click outside | Close popup |

Windows may hide new tray icons: Taskbar settings → Other system tray icons → **Usage Monitor** On.

Each popup bar: blue fill = used, white tick = elapsed time, red fill = usage ahead of the clock.

## Configuration

Optional `usage-monitor-settings.json` (first match wins):

1. `$CLAUDE_CONFIG_DIR/usage-monitor-settings.json` when `--config-dir` / `CLAUDE_CONFIG_DIR` is set
2. Next to the executable (or project root from source)
3. `~/.claude/usage-monitor-settings.json`

The app never creates this file on first run. Full key list: [Configuration](docs/configuration.md).

```json
{
  "poll_interval": 180,
  "ninerouter_url": "http://localhost:20128",
  "icon_style": "number+bars",
  "source_id": "grok"
}
```

## Building

```bash
npm install
cd src-tauri
cargo test --lib
npm run build
```

Outputs land in `src-tauri/target/release/bundle/`.

CI on every `v*` tag: Windows NSIS+MSI, Linux deb/rpm/AppImage (x64 + ARM64), macOS dmg (Apple Silicon + Intel). Arch: [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD). See `.github/workflows/release.yml`.

## Disclaimer

Independent project. Not created or endorsed by Anthropic, OpenAI, xAI, or Cursor. Product names are used only to describe compatibility.

### Local provider auto-updates

`provider_auto_update` (default `true`) checks visible Claude Code, Codex CLI, Grok CLI, OMP and Antigravity sources once every 24 hours, independently of usage polling and authentication. The app matches the executable to an existing installation before choosing an updater:

| Installation | Detection and update |
|---|---|
| npm global | Existing npm prefix, package manifest and binary target; updates that exact package in the same prefix |
| pnpm global | Manager-reported global bin and package roots, including isolated install groups; updates only the matching top-level registry package |
| Bun global | Manager-reported global bin and global package inventory, including custom directories; matches the binary/shim to the package before updating it |
| Homebrew | Installed formula/cask inventory and package file ownership; upgrades the matching package and respects pins |
| Scoop | Per-user install directory and executable/shim ownership; updates the installed app and respects holds |
| winget | Exact CLI package ID plus per-user portable directory or uninstall registration and install path; uses a silent, noninteractive upgrade |
| Native Claude | Standard per-user native install location; uses `claude update` |

Unknown/portable installations and API/web-only sources are skipped. Unavailable package managers are skipped. Machine-wide Windows packages requiring elevation are not updated. The worker never falls back to installing a second copy through another package manager. External package registries, installed Scoop buckets and the winget/Homebrew repositories may be contacted; their normal installation scripts and dependency handling apply. Updates may fail while the provider is running or if a package agreement requires interaction.

Use **Settings → Sources → Automatically update source apps** to turn provider updates on or off with one click. Disabling takes effect before any new installer starts, including when detection is already running. An installer that has already started is allowed to finish; turning the switch off does not interrupt it. If settings cannot be saved, disabling still takes effect for the current session and the UI warns that it may change after restarting. Enabling requires a successful save.

These settings in `usage-monitor-settings.json` are separate from Usage Monitor's own `auto_update`:

```json
{
  "provider_auto_update": true,
  "notify_provider_update": true
}
```

Notifications name the updated application and show the before/after versions. The legacy `notify_claude_update` preference is used when `notify_provider_update` is absent. Prerelease/custom JavaScript package versions are skipped. Checks, including failed attempts, are limited to once per day across restarts. Read-only detection commands have short timeouts; installers have up to 10 minutes to finish. Commands run without interactive input. Failures do not stop usage polling and are retried on the next daily check. Failed timestamp writes are also throttled for the current session. Command output and metadata reads are bounded. Settings writes preserve unrelated keys, reject unreadable/corrupt files and replace the saved file atomically.

Adapter command references: [npm prefixes](https://docs.npmjs.com/cli/configuring-npm/folders), [pnpm global packages](https://github.com/pnpm/pnpm/blob/main/pnpm/docs/global-packages.md), [Bun global updates](https://bun.sh/docs/pm/cli/update), [Homebrew](https://docs.brew.sh/Manpage), [Scoop](https://github.com/ScoopInstaller/Scoop/wiki/FAQ), [winget](https://learn.microsoft.com/windows/package-manager/winget/upgrade).
