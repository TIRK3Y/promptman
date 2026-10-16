# PromptMan test plan

Status key:

- **pass:** run and passing (the command is given).
- **planned:** not written yet.
- **manual:** checked by hand in a running build.
- **untested:** not checked at all.

Nothing below is marked pass without having been run.

## How to run

```
cargo test                                               # unit, GUI and CLI end-to-end tests
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo deny check advisories                              # needs: cargo install cargo-deny
```

## Automated tests (127, all pass on 2026-10-16)

`cargo test` runs 119 library tests, 2 CLI unit tests and 6 CLI end-to-end tests. No test calls a real provider: HTTP goes to a local mock server or to `127.0.0.1:9` (connection refused). GUI tests draw every screen headless at 360×640, 800×600, 1280×800 and 1920×1080 in both themes.

| Area | Module | Tests | What they cover |
|---|---|---|---|
| Streaming | `core_engine::llm` | 30 | Request shape and stream parsing for all four protocols; retries (429/5xx, `Retry-After`, give-up after two); idle timeout; a server that never answers; disconnects; Stop; keep-alive and junk lines; `"error": null`; truncation notes |
| Engine | `core_engine::engine` | 11 | Budget hard stop, monthly spend, base URLs, `run_all` order and per-model Stop, stopped/interrupted accounting, Optimize, `PROMPTMAN_DATA_DIR` |
| Database | `core_engine::db` | 10 | Runs, ratings, stats, prompt versions, chat storage, settings, analytics queries, history order, CSV export escaping, old-ledger import |
| Checks and CSV | `core_engine::evals` | 9 | Templates, checks, judge parsing, rewrite clean-up, CSV quoting, Unicode and malformed CSV |
| Tokens and prices | `core_engine::token_auditor` | 6 | Counting, costs, pricing file defaults and fallback |
| Ollama | `core_engine::local_bridge` | 4 | Status, offline, pull progress and errors, delete, preload |
| Other core | `chat`, `providers`, `secrets`, `hardware` | 10 | Context window, model refs, empty env keys, damaged vault, sampler shutdown |
| App shell | `app` | 4 | Every tab at every size in both themes, last tab remembered, saved run opens in Compare, data refresh |
| Chat | `gui::chat` | 14 | Send, stop, edit and resend, delete, regenerate, rename, export, no edits while streaming, stale Optimize results, context estimate, emptied conversation |
| Compare | `gui::compare` | 12 | Runs, per-model and full Stop, judge mapping and skipping empty answers, exports of the run on screen, Composer saved on change, Differences with one answer |
| Batch, Library, Analytics, Settings | `gui::*` | 9 | Rows, failures, Stop, stale events, rendering |
| CLI | `promptman-cli`, `tests/cli.rs` | 8 | Flags; exit codes 0/1/2; JSONL output; CSV rows; header-only CSV; `--no-save`; unreachable model; `models` |

## Planned tests by stage

| ID | Scenario | Expected result | Stage | Status |
|---|---|---|---|---|
| T1 | Drop a running hardware sampler | Thread exits | 1 | pass: `hardware::dropping_a_sampler_stops_its_thread` |
| T2 | Corrupt `vault.enc` (bad nonce length, non-hex text) | Error returned, no panic, file kept | 1 | pass: `secrets::damaged_vault_is_an_error_not_a_panic` |
| T3 | Invalid `pricing.json` at startup | Defaults used, notice shown | 1 | pass: `token_auditor::broken_pricing_file_falls_back` |
| T4 | Settings JSON missing a field / with an extra field | Loads; missing fields take defaults | 1 | pass: `engine::budget_tolerates_missing_and_unknown_fields` |
| T5 | CLI with a header-only CSV | Exit code 2 with a message | 1 | pass: `promptman-cli::csv_without_rows_is_an_error` |
| T6 | Stream idle timeout | Error after the idle limit; partial text kept | 2 | pass: `llm::stalled_stream_times_out_and_keeps_partial_text` |
| T7 | Mid-stream disconnect | Partial text kept, error recorded | 2 | pass: `llm::dropped_connection_keeps_partial_text`, `engine::interrupted_reply_keeps_text_and_reports_the_error` |
| T8 | Keep-alive / blank `data:` lines | Ignored | 2 | pass: `llm::keepalive_and_junk_lines_are_ignored` |
| T9 | 429 then 200 before the first byte | One retry, a single clean output | 2 | pass: `llm::rate_limit_is_retried_once_with_clean_output`, `llm::client_errors_are_not_retried` |
| T10 | Compare: one model fails, others succeed; Stop the whole run | Per-model status correct; Stop ends all requests | 2 | manual (per-model Stop, live, mock server); `engine::stopped_reply_keeps_text_and_estimates_tokens` |
| T11 | Schema upgrade from today's DB | All rows kept, `user_version` set | 3 | planned |
| T12 | Write failure (read-only DB file) | Error notice, no false "saved" | 3 | planned |
| T13 | Backup → restore | Relationships and row counts intact; `PRAGMA integrity_check` returns ok | 3 | planned |
| T14 | Hard stop with an unreadable spend value | Cloud requests blocked (fail closed) | 4 | planned |
| T15 | Unknown price | Shown as unknown, never `$0` | 4 | planned |
| T16 | CSV export with `=`, `+`, `-`, `@` cells, quotes, newlines, Unicode | Valid CSV; formula cells neutralised | 5 | partial: quotes, newlines and Unicode pass (`db::csv_export_escapes_awkward_text`); formula neutralising planned |
| T17 | Analytics `buckets()` daily/weekly with gaps | Correct bucket count and sums | 5 | pass: `analytics::daily_and_weekly_buckets` |
| T18 | Judge reply with reordered answers | Scores map by label | 5 | pass: `compare::judge_scores_land_on_the_right_answers` |

## Manual checks done this session

| Check | Result | Status |
|---|---|---|
| Every tab renders (dark theme, 1920 px) | OK | manual |
| Composer at 305 px wide, with egui's overflow debugger | No overflowing widgets after the fix | manual |
| Panel no longer grows by itself (drag to 450–510 px, wait) | Width stays put | manual |
| Chat: send, error reply, edit → Save & resend UI, rename/export/delete buttons | OK (no API key, so only the error path) | manual |
| Compare run with no keys and Ollama offline | Clear per-model errors | manual |
| CLI: `models`, offline run (exit 1), missing args (exit 2) | As expected | manual |
| NSIS installer build | Built; not installed or launched from the installer | manual |

## Manual checks done in Stage 2 (isolated profile + local mock server)

| Check | Result | Status |
|---|---|---|
| CLI streamed run with `--contains` | pass, exit 0 | manual |
| Chat: live streaming, Stop keeps partial text with estimated stats | OK; DB has 1 user + 1 assistant message, 1 chat run | manual |
| Optimize card with a real (mock) reply | Both prompts shown side by side, 20 → 15 tokens | manual |
| Compare: per-model Stop | Stopped model's result shows immediately; other keeps streaming | manual |
| Compare at a 922×576 window | No header overlap; metrics not stretched; Judge row wraps | manual |

## Untested so far

- **Not run:** real provider calls (no API keys on this machine, Ollama not installed), and the Optimize suggestion card with a real reply.
- **Not checked:**
  - light theme on the newer screens
  - keyboard-only navigation
  - high-DPI at other scales
  - macOS and Linux
  - installing from the installer and upgrading over an older version
  - long chats and large batches
- **Not run end to end:** keychain behaviour, and the legacy vault migration on this machine (there's no old vault file here).
