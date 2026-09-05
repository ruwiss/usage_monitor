# Privacy Policy

**Usage Monitor** is a local desktop app that shows quota for Claude, Codex, Grok, and 9Router. It runs on Windows and Linux.

## Data Collection

This application does **not** collect, store, or transmit personal data to us. There is no analytics, advertising, or telemetry.

## Network Communication

Only the selected source is contacted:

- **Claude** — `api.anthropic.com` (usage + profile)
- **Codex** — `chatgpt.com` (usage) and `auth.openai.com` (token refresh)
- **Grok** — `cli-chat-proxy.grok.com` (billing + user), `auth.x.ai` (token refresh), optionally `grok.com` (weekly credits)
- **9Router** — local origin from settings, default `http://localhost:20128`
- **Custom** — the HTTPS URL you typed

TLS is verified against the OS certificate store (Windows: `truststore`). A corporate TLS-inspecting proxy is trusted the same way the browser trusts it.

## Credentials

The app reads existing CLI logins. It does not ask you for a password.

| Source | File | Use |
|---|---|---|
| Claude | `~/.claude/.credentials.json` | `Authorization` header to Anthropic |
| Codex | `~/.codex/auth.json` | Bearer token; may rewrite `access_token` after refresh |
| Grok | `~/.grok/auth.json` | Bearer token; may rewrite `key` / `expires_at` after refresh |
| 9Router | none locally | 9Router already holds the provider tokens |
| Custom | settings JSON | optional header you stored |

Tokens are never logged or sent to a third party.

## Local Storage

Usage lives in memory and dies with the process.

**Windows** — no data files. Registry under `HKEY_CURRENT_USER`:

- `Software\Classes\AppUserModelId\UsageMonitor.9Router` — toast display name + icon. Rewritten on every start.
- `Software\Microsoft\Windows\CurrentVersion\Run\UsageMonitor` — autostart. Written only when you enable **Start at login**.

**Linux**:

- `~/.config/autostart/usage-monitor.desktop` — autostart, only if enabled
- `$XDG_RUNTIME_DIR/usage-monitor.lock` — single-instance lock, `0600`, cleared at logout

`--config-dir` adds a suffix so each instance has its own autostart and lock.

Optional `usage-monitor-settings.json` is read (and source/custom entries may be written when you change them in Settings). Search order: custom config dir, next to the EXE / project root, then `~/.claude/`.

## Claude Code side effect

If the **Claude** source returns 401, the app runs `claude update` so the Claude CLI can renew its own token. That command may also install a newer Claude Code. Codex and Grok refresh OAuth themselves and do not run a CLI updater.

## Contact

Open an issue on this project's repository.
