# Changelog

## [1.0.0] - 2026-09-05

Rewrite as a Tauri 2 tray app. Same product: live quota for Claude, Codex, Grok, 9Router, and custom HTTP sources.

### Added

- Native Windows / Linux / macOS binaries via Tauri (NSIS, MSI, deb, rpm, AppImage, dmg)
- GitHub Actions release workflow for those targets
- Multi-source tray (native CLI only if logged in; 9Router quota-capable providers only)
- Popup: pin, outside-click / Escape dismiss, compact height, Settings inside the popup
- Event commands with Python-compatible environment variables and a Test submenu

### Changed

- Package is `usage-monitor` (Tauri). Internal crate version `1.0.0`
- Popup no longer lists Claude CLI / IDE “Providers”
- Settings search order unchanged: `--config-dir`, next to the EXE, then `~/.claude/`
