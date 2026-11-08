# Claude browser sign-in — finish wiring (bounded task)

Previous agent wrote `src-tauri/src/claude_login.rs` but never wired it in.
Plan (approved in chat):

1. `src-tauri/src/lib.rs`: `mod claude_login;` + three commands
   (`claude_cli_sign_in`, `claude_cli_sign_out`, `claude_cli_cancel`) + register.
2. `ui/popup.html` + `ui/popup.js`: main Claude button = CLI browser sign-in,
   with "waiting" state + Cancel; keep app-in-app claude-web as fallback link.
3. Locale keys for the CLI flow across all 14 locale files.
4. `cargo test` + build.

- [x] 1. Backend wiring — `mod claude_login;` plus `claude_cli_available`,
  `claude_cli_sign_in`, `claude_cli_sign_out` and `claude_cli_cancel`
  registered in `lib.rs`
- [x] 2. UI — primary button runs the CLI browser login with a waiting state
  and Cancel; the in-app claude-web login stays as a fallback link
- [x] 3. Locales — 13 new keys in all 14 locale files
- [x] 4. Tests — `cargo test` passes (145 tests); a release bundle was not
  rebuilt in this task
