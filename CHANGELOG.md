# Changelog

This file lists changes that have actually been made. Finding IDs (D1, R5, …) refer to [docs/AUDIT_REPORT.md](docs/AUDIT_REPORT.md).

## Unreleased

### Open-source release preparation (2026-10-16)

- Released under the MIT License.
- New README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY and SUPPORT files, `docs/ARCHITECTURE.md`, issue and pull request templates, and a CI workflow (format, clippy and tests on Windows, macOS and Linux).
- Project reports moved to `docs/`.
- Removed the unused legacy Tauri and React frontend (`src_tauri/`, `src_frontend/`, `node_modules/`, `dist/`, `package*.json`, `tsconfig.json`, `vite.config.ts`).
- macOS bundle metadata now names the app PromptMan.

### Cross-check fixes and test coverage (2026-10-16)

Fixed (each has a regression test):

- A server that accepts the connection but never answers no longer hangs Chat, Compare, Batch or the CLI: it's retried, then fails after the 2-minute idle limit.
- `"error": null` in a stream chunk is no longer treated as an error.
- An empty `*_API_KEY` environment variable no longer counts as a connected key.
- Chat: Save & resend is disabled while a reply streams (it used to leave an orphan reply); sending discards a pending Optimize result for the old draft; an emptied conversation keeps its header (rename, export, delete); exports keep partial text above the error; the context-token estimate updates when message text changes.
- A request stopped before the model started costs $0 and shows no "0 tokens" stats.
- Compare: exports describe the run on screen (its system prompt and parameters, not the Composer's current ones); Markdown export includes parameters and partial text of failed answers; the judge skips empty or stopped answers; the Differences view falls back to Answers with fewer than two models; the Composer is saved whenever it changes.
- Batch: the progress count says "stopped" after Stop.
- A flaky Batch test that shared one temp file between parallel tests.

Added `tests/cli.rs` (end-to-end CLI tests against a local mock server). README and guide corrected where they didn't match the app.

Not changed: Chat replies don't show `~` on estimated tokens because chat messages don't store that flag; adding it needs a database change (Stage 3, awaiting approval). The note on stopped or interrupted replies says the numbers are estimated.

### Chat tidy-up (2026-10-15)

**Changed (second pass)**
- **Model choice moved to Settings → Chat.** The Chat header no longer has the provider/model pickers or the "Chat settings" button. A new **Model** row heads Settings → Chat, saying the choice applies to your next message.
- **An empty chat shows only the greeting, centred in the pane, with no header.** An open chat's header is the title, Rename/Export/Delete, and one muted line with the model and totals.
- **Analytics: "Latency per answer" says "No completed answers in this period"** instead of drawing an empty grid. This is a code change only; I haven't looked at it in the running app.

**Changed**
- **Empty chat shows only the greeting and the current model.**
  - "What can I help with?" and "Chatting with …" stay.
  - If that model can't answer yet, one line says so and links to Settings ("Anthropic isn't connected yet · Add an API key in Settings", or how to start Ollama).
  - Moved out of the chat: the "Saving tokens" card and the in-chat API-key banner.
- **Settings → Chat now explains the token-saving options** that used to be on the empty chat screen: model switching, concise replies, memory and Optimize. API keys are entered in Settings → Providers.
- **Your messages show a pencil icon instead of three links.**
  - Clicking it reveals Edit · Copy · Delete, and clicking again hides them.
  - The icon is drawn, not taken from a font, so it renders everywhere.
  - Replies keep Copy / Delete / Regenerate as before.

**Verified with:**
- **Checks:** fmt, clippy `-D warnings`, `cargo test` (37 passed).
- **Screenshots** of the preview on a throwaway profile: the empty chat with the connect hint, the pencil icon closed and open, and Settings → Chat.

### Stage 2: streaming reliability (2026-10-15)

**Fixed**
- **Long replies are no longer cut off at 10 minutes** (D2). The 600 s total timeout is gone; a reply only fails after 120 s with no data at all. `engine.rs`, `llm.rs`
- **Text that already arrived is kept when a stream stalls, drops or errors** (D2). Chat and Compare show it, with the error below. `llm.rs`, `engine.rs`, `gui/chat.rs`, `gui/results.rs`
- **Stopping is a clean signal now, not a killed task** (D5, D6). A stopped or interrupted reply is saved once, with its partial text and *estimated* tokens and cost (marked `~` and explained in the note), so the budget counts it. Previously a stopped chat reply recorded no cost and could be saved twice. `engine.rs`, `gui/chat.rs`
- **Compare has Stop buttons** (D3): Stop all in the Composer, and Stop on each card. A stopped model's result shows immediately while the others continue. `gui/compare.rs`, `gui/composer.rs`, `gui/results.rs`
- **Opening a saved run while a comparison is running stops that run first** (D12). Before, it kept billing in the background. `gui/compare.rs`
- **Batch Stop also ends the row in progress**, and Choose CSV… is locked during a run (D13).
  - Stopped cells are marked "stopped before finishing".
  - Not tested in the running app.
  - `gui/batch.rs`
- **Rate limits and server errors (429, 500, 502, 503, 504, 529) and request timeouts are retried up to 2 times** (R11). This only happens before any reply text arrives, so outputs are never mixed. Waits honour `Retry-After` (up to 20 s); otherwise 1 s, then 2 s. Connection-refused is not retried. A retried reply says so in its note. `llm.rs`
- **Keep-alive and other non-JSON stream lines are skipped** instead of failing the reply (R14). `llm.rs`
- **The hardware sampler no longer blocks an async worker thread when a local run ends** (R15). `engine.rs`
- **Compare cards no longer overlap or stretch text in narrow panes.**
  - Header buttons take their space first and the model name truncates.
  - Very narrow cards use two header rows.
  - Metrics are no longer justified letter by letter.
  - The Judge row wraps on narrow panes.
  - `gui/results.rs`
- **Analytics shows "Nothing spent in this period" instead of an empty −10…+10 chart.** `gui/analytics.rs`

**Added**
- **`PROMPTMAN_DATA_DIR` environment variable** to use a separate data folder (portable installs, testing, CI). `engine.rs`
- **`Prompt` struct** in `llm.rs`, bundling system prompt, messages and parameters; used by every request path.
- **`RunHooks`** (per-chunk and per-model-done callbacks) for `Engine::run_all`.
- **Dependency:** `tokio-util` 0.7, for `CancellationToken`. It was already in the build through reqwest.

**Tests added (8):** all run without a real provider, using a local scripted HTTP server.
- `llm::keepalive_and_junk_lines_are_ignored`
- `llm::rate_limit_is_retried_once_with_clean_output`
- `llm::client_errors_are_not_retried`
- `llm::stalled_stream_times_out_and_keeps_partial_text`
- `llm::dropped_connection_keeps_partial_text`
- `llm::stop_keeps_partial_text`
- `engine::stopped_reply_keeps_text_and_estimates_tokens`
- `engine::interrupted_reply_keeps_text_and_reports_the_error`

**Verified with:**
- **Automated checks:** `cargo fmt --check` passes; `cargo clippy --all-targets --all-features -- -D warnings` passes; `cargo test` passes 37 tests (35 library + 2 CLI), with the library tests repeated 3 times and stable.
- **Live check, on an isolated test profile with a local mock streaming server; your data and keys were not used:**
  - The CLI completed a streamed run with checks.
  - Chat streamed live; Stop kept the partial reply with estimated stats. The database held exactly one user and one assistant message and one chat run (no duplicates).
  - The Optimize card showed both prompts side by side (20 → 15 tokens).
  - In Compare, stopping one of two models showed its result immediately while the other kept streaming. Narrow cards (922×576 window) no longer overlap.

**Not verified:**
- Real providers (no keys on this machine).
- Batch Stop in the running app.
- Compare "Stop all" in the running app (it uses the same cancellation as per-card Stop).
- **Known cosmetic issue:** at very small window heights (about 576 px), a stopped card's text can run slightly past the card's bottom edge.

### Stage 1: crash, leak and repository-safety fixes (2026-10-15)

**Fixed**
- **Stopping a reply from a local model no longer leaks a background thread** (D1). The hardware sampler now stops itself when it's dropped. `src/core_engine/hardware.rs`
- **Clicking Stop just as a chat reply finishes no longer hides that reply** (D4). Switching chats or editing a message now stops listening for the old reply. `src/gui/chat.rs`
- **A damaged legacy `vault.enc` no longer crashes PromptMan at startup** (R5). `src/core_engine/secrets.rs`
  - Bad hex or nonce data is now an error.
  - Keys already in the keychain are no longer overwritten by the old vault.
  - Migrated keys are verified against the keychain only, so an environment variable can't mask a failed write.
- **A broken `pricing.json` no longer stops the app from starting** (R6). Built-in prices are used and a notice explains why; the file itself is left untouched. `src/core_engine/token_auditor.rs`, `engine.rs`
- **Zooming far out on the Analytics latency chart can no longer overflow date arithmetic** (R10). `src/gui/analytics.rs`
- **Saved settings written by an older or newer version still load** (D15). Missing fields take their defaults, so the budget, chat options and Composer are no longer reset. `engine.rs`, `gui/chat.rs`, `gui/compare.rs`
- **`promptman-cli --csv` with a header-only file now exits with code 2** (D17). Previously it passed with 0. `src/bin/promptman-cli.rs`

**Repository**
- **`.gitignore`:** ignores `node_modules/`, `dist/` and `*.log`, so the legacy frontend output can't be committed by accident (G2).
- **Formatting:** added `rustfmt.toml` (`max_width = 120`) and formatted all sources; `cargo fmt --check` now passes (G5).
- **Rust version:** declared `rust-version = "1.95"`, the minimum required by `egui_plot` 0.37 and `egui_commonmark` 0.25 (G6). This is based on their published metadata; Rust 1.95 itself wasn't tested.

**Tests added (6):**
- `hardware::dropping_a_sampler_stops_its_thread`
- `hardware::finish_returns_a_reading`
- `secrets::damaged_vault_is_an_error_not_a_panic`
- `token_auditor::broken_pricing_file_falls_back`
- `engine::budget_tolerates_missing_and_unknown_fields`
- `promptman-cli::csv_without_rows_is_an_error`

**Verified with:**
- `cargo fmt --check`: pass
- `cargo clippy --all-targets --all-features -- -D warnings`: pass
- `cargo test`: 29 passed, 0 failed
- A fresh release build launched, kept its window open, and reopened the last tab; `promptman-cli models` ran.

**Not verified:**
- The D4 race was not reproduced in a running app; it was checked by reading the code.
- R10 was not reproduced either.
