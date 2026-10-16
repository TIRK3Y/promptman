```
 ____  ____   ___  __  __ ____ _____ __  __    _    _   _
|  _ \|  _ \ / _ \|  \/  |  _ \_   _|  \/  |  / \  | \ | |
| |_) | |_) | | | | |\/| | |_) || | | |\/| | / _ \ |  \| |
|  __/|  _ <| |_| | |  | |  __/ | | | |  | |/ ___ \| |\  |
|_|   |_| \_\\___/|_|  |_|_|    |_| |_|  |_/_/   \_\_| \_|
```

**One prompt. Many models. Real numbers.**

PromptMan is a native desktop workbench for people who write prompts for a living. Chat with any model, send the same prompt to up to four models side by side, run it across a CSV of test cases, and see exactly what every answer cost in time, tokens and money. Cloud and local models, one window, nothing leaves your machine except the requests you send.

Written in Rust. Renders with [egui](https://github.com/emilk/egui). No web view, no Electron, no Node.js, no account, no telemetry.

```
LICENSE   MIT
VERSION   1.0.0
PLATFORM  Windows / macOS / Linux
RUST      1.95 or newer
BINARIES  promptman (desktop app)   promptman-cli (scripts and CI)
```

---

## Contents

```
01  Why PromptMan            09  Data, storage and privacy
02  Use cases                10  Pricing and token counting
03  Features at a glance     11  Configuration reference
04  Installation             12  Project structure
05  Quick start              13  Development
06  The six screens          14  Known limits
07  Command line             15  Contributing, support, security
08  Providers and models     16  License
```

---

## 01 Why PromptMan

Choosing a model, tuning a prompt and keeping spend under control are usually three tools and a spreadsheet. PromptMan puts them in one place and keeps the evidence:

- **Side by side, live.** Answers stream in parallel. Time to first token, total latency, tokens per second, token counts and cost sit on every card.
- **Honest numbers.** Costs come from a pricing file you control. Estimated token counts are marked. Unknown prices show as unknown, never as `$0`.
- **Local first.** Ollama, LM Studio, vLLM and llama.cpp sit next to OpenAI, Anthropic, Gemini, Groq and Mistral. Local runs record peak CPU, RAM and NVIDIA GPU use.
- **Repeatable.** Every run is saved and can be reopened, rated, judged, diffed and exported. The same engine runs from the command line with exit codes for CI.
- **Your keys stay yours.** API keys live in the operating system's credential store, never in a file.

---

## 02 Use cases

| Who | Task | How PromptMan helps |
|---|---|---|
| Prompt engineers | Find the wording that works | Compare up to four models at once, Optimize the draft, diff two answers word by word, save every version in the Library |
| Engineering teams | Pick a model for a feature | Run the same prompt against cloud and local candidates, read latency and cost per answer, let a judge model score them |
| QA and evaluation | Catch regressions | Batch a CSV of cases with contains, regex, JSON and expected-value checks, read pass rates per model, export the grid |
| CI pipelines | Gate a prompt change | `promptman-cli` exits 1 when an answer fails a check, emits JSONL for further processing |
| Cost owners | Keep spend predictable | Monthly budget with a warning level and an optional hard stop for cloud models; spend by day, week and model in Analytics |
| Local model users | Decide whether local is good enough | Run Ollama or an OpenAI-compatible server next to a cloud model on the same prompt and compare quality, speed and hardware load |
| Writers and analysts | Everyday chat with any model | Persistent, searchable conversations, editable history, concise-reply and memory controls that cut token use |

---

## 03 Features at a glance

```
CHAT        one model at a time, switch between replies, edit / delete / regenerate,
            Stop keeps partial text, concise replies, memory window, Markdown export
COMPARE     up to 4 models in parallel, live streaming, per-model Stop, ratings,
            judge scores, word-level diff, conversation mode, {{variables}},
            Markdown / JSON export, full run history
OPTIMIZE    a model rewrites your prompt clearer and shorter; you see both, edit
            either, and choose; dropped {{variables}} are flagged
BATCH       one run per CSV row, checks (contains, not contains, regex, valid JSON,
            expected column), pass rates per model, CSV export
LIBRARY     named, tagged, versioned prompts; load any version into the Composer
ANALYTICS   spend over time, spend by model, latency trends, per-model table with
            average rating and win rate, full history CSV export
SETTINGS    keys in the OS keychain, custom endpoints, Ollama pull / preload /
            delete, monthly budget and hard stop, pricing file
CLI         the same engine for scripts and CI, table or JSONL output, exit codes
```

---

## 04 Installation

PromptMan is distributed as source. Building takes a few minutes the first time.

### 4.1 Requirements

| Component | Version | Notes |
|---|---|---|
| Rust toolchain | 1.95 or newer | Installed with `rustup` |
| Git | any recent | To clone the repository |
| Windows | 10 or 11 | MSVC build tools (C++ workload) |
| macOS | 10.15 or newer | Xcode Command Line Tools |
| Linux | X11 or Wayland | Packages listed in 4.2, plus a Secret Service provider (GNOME Keyring or KWallet) for API keys |
| Ollama | optional | Only for local models |

### 4.2 Install the toolchain

**Windows** (PowerShell)

```powershell
# 1. C++ build tools: install "Desktop development with C++" from
#    https://visualstudio.microsoft.com/visual-cpp-build-tools/
# 2. Rust
winget install Rustlang.Rustup
rustup default stable
# 3. Git
winget install Git.Git
```

**macOS** (Terminal)

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**Linux** (Debian / Ubuntu)

```sh
sudo apt update
sudo apt install -y build-essential pkg-config git curl libssl-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev \
  libxi-dev libgl1-mesa-dev gnome-keyring
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Fedora: `sudo dnf install gcc pkg-config openssl-devel libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel mesa-libGL-devel gnome-keyring`

Confirm the toolchain:

```sh
rustc --version     # 1.95.0 or newer
cargo --version
```

### 4.3 Get the source and build

```sh
git clone https://github.com/TIRK3Y/promptman.git
cd promptman
cargo build --release
```

The binaries land in `target/release/`:

```
target/release/promptman        (promptman.exe on Windows)   desktop app
target/release/promptman-cli    (promptman-cli.exe)           command line
```

### 4.4 Run

```sh
cargo run --release                          # desktop app
cargo run --release --bin promptman-cli -- --help
```

Or run the built binaries directly:

```sh
./target/release/promptman
./target/release/promptman-cli models
```

### 4.5 Install on your PATH (optional)

```sh
cargo install --path .
promptman
promptman-cli --help
```

`cargo install` places both binaries in `~/.cargo/bin` (Windows: `%USERPROFILE%\.cargo\bin`), which `rustup` adds to `PATH`.

### 4.6 Build an installer (optional)

```sh
cargo install cargo-packager
cargo packager --release --formats nsis        # Windows: .exe setup
cargo packager --release --formats dmg         # macOS
cargo packager --release --formats deb,appimage  # Linux
```

Installers are unsigned. Signing needs your own Windows code-signing certificate or Apple Developer ID.

### 4.7 Update

```sh
cd promptman
git pull
cargo build --release
```

Your data lives outside the source folder (see section 09), so updating or deleting the checkout never touches it.

### 4.8 Uninstall

```sh
cargo uninstall promptman        # only if you used cargo install
```

Then delete the data folder from section 09 and the keys saved under the service "PromptMan" in your OS credential store.

---

## 05 Quick start

**1. Connect a model.** Open **Settings**, paste an API key next to each cloud provider you use and press **Save**. The key goes to the OS keychain.

**2. Or go local.** Install [Ollama](https://ollama.com), then:

```sh
ollama serve
ollama pull llama3.2:3b          # or pull from Settings > Ollama models
```

For LM Studio, vLLM or llama.cpp, start its server and set the **OpenAI-compatible** endpoint in Settings (LM Studio's default `http://localhost:1234/v1` is preset).

**3. Chat.** Choose the chat model in **Settings > Chat**, open **Chat**, type, press Enter.

**4. Compare.** Open **Compare**, pick up to four models in the Composer, write a prompt, press **Run comparison** or Ctrl+Enter.

**5. Test at scale.** Open **Batch**, choose a CSV, add checks, press **Run**.

If a model can't answer (no key, server not running, budget reached) its card says why. PromptMan never shows placeholder answers.

---

## 06 The six screens

### Top bar

| Element | Behaviour |
|---|---|
| Tabs | Chat, Compare, Batch, Library, Analytics, Settings. The app reopens on the tab you used last. |
| Ollama chip | `Ollama - N models` when the local server answers, `Ollama offline` otherwise. Checked at start and every 15 seconds. |
| Budget chip | `$spent of $cap` for the month. Colour changes at the warning level and the cap. Click opens Settings. |
| Theme toggle | Light or dark. Follows the OS until you choose. |
| Notices | Confirmations and errors appear under the bar, each with Dismiss. |

### Chat

One conversation, one model at a time.

| Area | What you can do |
|---|---|
| Sidebar | New chat, search chats by title, open any saved conversation (most recent first, with model and time). Drag to resize. |
| Header | Rename, Export (Markdown), Delete (two clicks). Shows model, message count, total tokens and cost. |
| Your messages | Pencil icon reveals Edit, Copy, Delete. **Save & resend** replaces the message, removes everything after it and asks for a new reply. Disabled while a reply streams. |
| Replies | Rendered Markdown. Header shows provider, model, seconds, tokens in and out, cost (or Free, or price unknown). Copy, Delete, and Regenerate on the last reply. |
| Message box | Enter sends, Shift+Enter adds a line. A live line shows the message tokens, the total sent with context, and its input cost. |
| Stop | Ends the reply and keeps the text received so far. Its tokens and cost are estimated and count toward the budget. |
| Optimize | Opens a card with your message and a suggested rewrite side by side. Edit either, then Send mine or Send suggested. |
| Empty chat | Shows the current model and, if it can't answer yet, a link to fix it. |

Chat options live in **Settings > Chat**: model, system prompt, concise replies, memory window, temperature, max reply tokens, optimizer model.

### Compare

Three resizable panels: Composer, Results, Usage.

**Composer**

| Control | Behaviour |
|---|---|
| Models | One row per model (provider, model, remove). Up to four. Any model ID can be typed. |
| Parameters | Temperature 0 to 2, top-p 0 to 1, max tokens 1 to 128,000. Not sent where a model rejects them. |
| Conversation mode | Each run continues the thread with every model, each keeping its own replies. |
| System prompt, Prompt | Monospace editors with character, word and token counts. |
| `{{variables}}` | One input appears per variable and is filled before sending. |
| Optimize prompt | Suggests a clearer, shorter prompt. Keep mine or Use suggested. Dropped variables are flagged. |
| Run comparison | Ctrl+Enter from anywhere on the tab. A line explains why it is disabled. Becomes **Stop all** during a run. |

The Composer is saved as you edit it.

**Results**

| Control | Behaviour |
|---|---|
| Answer cards | Provider, model, Copy, Stop (this model only). Latency, time to first token, tokens, tokens per second, cost; peak CPU, RAM, GPU for local models; notes; 1 to 5 star rating; judge score with its reason. |
| Failed answers | Show the error, with any text received before the failure kept underneath. |
| Differences | Pick two answers and read a word-level diff. Needs at least two answers. |
| Render Markdown | Toggle between formatted and raw text. |
| Judge | A judge model scores every answer with text from 0 to 10, with a one-sentence reason. |
| Export | Markdown or JSON of the run on screen, including its system prompt and parameters. |

**Usage**

Prompt tokens and input cost across the selected models, this month's budget bar, and the history of the latest 200 answers grouped by run. Click a run to reopen it.

### Batch

Runs the Composer prompt once per CSV row; columns fill `{{variables}}`.

| Step | Behaviour |
|---|---|
| Inputs | Choose CSV. Shows rows and columns, warns about variables with no matching column. A `prompt` column is used when the Composer prompt is empty. |
| Checks | Contains, Does not contain (case-insensitive), Matches regex, Is valid JSON (code fences accepted). A column named `expected` adds a contains-check for its row. |
| Run | Rows run in order, models in parallel. **Stop** halts at once and marks unfinished answers stopped. |
| Summary | Per model: passed, pass rate, errors, average latency, spend. |
| Grid and export | Pass or fail per cell with hover detail. Export CSV with inputs, model, result, failures, latency, cost, output. |

### Library

Save the Composer's system prompt and prompt under a name with tags. Saving under an existing name adds a version; older versions stay. Search by name or tag, switch versions, **Load into Composer**, delete (two clicks).

### Analytics

Range of 7, 30 or 90 days or 1 year. Spend, answer count, average latency and most-used model; spend per day or week; spend by model; latency per answer over time; a per-model table with errors, latency, time to first token, tokens per second, spend, average rating and win rate. **Export all history (CSV)** writes every answer ever recorded.

Win rate: among runs where two or more answers were rated, how often a model had the single highest rating.

### Settings

| Section | Contents |
|---|---|
| Providers | Key status (keychain, environment variable, none, not needed), paste or remove a key, edit or reset the endpoint URL. |
| Chat | Model, system prompt, concise replies, memory (last 1, 3, 6, 12 exchanges or all), temperature, max reply tokens, optimizer model. |
| Ollama models | Status and version, installed models with size, parameters and quantization, Preload, Delete, Pull with progress. |
| Monthly budget | Cap (default $10), warning level (default 80 percent), hard stop for cloud models (off by default). |
| Pricing and data | Open and reload `pricing.json`; open the data folder. |

### Keyboard

| Keys | Where | Action |
|---|---|---|
| Enter | Chat message box | Send |
| Shift+Enter | Chat message box | New line |
| Ctrl+Enter (Cmd+Enter) | Compare | Run comparison |
| Enter / Esc | Renaming a chat | Save / cancel |

---

## 07 Command line

`promptman-cli` uses the same engine, keys, prices and history as the app.

```
promptman-cli --models <provider/model,...> (--prompt <text> | --prompt-file <path>) [options]
promptman-cli models
```

| Option | Meaning |
|---|---|
| `--models` | Comma-separated `provider/model` list. Required. |
| `--prompt`, `--prompt-file` | Prompt or template text. |
| `--csv <path>` | Run once per row. Columns fill `{{column}}`; `prompt` and `expected` columns work as in Batch. |
| `--system <text>` | System prompt. |
| `--temperature <f>` | Default 0.7. |
| `--top-p <f>` | Default 1.0. |
| `--max-tokens <n>` | Default 4096. |
| `--contains <text>` | Fail unless the output contains the text. Repeatable. |
| `--not-contains <text>` | Fail if the output contains the text. Repeatable. |
| `--regex <pattern>` | Fail unless the output matches. Repeatable. |
| `--json` | Fail unless the output is valid JSON. |
| `--format table\|jsonl` | Human-readable table (default) or one JSON object per answer. |
| `--no-save` | Do not record runs in history. |

| Exit code | Meaning |
|---|---|
| 0 | Every answer succeeded and passed its checks |
| 1 | At least one answer errored or failed a check |
| 2 | Usage error (bad flags, unreadable or empty CSV) |

Examples:

```sh
promptman-cli models
promptman-cli --models openai/gpt-4o,ollama/llama3 --prompt "Capital of France?" --contains paris
promptman-cli --models mistral/mistral-small-latest --csv cases.csv \
  --prompt "Translate {{text}} to French" --format jsonl > results.jsonl
```

CI example (GitHub Actions step):

```yaml
- name: Prompt regression
  env:
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
  run: promptman-cli --models openai/gpt-4o-mini --csv tests/cases.csv --no-save --format jsonl
```

---

## 08 Providers and models

| Provider | ID | Needs | Default endpoint |
|---|---|---|---|
| OpenAI | `openai` | API key | `https://api.openai.com/v1` |
| Anthropic | `anthropic` | API key | `https://api.anthropic.com/v1` |
| Google Gemini | `gemini` | API key | `https://generativelanguage.googleapis.com/v1beta` |
| Groq | `groq` | API key | `https://api.groq.com/openai/v1` |
| Mistral | `mistral` | API key | `https://api.mistral.ai/v1` |
| OpenAI-compatible (LM Studio, vLLM, llama.cpp) | `custom` | Running server, key optional | `http://localhost:1234/v1` |
| Ollama | `ollama` | Running Ollama | `http://localhost:11434` |

- Model pickers suggest the models in `pricing.json` (cloud) or your installed models (Ollama). Any other model ID can be typed.
- Every endpoint can be changed in Settings, for proxies, gateways or a server on another machine.
- Rate limits and temporary server errors (429, 500, 502, 503, 504, 529) are retried up to twice before a reply starts, honouring `Retry-After`.
- A request gives up after 2 minutes with no data. Long replies are never cut off while data keeps arriving.
- For Anthropic's newest models, server-side fallback is enabled: if the model declines a request, another Anthropic model answers and the reply says so.

---

## 09 Data, storage and privacy

| What | Where |
|---|---|
| API keys | OS credential store: Windows Credential Manager, macOS Keychain, Secret Service on Linux. Never written to a file. |
| Chats, runs, answers, ratings, judge scores, library, settings | One SQLite file, `promptman.db`, in the data folder |
| Prices | `pricing.json` in the data folder |

Default data folder:

```
Windows  %APPDATA%\promptman\PromptMan\data
macOS    ~/Library/Application Support/com.promptman.PromptMan
Linux    ~/.local/share/promptman
```

**Settings > Pricing and data > Open folder** shows the exact path.

- Requests go only to the providers you select. Local models never leave your machine.
- No telemetry, no update checks, no account.
- Set `PROMPTMAN_DATA_DIR` to use another folder: a portable install, a separate profile, or a throwaway test profile.
- Upgrading from 0.1: keys in the old `vault.enc` move to the OS keychain and the file is deleted; `audit_ledger.json` is imported and renamed `audit_ledger.imported.json`.

---

## 10 Pricing and token counting

`pricing.json` lists USD per million input and output tokens per model. It ships with list prices for common models. Providers change prices, so check them against your provider's billing, edit the file and press **Reload** in Settings.

- A cloud model missing from the file shows "price unknown" and is not counted toward the budget.
- Local models are free.
- Token counts reported by the provider are used when available. Otherwise they are estimated locally with OpenAI's tokenizer and marked `~`, which is approximate for other vendors' models.
- Stopped and interrupted replies always use estimates, since providers do not report usage for unfinished replies.

---

## 11 Configuration reference

| Variable | Purpose |
|---|---|
| `PROMPTMAN_DATA_DIR` | Use this folder for the database, prices and settings |
| `OPENAI_API_KEY` | Key for `openai` when none is saved in the keychain |
| `ANTHROPIC_API_KEY` | Key for `anthropic` |
| `GEMINI_API_KEY` | Key for `gemini` |
| `GROQ_API_KEY` | Key for `groq` |
| `MISTRAL_API_KEY` | Key for `mistral` |
| `CUSTOM_API_KEY` | Optional key for the OpenAI-compatible endpoint |
| `RUST_LOG` | Log filter, for example `RUST_LOG=promptman=debug` |

A key saved in the keychain takes precedence over an environment variable. Empty variables are ignored.

Defaults:

| Setting | Default |
|---|---|
| Chat model | Anthropic, largest current model |
| Compare models | `openai/gpt-4o`, `ollama/llama3` |
| Judge and optimizer | Anthropic, largest current model |
| Temperature, top-p, max tokens | 0.7, 1.0, 4096 |
| Concise replies, memory | On, last 6 exchanges |
| Budget | $10 a month, warn at 80 percent, hard stop off |
| Theme | Follows the OS |

---

## 12 Project structure

```
promptman/
|-- src/
|   |-- main.rs                  desktop entry point
|   |-- lib.rs
|   |-- app.rs                   app shell, tabs, background events
|   |-- bin/promptman-cli.rs     command-line runner
|   |-- core_engine/             no GUI code; shared by app and CLI
|   |   |-- engine.rs            runs models, records results, budget
|   |   |-- llm.rs               streaming for OpenAI, Anthropic, Gemini, Ollama APIs
|   |   |-- providers.rs         provider table, provider/model references
|   |   |-- db.rs                SQLite: runs, chats, library, settings, analytics
|   |   |-- secrets.rs           OS keychain access, legacy vault migration
|   |   |-- token_auditor.rs     token counting, pricing.json
|   |   |-- evals.rs             templates, CSV, checks, judge and rewrite helpers
|   |   |-- chat.rs              chat context window and prompts
|   |   |-- hardware.rs          CPU / RAM / GPU sampling for local runs
|   |   `-- local_bridge.rs      Ollama status, pull, delete, preload
|   `-- gui/
|       |-- chat.rs compare.rs batch.rs library.rs analytics.rs settings.rs
|       |-- composer.rs          prompt editor, model pickers, Optimize card
|       |-- results.rs           answer cards, diff view, exports
|       |-- theme.rs             palette and shared widgets
|       `-- test_support.rs      headless rendering for tests
|-- tests/cli.rs                 end-to-end CLI tests against a mock server
|-- assets/                      icons, macOS and Windows bundle metadata
|-- docs/                        architecture, test plan, audit and history
`-- .github/                     CI, issue and pull request templates
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how a request flows from the UI to a provider and back.

---

## 13 Development

```sh
cargo run                                                 # debug build of the app
cargo test                                                # all tests
cargo fmt --check                                         # formatting
cargo clippy --all-targets --all-features -- -D warnings  # lints, warnings are errors
```

- 127 automated tests: stream parsing for every protocol, retries and timeouts, storage, pricing, checks, every screen rendered headless at four window sizes in both themes, and the CLI end to end.
- No test calls a real provider. HTTP goes to a local mock server or to a closed port.
- Debug builds disable overflow checks for `winit` only, working around an IME overflow panic in winit 0.30.13 (see `Cargo.toml`).

Details: [docs/TEST_PLAN.md](docs/TEST_PLAN.md).

---

## 14 Known limits

- Replies that drop halfway keep their partial text but are not resumed.
- GPU use is measured on NVIDIA cards only (via `nvidia-smi`).
- Token counts for non-OpenAI models are estimates when the provider does not report them.
- Prices in `pricing.json` are starting values.
- No auto-update. Installers are unsigned.
- Primary development and testing happen on Windows; macOS and Linux are built by CI.

---

## 15 Contributing, support, security

| Need | Go to |
|---|---|
| Contribute code or docs | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Ask a question or get help | [SUPPORT.md](SUPPORT.md) |
| Report a bug | [New issue: bug report](https://github.com/TIRK3Y/promptman/issues/new?template=bug_report.md) |
| Request a feature | [New issue: feature request](https://github.com/TIRK3Y/promptman/issues/new?template=feature_request.md) |
| Report a vulnerability | [SECURITY.md](SECURITY.md). Do not open a public issue. |
| Community standards | [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) |
| Release history | [CHANGELOG.md](CHANGELOG.md) |

---

## 16 License

MIT. See [LICENSE](LICENSE).

```
Copyright (c) 2026 TIRK3Y
```
