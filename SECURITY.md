# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 1.0.x | Yes |
| Older than 1.0 | No. Upgrade; on first launch 1.0 moves keys out of the old `vault.enc` file |

## Reporting a vulnerability

Do not open a public issue.

Report privately through GitHub: **[Security > Report a vulnerability](https://github.com/TIRK3Y/promptman/security/advisories/new)**. If that is unavailable, email **ajaiown10@gmail.com** with the subject `PromptMan security`.

Include:

- What the problem is and its impact.
- Steps to reproduce, and the version and OS.
- A proof of concept if you have one. Do not include real API keys.

| Step | Target |
|---|---|
| Acknowledgement | Within 3 days |
| Initial assessment | Within 7 days |
| Fix or mitigation for confirmed issues | As soon as practical, with credit to the reporter unless you prefer otherwise |

## Security model

- **API keys** are stored only in the OS credential store (Windows Credential Manager, macOS Keychain, Secret Service). They are never written to files or logs.
- **Network** traffic goes only to the provider endpoints configured in Settings, over HTTPS for cloud providers by default. There is no telemetry and no update check.
- **Local data** (`promptman.db`, `pricing.json`) is stored unencrypted in the user's data folder and is protected by the operating system's file permissions. Anyone with access to your user account can read your history.
- **Exports** (CSV, Markdown, JSON) contain prompts and answers in plain text. Treat them as sensitive.
- **Custom endpoints** receive your prompts and, if configured, a key. Only point PromptMan at servers you trust.

## In scope

- Key leakage (to disk, logs, exports, or the wrong endpoint).
- Remote code execution or file writes triggered by provider responses, CSV files or pricing files.
- Crashes caused by malformed provider streams or files that could be used against users.

## Out of scope

- Attacks that need full control of the user's account or machine.
- Unsigned installers (documented limitation).
- Prompt injection inside model answers. PromptMan displays answers; it does not act on them.
