# Changelog

## Unreleased

## [1.0.7] - 2026-09-06

- Check for app updates on every launch, not once every 6 hours
- Tray icon uses the first percentage quota from the popup, not Claude `five_hour` / `seven_day`; X only when that quota is exhausted
- Settings: fade between usage and settings; left tabs rise in from the bottom
- Settings: fade and ease the window height when switching tabs
- Settings: no shrink flicker on first open, and no scrollbar flash on Custom

## [1.0.6] - 2026-09-06

- GitHub Releases: drop `.sig` assets and list one download link per OS

## [1.0.5] - 2026-09-06

- Fix Grok credits decode aborting the app on every poll
- Settings: toggle limit display between used and remaining; tray number follows
- Tray icon shows 0 (used) or 100 (remaining) when a quota is unused
- Settings: left tabs, distinct used/remaining colors, rename 9Router to 9Router / OmniRoute
- Settings: rename Saved to Sources; uncheck native or custom sources to hide them from the tray menu
- Auto-update from GitHub Releases on startup: one notification, then a silent background install
- macOS auto-update clears Gatekeeper quarantine so ad-hoc builds do not need `xattr -cr` after install

## [1.0.4] - 2026-09-05

- Cap the popup at 90% of the screen height and scroll when settings overflow
- Settings: separate 9Router, custom source, and delete cards; rename custom field display names
- Custom sources: test the URL, pick suggested fields, any JSON key, or type a path
- Replace the app icon on Windows, Linux, and macOS
- Keep native Grok visible even when OMP already lists `xai-oauth`
- **macOS:** menu-bar accessory (no Dock icon); left-click opens the popup under the status item, right-click opens the tray menu
- **macOS:** tray icon is a template image so it stays visible on both light and dark (including wallpaper-tinted) menu bars
- **macOS:** system locale, idle time, and screen-lock detection (adaptive polling)
- **macOS:** error dialogs via `osascript`; GUI `PATH` includes Homebrew and user bin dirs
- **macOS:** load bundled locale files from `Contents/Resources`; persist settings to `~/.claude` when the app bundle is not writable
- Bundle `icon.png` so macOS `.icns` is generated; `LSUIElement` + `minimumSystemVersion` 12.0
- **macOS CI:** ad-hoc sign without hardened runtime (empty Apple cert secrets + hardened runtime made Gatekeeper report a damaged app)

## [1.0.3] - 2026-09-05

- Add **OMP** source when `omp` is installed: `omp usage --json`, one tray entry per provider
- OMP bars show remaining % (same as `omp usage`); gpt-4 request counts render as text
- Hide native Grok while OMP already lists `xai-oauth`

## [1.0.2] - 2026-09-05

- Drop unused `tauri::Manager` import on Unix
- Stop passing empty Apple certificate secrets in CI (broke ad-hoc codesign)

## [1.0.1] - 2026-09-05

- Drop unused Rust types, functions, and the `sha1` crate
- Linux packages: `.deb`, `.rpm`, `.AppImage` (x64 + ARM64); Arch `PKGBUILD`
- macOS ad-hoc sign so Gatekeeper does not report a damaged app; `xattr -cr` fallback in README

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
