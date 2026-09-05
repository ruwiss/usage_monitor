# Usage Monitor

Windows / Linux / macOS tray app for live quota of **Claude**, **Codex**, **Grok**, and **9Router**. No API key entry: it reads the CLI login you already have.

## What it tracks

| Source | When it appears | Credentials |
|---|---|---|
| **Claude** | Claude CLI logged in | `~/.claude/.credentials.json` |
| **Codex** | Codex CLI logged in | `~/.codex/auth.json` |
| **Grok** | Grok CLI logged in | `~/.grok/auth.json` |
| **9Router** | Local 9Router running | `http://localhost:20128` (quota-capable providers only) |
| **Custom** | HTTP source in Settings | `usage-monitor-settings.json` |

A provider with **no account** stays off the list. Native CLI and 9Router of the same provider are separate entries.

## Features

- **Live tray icon** — session + weekly bars, or stacked percentages via `icon_style`. Theme-aware.
- **Detail popup** — left-click (Windows). Account, reset countdown, extra usage, quota bars. Pin it. Compact view via `compact_hide`.
- **Smart alerts** — per-quota thresholds, time-aware mode, reset toasts.
- **[Event commands](docs/event-commands.md)** — shell command on reset, threshold, startup, or tray quick action.
- **Adaptive polling** — faster while usage climbs, pauses when idle/locked, aligns to the next reset.
- **Start at login** — tray menu, Windows and Linux.
- **13 languages** — auto-detected, override with `language`.

## Security

Credentials stay on disk where the CLI put them. Used only as HTTP `Authorization` headers. Never logged.

Network: Anthropic (`api.anthropic.com`), OpenAI Codex (`chatgpt.com`), xAI Grok (`cli-chat-proxy.grok.com`, `auth.x.ai`, `grok.com`), optional local 9Router (`localhost:20128`) plus any custom URL you add.

No analytics, tracking, or telemetry. See [PRIVACY.md](PRIVACY.md).

## Requirements

- **Windows 10/11** (64-bit), **Linux** (freedesktop tray), or **macOS**.
- At least one logged-in CLI (Claude, Codex, or Grok) **or** a running 9Router.

Missing token: tray shows `!`. Log in to that CLI; the next poll picks it up. Claude 401 runs `claude update`. Codex/Grok refresh their own OAuth tokens.

## Install

**[Download the latest release](https://github.com/ruwiss/usage_monitor/releases/latest)**

| Platform | Artifact |
|---|---|
| Windows | NSIS installer (`.exe`) and MSI |
| Debian / Ubuntu / Mint | `.deb` (amd64, arm64) |
| Fedora / RHEL / openSUSE | `.rpm` (x86_64, aarch64) |
| Any Linux | `.AppImage` (x64 and ARM64) |
| Arch Linux | [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD) (binary, wraps AppImage) |
| macOS | `.dmg` (Apple Silicon and Intel) |

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

Open the `.dmg`, drag **Usage Monitor** to Applications. Unsigned build: Gatekeeper may say the app is damaged. Clear quarantine:

```bash
xattr -cr "/Applications/Usage Monitor.app"
```

Then open it from Applications (not from the DMG).

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
| **Left-click** (Windows) | Detail popup |
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

Independent project. Not created or endorsed by Anthropic, OpenAI, or xAI. Product names are used only to describe compatibility.
