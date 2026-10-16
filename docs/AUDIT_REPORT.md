# PromptMan audit report

Date: 2026-10-15. Scope: everything in this folder at the time of the audit (version 1.0.0, branch `master`, no commits yet).

How this audit was done:

- **Measured:** sizes, build times, tests, lint, startup and memory. The exact commands are below.
- **Read:** every file under `src/`, twice: once by the author of the recent changes, and once by an independent reviewer who had not written the code.
- **Viewed:** every screen, in a running build. Screenshots were taken during this session; none are committed.

Statuses used below:

- **confirmed:** verified by reading the code or running it.
- **suspected:** plausible but not proven.
- **fixed / verified:** only after a change and a passing check.

---

## 1. What the project actually is

| Item | Finding |
|---|---|
| Language / build | Rust 2021, a single crate, Cargo. Toolchain used: rustc 1.99.0, cargo 1.99.0. No workspace. |
| Desktop UI | `eframe`/`egui` 0.36 with the `glow` (OpenGL) renderer. Native window; no web view. |
| Binaries | `promptman` (GUI, `src/main.rs`), `promptman-cli` (`src/bin/promptman-cli.rs`). Shared library in `src/lib.rs`. |
| Async / network | `tokio` multi-thread runtime; `reqwest` 0.12 streaming (`src/core_engine/llm.rs`). |
| Providers | OpenAI-style (OpenAI, Groq, Mistral, any OpenAI-compatible server), Anthropic Messages, Gemini, Ollama. One streaming function handles all of them (`llm.rs::stream_chat`). |
| Storage | SQLite through `rusqlite` (bundled), a single file `promptman.db` (`src/core_engine/db.rs`). Prices in `pricing.json`. Both live in the OS app-data folder. |
| Secrets | `keyring` 4 (OS keychain), falling back to `<PROVIDER>_API_KEY` environment variables (`src/core_engine/secrets.rs`). |
| Packaging | `cargo-packager` metadata in `Cargo.toml`; an NSIS installer was built successfully earlier this session (unsigned). |
| Tests | 23 unit tests in 7 files. No integration tests, no HTTP mocks, no GUI tests. |
| CI | None. |
| Legacy code | `src_tauri/`, `src_frontend/`, `node_modules/`, `dist/`, `package.json`, `package-lock.json`, `tsconfig.json`, `vite.config.ts` are an older Tauri + React version. Nothing in the Rust crate references them. |

### Source map

```
src/
├── main.rs                  GUI entry: tokio runtime, engine, window
├── lib.rs
├── app.rs                   Shell: top bar, tabs, notices, event routing
├── bin/promptman-cli.rs     CLI
├── core_engine/             No GUI code
│   ├── engine.rs            run_one / run_all, budget, optimize_prompt
│   ├── llm.rs               Streaming for 4 wire protocols
│   ├── providers.rs         Provider list, ModelRef
│   ├── db.rs                Schema, runs, chat, library, settings, analytics
│   ├── secrets.rs           Keychain + legacy vault migration
│   ├── token_auditor.rs     tiktoken counts, pricing.json
│   ├── evals.rs             Templates, CSV, checks, judge, rewrite helpers
│   ├── chat.rs              Chat context window, concise prompt, titles
│   ├── hardware.rs          CPU/RAM/GPU sampler for local runs
│   └── local_bridge.rs      Ollama status/pull/delete/preload
└── gui/                     One module per tab + theme.rs (palette, widgets)
```

6,695 lines of Rust in 23 files. The largest are `gui/chat.rs` (951), `core_engine/db.rs` (917) and `gui/compare.rs` (527).

---

## 2. Disk usage: where the space goes

Measured with `du -sh` and `git count-objects -vH` (Git Bash on Windows).

| Path | Size | Kind | Safe to remove? |
|---|---|---|---|
| `target/debug` | 4.2 GB | Rust debug build output (3.4 GB `deps`, mostly debug info; 651 MB `incremental`) | Yes, rebuilt by `cargo build` |
| `target/release` | 872 MB | Rust release build output, including a 12 MB installer | Yes, rebuilt by `cargo build --release` |
| `node_modules` | 70 MB | npm packages for the unused Tauri/React frontend | Yes if the legacy frontend is dropped (needs owner approval) |
| `.git` | 49 MB | About 36 MB of loose objects from a `git add` that was never committed, plus 4 temp "garbage" objects | `git gc --prune=now` would clean it (not done) |
| `src_tauri`, `src_frontend`, `dist` | under 1 MB | Legacy source | Owner decision |
| Source (`src`, `assets`, manifests) | about 0.4 MB | The product | Keep |
| User data (`%APPDATA%\promptman`) | 116 KB | DB, pricing, imported ledger | Keep: user data |

**Total right now: about 5.4 GB.** About 99% is reproducible Rust build output. There are no model weights, large binaries, databases or logs in the folder, and no git history (zero commits).

**About the reported 11+ GB:** I can't reconstruct the exact peak. Known contributors since the project started:

- `target/` building up across two eframe versions (0.29 and 0.36).
- Two temporary `target/preview` builds of about 1.7 GB each that I made during earlier UI work and deleted.
- A temporary 1.3 GB timing build made and deleted during this audit.

Debug info is the main lever: `deps` is 3.4 GB because every dependency is built with full debug info.

---

## 3. Baseline checks (run before any change)

| Check | Command | Result |
|---|---|---|
| Unit tests | `cargo test` | **Pass**: 22 library + 1 CLI = 23 tests, 0 failed |
| Lint (strict) | `cargo clippy --all-targets --all-features -- -D warnings` | **Pass**, no warnings |
| Format | `cargo fmt --check` | **Fail**: 276 diff hunks across all 23 files (never formatted) |
| Security advisories | `cargo deny check advisories` (RustSec DB) | **Pass**: "advisories ok" |
| Licenses | `cargo deny list -l license` | All dependencies are permissive or offer a permissive option (see §6) |
| Cold release build | `cargo build --release` into an empty target dir | **4 min 38 s**, success |
| Release binary size | — | GUI 22.2 MB, CLI 12.0 MB, installer 12 MB |
| GUI startup (release, warm) | time to first window, 3 runs | 95 ms, 77 ms (the first run of a new binary: 4.8 s, probably antivirus scanning; not verified) |
| GUI memory | working set / private bytes 5 s after start | 138 MB / 92 MB |
| CLI startup (`promptman-cli models`) | 3 warm runs | 25–38 ms (first run 3.2 s, same likely cause) |

Details are in `PERFORMANCE_BASELINE.md`.

---

## 4. Findings

Format of each finding:

- **ID · Severity:** P0 (blocks the release), P1 (should fix before the release), P2 (later).
- **Evidence:** file:line.
- **Impact:** what goes wrong for the user.
- **Fix:** the recommended change.
- **Done when:** the acceptance criteria.
- **Tests:** whether existing tests cover it.
- **Status.**

### 4.1 Observed defects

**D1 · P1 · Stopping a local-model reply leaks a background thread**
- **Evidence:** `core_engine/hardware.rs:23-53` (`Sampler` has no `Drop`); `engine.rs:146,150`; `gui/chat.rs` `stop()`.
- **Impact:** every Stop (or switching chats mid-reply) on Ollama or a custom server leaves a thread polling sysinfo and spawning `nvidia-smi` every 500 ms until the app closes.
- **Fix:** add `Drop` for `Sampler` that signals the thread to stop.
- **Done when:** dropping a running sampler ends its thread.
- **Tests:** none. Add a unit test.
- **Status:** fixed, verified by unit test

**D2 · P1 · Long replies are cut off at 10 minutes, and partial text is lost on any stream error**
- **Evidence:** `engine.rs:62-65` (a 600 s total timeout); `llm.rs` `stream_chat` returns `Err` and drops `out.text`; `gui/compare.rs` replaces the visible text with the empty output.
- **Impact:** slow local models or long generations fail, and the text already shown disappears.
- **Fix:** use an idle timeout between chunks instead of a total timeout, and return the partial text with the error.
- **Done when:** a stream that keeps sending data isn't killed, and a failed stream keeps its partial text in Chat and Compare.
- **Tests:** none.
- **Status:** fixed in Stage 2; verified by unit tests (mock server) and live check

**D3 · P1 · Compare has no Stop**
- **Evidence:** `gui/composer.rs` (the button only says "Running…"); `engine.rs::run_all` spawns detached tasks.
- **Impact:** a hung provider blocks the tab, and the user can't cancel a paid run.
- **Fix:** keep the task handles, and add Stop for the whole run and for each model.
- **Done when:** Stop ends all in-flight requests and keeps their partial output.
- **Tests:** none.
- **Status:** fixed in Stage 2 (Stop all and per-model Stop); per-model Stop verified live

**D4 · P2 · Clicking Stop just as a reply finishes hides that reply**
- **Evidence:** `gui/chat.rs` `stop()` takes `streaming` before checking `is_finished()`, so the `ChatDone` that follows is ignored.
- **Impact:** the reply is saved but missing from the screen and from the next message's context.
- **Done when:** the reply shows after that race.
- **Status:** fixed (checked by code review; the race was not reproduced live)

**D5 · P2 · A Stop/finish race can save two replies**
- **Evidence:** `gui/chat.rs` (the task saves after `run_one`; `stop()` also saves a "Stopped early" message).
- **Impact:** a duplicate reply appears after reopening the chat.
- **Fix:** do all persistence on one side, or check a cancellation flag before saving.
- **Status:** fixed in Stage 2 (one save path, cooperative stop); no duplicate found in a live check

**D6 · P1 · Failed and stopped requests record no tokens or cost**
- **Evidence:** `engine.rs:157-163`; the `stop()` note in `gui/chat.rs`.
- **Impact:** the budget under-counts. Repeatedly stopping replies can get around the hard stop.
- **Fix:** estimate tokens and cost from the input plus the partial output, and mark the figure as an estimate.
- **Status:** fixed in Stage 2 (estimates marked `~`); verified by unit test and live check

**D7 · P1 · Gaps in budget enforcement**
- **Evidence:** `engine.rs:134` (a check-then-act); `providers.rs:72-80`; `token_auditor.rs:79-81`; `engine.rs:89,94`; `bin/promptman-cli.rs` (`--no-save`).
- **Impact:**
  - (a) Parallel requests can overshoot the cap.
  - (b) The "custom" provider is always treated as free and local, even when it points at a paid cloud service.
  - (c) A database read error turns the hard stop off.
  - (d) CLI runs with `--no-save` spend without being recorded.
- **Fix:** add a billable/local setting per custom endpoint; fail closed when the hard stop is on; document `--no-save`.
- **Status:** confirmed.

**D8 · P2 · Gemini thinking tokens are probably not counted**
- **Evidence:** `llm.rs` reads only `candidatesTokenCount`.
- **Status:** suspected (the API behaviour hasn't been verified here).

**D9 · P2 · Exporting a reopened Compare run uses the current Composer's system prompt and parameters**
- **Evidence:** `gui/results.rs` `export_*`; `compare.rs` `open_run`.
- **Status:** confirmed.

**D10 · P2 · Conversation mode in Compare can pair old replies with different models after the model list changes**
- **Evidence:** `composer.rs:21`; `compare.rs:179-192`. Failed turns are sent as "(no response)".
- **Status:** confirmed.

**D11 · P2 · Judge scores are matched by position, not by answer label; a stale judge reply can re-enable the button**
- **Evidence:** `evals.rs` `parse_judge`; `compare.rs` (the `JudgeDone` handler).
- **Status:** confirmed.

**D12 · P2 · Opening a saved run while a comparison is running lets a second run start, while the first keeps billing**
- **Evidence:** `compare.rs` `open_run`.
- **Status:** fixed in Stage 2 (code review)

**D13 · P2 · Batch Stop only takes effect between rows, and "Choose CSV…" stays enabled while a run is going**
- **Evidence:** `gui/batch.rs:76,190,289`.
- **Status:** fixed in Stage 2 (in-flight row is cancelled, CSV locked); not tested live

**D14 · P2 · Chat's "sends ~N tokens" cache keys on the system prompt's length, not its content**
- **Evidence:** `gui/chat.rs` `context_tokens`.
- **Status:** confirmed.

**D15 · P2 · Saved settings structs have no `#[serde(default)]`**
- **Evidence:** `engine.rs` `Budget`; `gui/chat.rs` `Saved`; `gui/compare.rs` `Saved`.
- **Impact:** adding a field in a later version silently resets the user's budget (including the hard stop), chat options and Composer, because parse errors are swallowed in `db.rs` `get_setting`.
- **Status:** fixed, verified by unit test (`Budget`)

**D16 · P2 · Analytics shows "No runs in this period" when a query fails**
- **Evidence:** `gui/analytics.rs:37-39`.
- **Status:** confirmed.

**D17 · P2 · The CLI exits 0 for a header-only CSV**
- **Evidence:** `bin/promptman-cli.rs:137-164`.
- **Impact:** a false pass in CI.
- **Status:** fixed, verified by unit test

**D18 · P2 · Chat rename reads Enter/Esc globally**
- **Evidence:** `gui/chat.rs` `header`.
- **Status:** confirmed.

### 4.2 Potential risks

**R1 · P1 · No database schema versioning**
- **Evidence:** `db.rs` `SCHEMA` uses only `CREATE … IF NOT EXISTS`, with no `PRAGMA user_version`.
- **Impact:** the first release that adds a column breaks saving for existing users, after the paid request has already completed.
- **Fix:** versioned, transactional migrations.
- **Status:** confirmed. Changing the DB format needs owner approval.

**R2 · P2 · No WAL or explicit `busy_timeout`; some multi-statement writes aren't wrapped in a transaction**
- **Evidence:** `db.rs` `add_message`; the chat edit (`update_message` + `delete_messages_after`).
- **Impact:** the GUI and CLI share one file, and `export_csv` holds the DB lock while writing a large file.
- **Status:** confirmed / suspected (the default busy timeout hasn't been verified).

**R3 · P1 · Many failed writes are ignored with `let _ =` or `.ok()`**
- **Evidence:**
  - `engine.rs:193`
  - `gui/batch.rs:89`
  - `gui/chat.rs` (run save, assistant message, regenerate delete)
  - `gui/compare.rs` (judge run and score)
  - settings persistence in several files
- **Impact:** spend or messages silently missing after a disk or DB error.
- **Fix:** surface the error as a notice.
- **Status:** confirmed.

**R4 · P2 · The legacy ledger import skips a corrupt file silently; a failure partway can import duplicates on the next launch**
- **Evidence:** `db.rs` `import_legacy_ledger`.
- **Status:** confirmed.

**R5 · P1 · The legacy vault migration can crash the app at every startup on a corrupt `vault.enc`**
- **Evidence:** `secrets.rs:110` (`Nonce::from_slice` panics); `:99` (byte slicing on non-ASCII input).
- **Other problems:**
  - Its verification step can be fooled by an environment variable.
  - It overwrites newer keychain entries on retry.
- **Status:** fixed (panics and overwrite); verified by unit test

**R6 · P2 · An invalid `pricing.json` stops the app from starting**
- **Evidence:** `token_auditor.rs` via `engine.rs::open`.
- **Note:** Settings → Reload already handles the same error gracefully.
- **Status:** fixed, verified by unit test

**R7 · P2 · Secrets in errors and logs: low exposure**
- **Good:**
  - Keys are only ever sent in headers, never in URLs.
  - There are no `tracing` call sites, so prompts, responses and keys are never logged.
  - Exports contain no keys.
- **Exposure:** error messages include the endpoint URL, which would show credentials only if a user embeds them in a custom URL.
- **Status:** confirmed.

**R8 · P2 · Endpoint URLs aren't validated**
- **Impact:** an `http://` cloud endpoint sends the key in cleartext. On a cross-host redirect, reqwest probably forwards the custom `x-api-key` / `x-goog-api-key` headers.
- **Status:** suspected.

**R9 · P2 · CSV exports don't guard against spreadsheet formulas**
- **Impact:** a cell starting with `=`, `+`, `-` or `@` runs as a formula in Excel.
- **Status:** confirmed.

**R10 · P2 · Analytics latency chart: zooming out far enough can panic**
- **Evidence:** `analytics.rs` x-axis formatter: `since + Duration::days(...)` is unchecked.
- **Status:** fixed (checked date arithmetic; not reproduced live)

**R11 · P2 · No retry or backoff on 429/5xx**
- **Impact:** hurts Batch most.
- **Status:** fixed in Stage 2 (bounded retry before first byte); verified by unit tests

**R12 · P2 · Concurrency is bounded per feature but not globally**
- **Evidence:** Compare runs at most 4 at once; Batch runs one row at a time with at most 4 per row. Features can overlap (see D12).
- **Status:** confirmed.

**R13 · P2 · The Ollama status poll creates a new HTTP client every 15 s, even if Ollama is never used**
- **Status:** confirmed.

**R14 · P2 · One unparseable `data:` line fails a whole stream**
- **Impact:** some OpenAI-compatible servers send keep-alives that would trigger this.
- **Status:** fixed in Stage 2; verified by unit test

**R15 · P2 · The sampler's `finish()` blocks a tokio worker for up to about 0.5 s; CPU and RAM figures are system-wide**
- **Status:** fixed in Stage 2 (`spawn_blocking`); code review

**R16 · P2 · Database queries run on every frame while idle**
- **Evidence:** the Compare blocked-reason check runs 2 queries; the Settings endpoint row runs 7; the budget slider writes on every drag frame.
- **Status:** confirmed.

**R17 · P2 · Some work is recomputed every frame**
- **Evidence:** re-tokenizing (chat input, the suggestion card) and re-diffing (Differences view).
- **Status:** confirmed.

**R18 · P2 · No virtualisation for long chats, large batches or many history rows**
- **Status:** confirmed. The cost hasn't been measured.

### 4.3 UI/UX findings (from running the app this session)

| ID | Sev | Finding | Status |
|---|---|---|---|
| U1 | — | Composer panel grew by itself, and content clipped at narrow widths. **Fixed this session**: content is contained and dropdowns are sized. Checked at 305 px with egui's overflow debugger: no overflowing widgets. | verified |
| U2 | P2 | Several destructive actions don't confirm: delete an Ollama model (multi-GB), remove an API key, delete a chat message, Regenerate (deletes the old reply), Library "Load into Composer" (overwrites the draft), Choose CSV during a run. The two-click confirms never reset. | confirmed |
| U3 | P2 | Custom clickable rows (conversation list, history runs, budget chip, nav tabs) have no keyboard focus or accessible label. | suspected (keyboard navigation not tested) |
| U4 | P2 | Notices pile up with no auto-dismiss. Error vs info is decided by matching the text "Could not". | confirmed |
| U5 | P2 | Page headers aren't consistent: Chat and Results use their own headings, the other tabs use `theme::title`. | confirmed |
| U6 | P2 | The diff header (two 220 px dropdowns) and the chat model picker (340 px) can overflow narrow centre panes at a 1024 px window. | suspected |
| U7 | P2 | No tooltip on the Batch check "×". | confirmed |
| U8 | P2 | Light theme not re-checked on the newer screens (Chat, the new top bar). | not verified |
| U9 | P2 | Compare card header: Stop/Copy buttons covered the provider badge and model name in narrow panes. | fixed in Stage 2, verified live (922 px window) |
| U10 | P2 | Compare card metrics were justified letter by letter in narrow cards. | fixed in Stage 2, verified live |
| U11 | P2 | Spend chart with zero spend drew an empty −10…+10 axis. | fixed in Stage 2 (code only) |
| U12 | P2 | At a window height of about 576 px, a stopped card's text can run slightly past the card's bottom edge. | open |

### 4.4 Release-readiness gaps

| ID | Sev | Gap |
|---|---|---|
| G1 | P1 | No `LICENSE` file, and `Cargo.toml` has no `license`, `repository` or `authors`. The license is the owner's decision. Dependency licenses allow MIT, Apache-2.0 or most other choices (see §6). |
| G2 | P1 | **fixed in Stage 1.** `.gitignore` doesn't exclude `node_modules/` or `dist/`. With zero commits, `git add .` would commit 70 MB of npm packages. |
| G3 | P1 | No CI. |
| G4 | P1 | No `SECURITY.md` (important for an app that handles API keys), `CONTRIBUTING.md`, `CHANGELOG.md` or `CODE_OF_CONDUCT.md`. |
| G5 | P2 | **fixed in Stage 1.** `cargo fmt --check` fails (276 hunks), and there is no `rustfmt.toml`. |
| G6 | P2 | **fixed in Stage 1 (`rust-version = "1.95"`).** No `rust-version` (MSRV) declared; the code needs at least 1.87 (`is_multiple_of`). |
| G7 | P1 | Legacy Tauri/React files contradict the README ("no Node.js") and confuse contributors. `src_tauri/Cargo.toml` is also named `promptman`. Removing them or moving them to an archive branch needs the owner's approval. |
| G8 | P2 | `assets/windows/app.manifest` and `assets/macos/Info.plist` are unused leftovers (the manifest still names "hellorust"). `assets/icons/README.md` describes the Tauri CLI. |
| G9 | P2 | `tracing` is a dependency with no call sites, so there are no diagnostics for bug reports. |
| G10 | P2 | Default model IDs and prices are hard-coded and will go stale (the README documents this). |
| G11 | P2 | The installer is unsigned and there's no auto-update (documented). |

---

## 5. Test coverage map

| Module | Tests | Gaps |
|---|---|---|
| `core_engine/llm.rs` | stream-line parsing for all 4 protocols, error bodies | `stream_chat` itself (no mock server), partial and error paths |
| `core_engine/db.rs` | runs, ratings, judge, spend, stats, prompts, chat, settings | `export_csv`, legacy import, `spend_by_day`, `latency_points`, schema upgrade |
| `core_engine/chat.rs`, `evals.rs`, `providers.rs`, `token_auditor.rs` | good unit coverage | judge label mapping, invalid pricing file |
| `bin/promptman-cli.rs` | argument parsing | exit codes, empty CSV |
| `core_engine/engine.rs` | **none** | budget, `run_one`, `run_all`, optimize |
| `core_engine/secrets.rs` | **none** | vault decrypt and migration (this deletes a user file, so it needs tests) |
| `core_engine/hardware.rs`, `local_bridge.rs` | **none** | sampler lifecycle (would have caught D1), Ollama response parsing |
| `app.rs`, `gui/*` | **none** | Stop/Done races, judge mapping, analytics `buckets()` (pure; easy to test) |

---

## 6. Dependencies and security

- **Advisories:** RustSec reports no vulnerable crates (`cargo deny check advisories`).
- **Licenses:**
  - About 470 MIT and 370 Apache-2.0 crates, plus BSD, ISC, Zlib, Unicode-3.0, BSL-1.0 and Unlicense.
  - `self_cell` (Apache-2.0 OR GPL-2.0) and `r-efi` (MIT/Apache OR LGPL) are dual-licensed, so the permissive option applies.
  - The bundled egui fonts are under OFL-1.1 and the Ubuntu Font Licence, which require attribution in redistributions.
  - **The project itself is currently unlicensed.**
- **Duplicates:** the `windows-sys` family (4 versions), `base64` (2), `hashbrown` (2), `syn` (2), `miniz_oxide` (2). These come in through dependencies and aren't actionable from here.
- **Secrets handling:**
  - Keys live in the OS keychain.
  - Environment variables are the fallback, and a saved key takes precedence over the environment variable (`secrets.rs::get_key`).
  - Keys are never written to a file or log.

---

## 7. Recommended priorities

Progress: **Stages 1 and 2 are done** (see `CHANGELOG.md`). Stages 3–6 have not started.


**Stage 1. Repository safety and crash/leak fixes.** Low risk, no data-format changes.
- `.gitignore` for the legacy output (G2), `cargo fmt` plus `rustfmt.toml` (G5), MSRV (G6).
- Code fixes: D1, D4, R5 panics, R6, R10, D15 (`serde(default)`), D17.

**Stage 2. Streaming robustness.**
- D2 (idle timeout, partial text kept), D3 (Compare Stop), D5, D6 (estimated cost for partial/failed), R14, R11 (bounded retry).

**Stage 3. Data safety.** Needs owner approval for the DB format change.
- R1 (versioned migrations), R2 (WAL, `busy_timeout`, transactions), R3 (surface write errors), R4.

**Stage 4. Budget correctness.**
- D7 (custom endpoint billable flag, fail closed), D8.

**Stage 5. UI/UX consistency and accessibility.**
- U2–U8, R16–R18 (per-frame work), D9–D14, D16, D18.

**Stage 6. Release readiness.** Needs owner decisions.
- License, CI, SECURITY / CONTRIBUTING / CHANGELOG, legacy-file removal, packaging and signing notes.

Owner decisions needed:
1. Which license?
2. May the legacy Tauri/React files (`src_tauri/`, `src_frontend/`, `node_modules/`, `dist/`, `package*.json`, `tsconfig.json`, `vite.config.ts`) be deleted (they're not in git, so this can't be undone), or should they move to an `archive/` folder?
3. Approve adding schema versioning to `promptman.db` (Stage 3).
4. May `cargo clean` reclaim about 5 GB of build output? It's fully reproducible, but the next build takes about 5 minutes.
