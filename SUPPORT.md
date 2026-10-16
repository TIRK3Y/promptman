# Support

## Before asking

1. Read the [README](README.md), especially sections 04 (installation), 05 (quick start) and 14 (known limits).
2. Search [existing issues](https://github.com/TIRK3Y/promptman/issues?q=is%3Aissue), open and closed.

## Common problems

| Symptom | Fix |
|---|---|
| "No API key for ..." | Settings > Providers: paste the key and press Save, or set the provider's `*_API_KEY` variable |
| "Can't reach http://localhost:11434" | Start Ollama with `ollama serve`, or fix the endpoint in Settings |
| LM Studio / vLLM not answering | Start its server and set the OpenAI-compatible endpoint (default `http://localhost:1234/v1`) |
| "price unknown" | Add the model to `pricing.json` (Settings > Pricing and data > Open file), then Reload |
| Cloud requests blocked | The monthly hard stop is on and the cap is reached. Raise the cap or turn the hard stop off in Settings |
| Keys not saved on Linux | Install and unlock a Secret Service provider (GNOME Keyring or KWallet) |
| Build fails on Linux | Install the packages in README section 4.2 |
| Build fails on Windows with a linker error | Install the Visual Studio C++ build tools |

## Getting help

| Need | Where |
|---|---|
| Bug | [Bug report](https://github.com/TIRK3Y/promptman/issues/new?template=bug_report.md) |
| Feature | [Feature request](https://github.com/TIRK3Y/promptman/issues/new?template=feature_request.md) |
| Question | [Open an issue](https://github.com/TIRK3Y/promptman/issues/new) with the `question` label |
| Security | [SECURITY.md](SECURITY.md), privately |

When reporting, include the PromptMan version, your OS, the exact steps and any error text. Never paste API keys. To capture logs, run with `RUST_LOG=promptman=debug`.

PromptMan is maintained by volunteers. There is no paid support or guaranteed response time.
