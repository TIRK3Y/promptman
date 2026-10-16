# Contributing to PromptMan

Thanks for considering a contribution. This document is the contract for how changes get in.

```
01  Ground rules          05  Code standards
02  Ways to contribute    06  Tests
03  Development setup     07  Commits and pull requests
04  Workflow              08  Review and merge
```

---

## 01 Ground rules

- Follow the [Code of Conduct](CODE_OF_CONDUCT.md).
- Security problems go through [SECURITY.md](SECURITY.md), never a public issue.
- Discuss large changes in an issue before writing them. A pull request that redesigns a screen, adds a dependency or changes the database format without prior agreement may be closed.
- One concern per pull request. A bug fix does not carry a refactor.
- Never commit API keys, personal data, databases (`promptman.db`) or build output.
- By contributing you agree your work is released under the [MIT License](LICENSE).

## 02 Ways to contribute

| Contribution | Start here |
|---|---|
| Bug report | Search existing issues, then use the bug report template with exact steps, OS and version |
| Feature idea | Feature request template. Describe the problem before the solution |
| Code | Issues labelled `good first issue` or `help wanted`, or one you opened and discussed |
| Documentation | README, `docs/`, in-app text. Typos and clarifications are welcome without an issue |
| Provider support | New providers belong in `src/core_engine/providers.rs` and `llm.rs`, with request-shape and stream-parsing tests |
| Pricing updates | Edit the defaults in `src/core_engine/token_auditor.rs` and link the provider's price page in the pull request |

## 03 Development setup

Install the toolchain for your OS as described in [README section 04](README.md#04-installation), then:

```sh
git clone https://github.com/<your-username>/promptman.git
cd promptman
git remote add upstream https://github.com/TIRK3Y/promptman.git
cargo build
cargo test
```

Use a separate profile so development never touches your real history:

```sh
# macOS / Linux
PROMPTMAN_DATA_DIR=./.dev-profile cargo run
# Windows PowerShell
$env:PROMPTMAN_DATA_DIR = ".\.dev-profile"; cargo run
```

`.dev-profile/` is ignored by git. API keys still go to the OS keychain; use test keys or a local model.

For a free provider during development, run Ollama (`ollama serve`, `ollama pull llama3.2:3b`) or any OpenAI-compatible server.

## 04 Workflow

```sh
git fetch upstream
git switch -c fix/short-description upstream/main
# make the change, add tests
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
git push origin fix/short-description
```

Then open a pull request against `main`.

Branch prefixes: `fix/`, `feat/`, `docs/`, `test/`, `refactor/`, `chore/`.

## 05 Code standards

- **Formatting:** `cargo fmt`. The configuration is in `rustfmt.toml` (max width 120).
- **Lints:** `cargo clippy --all-targets --all-features -- -D warnings` must pass. Do not silence a lint without a comment explaining why.
- **Layering:** `src/core_engine/` has no GUI code and is shared by the app and the CLI. GUI code lives in `src/gui/`. Keep it that way.
- **No panics on user input or provider data.** Malformed files, CSVs, streams and settings return errors or fall back to defaults.
- **No blocking in the UI thread.** Network, disk-heavy and model work run on the tokio runtime and report back through events.
- **Honest numbers.** Never show an estimate as exact or an unknown price as zero.
- **Secrets** go through `secrets.rs` and the OS keychain only. Never log or write them.
- **Dependencies:** prefer the standard library and existing crates. A new dependency needs a reason in the pull request.
- **Comments** explain why, not what. Match the density of the surrounding code.
- **User-facing text** is plain, specific and short. No emoji.

## 06 Tests

Every bug fix comes with a test that fails without the fix. Every feature comes with tests for its main path and its failure paths.

| Area | Where | How |
|---|---|---|
| Streaming, retries, timeouts | `core_engine/llm.rs` | `llm::test_server` scripts a mock HTTP server |
| Engine, storage, pricing, checks | `core_engine/*.rs` | In-memory database via `Engine::for_test` and `Db::open_in_memory` |
| Screens | `gui/*.rs` | `gui::test_support::render_all` draws headless at four sizes in both themes |
| CLI | `tests/cli.rs` | Runs the real binary against a local mock server |

Rules:

- Tests never call a real provider. Use the mock server or `test_support::NOWHERE`.
- Tests never touch the real data folder or keychain entries of real providers.
- Tests must pass when run in parallel. Use unique temp file names.

See [docs/TEST_PLAN.md](docs/TEST_PLAN.md) for coverage.

## 07 Commits and pull requests

Commit messages: imperative mood, 72-character subject, body explaining why.

```
Stop the judge from scoring empty answers

A stopped answer with no text got a score, which skewed win rates.
```

A pull request must:

- Reference its issue (`Fixes #123`) when there is one.
- Describe what changed and why, and how it was tested.
- Include screenshots for visible UI changes, in light and dark themes.
- Update `README.md`, `docs/` and `CHANGELOG.md` (under "Unreleased") when behaviour changes.
- Pass CI: format, clippy and tests on Windows, macOS and Linux.

## 08 Review and merge

- A maintainer reviews within about a week. Ping the pull request if it has been quiet longer.
- Address review comments with new commits; they are squashed on merge.
- Changes to the database schema, the pricing defaults, the license or the security model need maintainer approval before work starts.
- Maintainers may edit a pull request to finish it, with credit kept.
