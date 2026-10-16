# PromptMan implementation plan

Based on `AUDIT_REPORT.md`; finding IDs refer to it. Each stage is small, can be reviewed on its own, and ends with this check:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

plus any targeted test the stage adds. A stage is marked done only with that evidence.

Every stage preserves existing behaviour and user data. Owner approval is needed where noted.

---

## Stage 1: repository safety, crash and leak fixes (no data-format change). DONE 2026-10-15

Evidence: fmt, clippy `-D warnings` and `cargo test` (29 passed) all pass, and a fresh release build was smoke-tested. See `CHANGELOG.md`.

| Item | Change | Done when | Test |
|---|---|---|---|
| G2 | `.gitignore`: add `node_modules/`, `dist/`, `*.log`. | `git status` no longer offers `node_modules/` or `dist/`. | `git status --short` |
| G5 | Add `rustfmt.toml` (`max_width = 120`, matching the existing style) and run `cargo fmt`. | `cargo fmt --check` passes. | fmt check |
| G6 | Add `rust-version` to `Cargo.toml`. | `cargo check` passes; the value is the lowest version the code compiles with. | build |
| D1 | `impl Drop for Sampler`. | Dropping a running sampler joins its thread. | new unit test |
| D4 | `ChatState::stop` checks `is_finished()` *before* taking `streaming`. | A reply that finished just before Stop is shown. | code review (GUI is not unit-testable today) |
| R5 | Vault decrypt validates nonce length and hex input and returns errors instead of panicking. Verification reads only the keychain. Keys the keychain already has are left alone. | A corrupt `vault.enc` gives a notice, not a crash. | new unit tests for hex/nonce validation |
| R6 | An invalid `pricing.json` at startup falls back to the defaults (in memory, the file is left untouched) and shows a notice. | The app starts with a broken pricing file. | new unit test |
| R10 | The analytics x-axis formatter uses checked date arithmetic. | Extreme zoom can't panic. | code review |
| D15 | `#[serde(default)]` on saved settings structs (`Budget`, chat `Saved`, composer `Saved`), each with a `Default`. | Older or newer settings JSON still loads, keeping the fields it has. | new unit test (`Budget`) |
| D17 | The CLI fails (exit 2) on a CSV with no data rows. | `--csv` with headers only exits 2 with a message. | new unit test |

**Rollback:** each item is a separate, small diff; revert per file.

## Stage 2: streaming robustness. DONE 2026-10-15

Evidence: fmt, clippy `-D warnings` and `cargo test` (37 passed) all pass, plus a live check on a mock server. See `CHANGELOG.md`.

| Item | Change | Done when |
|---|---|---|
| D2 | Remove the 600 s total timeout. Add a 120 s *idle* timeout between chunks and a connect timeout. `stream_chat` returns partial text with the error. Chat and Compare keep the partial text, with the error shown below it. | A failed or timed-out stream keeps its partial output. Mock tests for an idle timeout and a mid-stream error. |
| R14 | Skip empty or unparseable `data:` lines instead of failing the stream. | A keep-alive line doesn't break a reply. Unit test. |
| R11 | Bounded retry (up to 2 attempts, exponential backoff, honouring `Retry-After`) for 429/5xx **before the first byte only**. | Retries never mix outputs. Mock test. |
| D3 | Compare Stop for the whole run and per model. `run_all` keeps its task handles (`JoinSet`). | Stop ends all requests; partial output is kept. |
| D5/D6 | All persistence on the UI side after Done or Stop. Stopped and failed requests record *estimated* tokens and cost, flagged as estimates. | No duplicate replies; the budget counts partial spend. |

**Risk:** touches the core request path. Mitigated with a local mock HTTP server in tests, so no real provider is needed.

## Stage 3: data safety (owner approval needed for the schema change)

- **R1:** `PRAGMA user_version` with ordered, transactional migrations. Version 1 is today's schema, so existing databases are adopted as-is, with no data rewrite. Test: a database created by the current code opens, upgrades and keeps all rows.
- **R2:** WAL journal, a `busy_timeout` of 5 s, transactions for multi-statement writes, and CSV export done outside the DB lock.
- **R3:** every failed write becomes a visible notice.
- **R4:** legacy import in one transaction, reporting parse errors.
- Backup: "Back up database" in Settings → Data, using SQLite's backup API to a file the user chooses. Restore is documented.

## Stage 4: budget correctness

- **D7:**
  - A per-endpoint "billable" flag for the custom OpenAI-compatible provider (default: local/free, as today).
  - The hard stop fails closed when spend can't be read.
  - Document CLI `--no-save`.
- **D8:** count Gemini thinking tokens if the API reports them (verify against the docs first).
- Unknown prices stay "unknown", never `$0`. This already holds; add a test to keep it.

## Stage 5: UI/UX consistency and per-frame cost

- **U2:** confirm before every destructive action (Ollama delete, key remove, message delete, regenerate, library load over an unsaved draft). Two-click confirms time out after 4 s.
- **U4:** typed notices (info/error), auto-dismiss for info after 6 s, capped at 3.
- **U5:** one page-header component on every tab.
- **U3, U7:** tooltips everywhere; keyboard focus on custom rows (where egui allows).
- **R16, R17:** cache DB-derived values and tokenizer/diff results; refresh on change, not every frame.
- **R18:** virtualised lists for chat history, batch grid and history (`ScrollArea::show_rows`) where measured to help.
- **D9–D14, D16, D18:** the remaining Compare, Batch, Analytics and Chat correctness items.
- **U6, U8:** re-check narrow windows and the light theme with screenshots.

## Stage 6: open-source release readiness (owner decisions needed)

- **Owner decides:** LICENSE file plus `Cargo.toml` metadata.
- `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`, issue and PR templates.
- CI on GitHub Actions for Windows, macOS and Linux:
  - fmt, clippy `-D warnings`, test, release build, `cargo deny check advisories`
  - artifact-size reporting
- **Owner approves:** remove or archive the legacy Tauri/React files (G7) and the unused manifests (G8).
- README: the privacy section (what stays local vs what is sent to providers), troubleshooting, real screenshots, and DB backup/upgrade guidance.
- **G9:** structured `tracing` diagnostics with secret redaction, off by default for prompt content.

---

## Not planned (needs a clear user need first)

Plugin SDK, semantic search, shareable reports, command palette, saved evaluation scenarios. These are listed in the requirements as P1/P2 ideas. They come after the stages above, and only with confirmed demand.
