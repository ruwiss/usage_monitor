# Claude browser sign-in — finish wiring (bounded task)

Previous agent wrote `src-tauri/src/claude_login.rs` but never wired it in.
Plan (approved in chat):

1. `src-tauri/src/lib.rs`: `mod claude_login;` + three commands
   (`claude_cli_sign_in`, `claude_cli_sign_out`, `claude_cli_cancel`) + register.
2. `ui/popup.html` + `ui/popup.js`: main Claude button = CLI browser sign-in,
   with "waiting" state + Cancel; keep app-in-app claude-web as fallback link.
3. Locale keys for the CLI flow across all 14 locale files.
4. `cargo test` + build.

- [ ] 1. Backend wiring
- [ ] 2. UI
- [ ] 3. Locales
- [ ] 4. Tests + build
