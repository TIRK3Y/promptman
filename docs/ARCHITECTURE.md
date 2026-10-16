# Architecture

How PromptMan is put together, for contributors.

## Layers

```
+--------------------------------------------------------------+
|  src/main.rs           eframe window, tokio runtime           |
|  src/app.rs            tabs, top bar, event routing           |
|  src/gui/*             one module per screen (UI thread only) |
+------------------------------+-------------------------------+
                               | spawn tasks / receive Events
+------------------------------v-------------------------------+
|  src/core_engine/engine.rs   Engine: run models, budget,     |
|                              record results                  |
|  llm.rs        streaming HTTP for 4 protocols, retries,      |
|                timeouts, cancellation                        |
|  db.rs         SQLite storage    secrets.rs   OS keychain    |
|  token_auditor.rs  tokens+prices  evals.rs   checks, judge   |
|  local_bridge.rs   Ollama admin   hardware.rs CPU/RAM/GPU    |
+------------------------------+-------------------------------+
                               |
              src/bin/promptman-cli.rs uses the same Engine
```

`core_engine` contains no GUI code. The desktop app and the CLI are two front ends over it.

## Threads and events

- The UI runs on the main thread and must never block.
- `main.rs` starts a multi-threaded tokio runtime. Screens spawn work onto it (`tokio::spawn`).
- Background tasks report back through an `mpsc` channel of `app::Event` values (`Delta`, `SlotDone`, `CompareDone`, `ChatDelta`, `ChatDone`, `BatchCell`, `PullProgress`, and so on). Each send also requests a repaint.
- Every frame, `App::handle_events` drains the channel and routes each event to the screen that owns it.
- Every request carries a `token` (a counter). A screen ignores events whose token is not its current one, so results from an abandoned or replaced request never land on screen.

## A Compare run, end to end

```
Run comparison
  -> CompareState::start_run          snapshot prompt, system, params; new token;
                                      one CancellationToken per model
  -> tokio::spawn(Engine::run_all)    all models in parallel
       -> Engine::run_one             budget check, key lookup, endpoint
            -> llm::stream_chat       HTTP request, retries before first byte,
                                      idle timeout, SSE / NDJSON parsing
                 on_delta  -> Event::Delta      text appears live
            <- Completion or StreamError (partial text kept)
       -> token counts (reported or estimated), cost from pricing.json,
          hardware peaks for local models
       on_done -> Event::SlotDone             card finishes independently
  -> Db::save_run                     run + results in one transaction
  -> Event::CompareDone               run id, save error if any
```

Stop cancels the `CancellationToken`. `stream_chat` returns the partial text with `cancelled = true`; the engine estimates tokens and cost and records it as stopped.

## Protocols

`llm.rs` speaks four wire formats, chosen per provider in `providers.rs`:

| Protocol | Providers |
|---|---|
| OpenAI chat completions | OpenAI, Groq, Mistral, OpenAI-compatible servers |
| Anthropic messages | Anthropic |
| Gemini `streamGenerateContent` | Google Gemini |
| Ollama `/api/chat` | Ollama |

Adding an OpenAI-compatible provider is one entry in `PROVIDERS`. A new protocol needs a request builder and a `parse_line` branch in `llm.rs`, each with tests.

## Storage

One SQLite file, `promptman.db`, opened by `Db`:

| Table | Holds |
|---|---|
| `runs` | One row per run: kind (compare, chat, batch, judge, rewrite, cli), prompt, system prompt, parameters, time |
| `results` | One row per model answer: output, error, tokens, cost, latency, time to first token, ratings, judge score, hardware peaks |
| `conversations`, `messages` | Chat history |
| `prompts` | Library, one row per version |
| `settings` | Key / JSON value pairs (Composer, chat options, budget, endpoints, last tab) |

API keys are never stored here; `secrets.rs` keeps them in the OS keychain under the service `PromptMan`.

## Failure handling principles

- A model that fails shows its own error; the others keep going.
- Partial text is never thrown away.
- Retries happen only before the first byte, so output is never duplicated.
- Malformed files and streams produce errors or defaults, never panics.
- Unknown prices are shown as unknown, not zero.
