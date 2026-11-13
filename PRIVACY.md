# Privacy Policy

**Usage Monitor** is a local desktop app that shows quota for Claude, Codex, Grok, Antigravity, OMP, and 9Router. It runs on Windows and Linux.

## Data Collection

This application does **not** collect, store, or transmit personal data to us. There is no analytics, advertising, or telemetry.

## Network Communication

Only the selected source is contacted:

- **Claude** — `api.anthropic.com` (usage + profile)
- **Claude (web)** — only after you sign in via Settings > Sources: `claude.ai` (`/api/organizations`, `/api/organizations/<id>/usage`, `/api/account`), requested from inside the app's own sign-in browser view. The sign-in window may only navigate to `claude.ai`, `claude.com`, `anthropic.com` and Google sign-in domains (`google.com`, `gstatic.com`, `googleusercontent.com`).
- **Codex** — `chatgpt.com` (usage) and `auth.openai.com` (token refresh)
- **Antigravity** — `cloudcode-pa.googleapis.com` (quotas, plan), `oauth2.googleapis.com` (access-token refresh, only when the stored token is expired)
- **Grok** — `cli-chat-proxy.grok.com` (billing + user), `auth.x.ai` (token refresh), optionally `grok.com` (weekly credits)
- **9Router** — local origin from settings, default `http://localhost:20128`
- **OMP** — no remote call from this app; it runs the local `omp usage --json` CLI
- **Custom** — the HTTPS URL you typed
- **OpenRouter** — `openrouter.ai` (`/api/v1/key`, `/api/v1/credits`)
- **GitHub Copilot** — `api.github.com` (`/copilot_internal/user`). The token comes from `gh auth token`, `GH_TOKEN` / `GITHUB_TOKEN`, or a token you added
- **OpenCode Go** — `opencode.ai` (`/zen/go/v1/usage`)
- **z.ai GLM** — `api.z.ai`, or `open.bigmodel.cn` if the key belongs to the mainland platform
- **Kimi Code** — `api.kimi.com`
- **MiniMax** — `api.minimax.io`, or `api.minimaxi.com` as fallback
- **DeepSeek** — `api.deepseek.com`
- **Moonshot** — `api.moonshot.ai`, or `api.moonshot.cn` as fallback

TLS is verified against the OS certificate store (Windows: `truststore`). A corporate TLS-inspecting proxy is trusted the same way the browser trusts it. Certificate checks are only relaxed for custom-source URLs on this machine or a private network (localhost, `*.local`, 127.0.0.0/8, `::1`, 10.x, 172.16-31.x, 192.168.x), for self-signed local routers; every other host is always verified. Redirects are followed only within the same host.

## Credentials

The app reads existing CLI logins. It does not ask you for a password.

| Source | File | Use |
|---|---|---|
| Claude | `~/.claude/.credentials.json` | `Authorization` header to Anthropic |
| Claude (web) | the app's own webview profile: `%LOCALAPPDATA%\com.9router.usage-monitor\claude-web\` (Windows; the platform data directory elsewhere, a WebKit data store on macOS) | The claude.ai browser session (cookies) lives only there, managed by the web engine. The app never reads cookie values, never writes them elsewhere and never logs them; it only checks that a `sessionKey` cookie exists to detect a finished sign-in, and keeps a `signed-in` marker file next to the profile. **Sign out** in Settings > Sources clears the webview data and deletes the directory. |
| Codex | `~/.codex/auth.json` | Bearer token; may rewrite `access_token` after refresh |
| Antigravity (Windows only) | Windows Credential Manager entry `gemini:antigravity` | Read-only: email, access/refresh token. A refreshed token is kept in memory only; the entry is never written. To refresh, the OAuth client id/secret is read from the installed `%LOCALAPPDATA%\agy\bin\agy.exe` and `%LOCALAPPDATA%\Programs\Antigravity\**\language_server*.exe` |
| Grok | `~/.grok/auth.json` | Bearer token; may rewrite `key` / `expires_at` after refresh |
| 9Router | none locally | 9Router already holds the provider tokens |
| OMP | none locally | spawns `omp usage --json`; OMP keeps its own logins |
| Custom | settings JSON | optional header you stored |
| Provider keys (OpenRouter, z.ai, Kimi, MiniMax, DeepSeek, Moonshot, optional Copilot / OpenCode Go) | settings JSON, plain text | key you typed in Settings; sent only to that provider's API host; never shown back in the UI or logged |
| GitHub Copilot (auto) | none stored | runs `gh auth token` (no console window) or reads `GH_TOKEN` / `GITHUB_TOKEN`; detection only checks that `gh`'s `hosts.yml` exists |
| OpenCode Go (auto) | `~/.local/share/opencode/auth.json` | read-only; the `opencode-go` key is used as a Bearer token |

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

## Local provider updates

When `provider_auto_update` is enabled (default), a separate worker checks visible local Claude, Codex, Grok, OMP and Antigravity sources once per day. The executable must match an existing installation owned by npm, pnpm, Bun, Homebrew, Scoop or winget before that manager is used. Native Claude installations in the standard user directory use `claude update`. The app does not install another copy through a different package manager when detection fails.

This may contact the package manager's configured registries/repositories, Scoop buckets, winget sources and the provider's update service, and may run their normal package scripts and dependency installers. Unknown installations, machine-wide Windows installations requiring elevation and API-only/web sources are skipped. Commands run with no interactive input and bounded timeouts.

Updates are independent of authentication failures. Turn off **Settings → Sources → Automatically update source apps**, or set `provider_auto_update` to `false`, to disable this feature. A started installer is allowed to finish. If saving fails, disabling still takes effect for the current session and the UI explains this. Set `notify_provider_update` to `false` to hide update notifications. Usage Monitor's own `auto_update` setting is separate.

## Contact

Open an issue on this project's repository.
