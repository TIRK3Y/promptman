# Performance baseline

Measured 2026-10-15 on the developer's Windows 11 machine (1920×1009 client area, display scale about 1.1). These are single-machine numbers. Use them to spot regressions, not as targets.

| Metric | How | Result |
|---|---|---|
| Cold release build | `cargo build --release` into an empty target dir | 278 s (4 min 38 s) |
| GUI binary (`promptman.exe`, release) | file size | 22.2 MB |
| CLI binary (`promptman-cli.exe`, release) | file size | 12.0 MB |
| Windows installer (NSIS, unsigned) | `cargo packager --release --formats nsis` | 12 MB |
| GUI time to first window | release build, stopwatch from process start until the window exists; 3 runs | 4,837 ms (first launch of a new binary), 95 ms, 77 ms |
| GUI memory 5 s after start | working set / private bytes | 138–160 MB / 92–99 MB |
| CLI `models` | release build, 3 warm runs | 38, 25, 26 ms (first launch 3,173 ms) |
| Unit tests | `cargo test` | 23 tests, about 1.5 s |

**First-launch delay:** the slow first launch of each new binary (3–5 s) wasn't seen again on later runs. It's probably antivirus scanning a newly written executable, but that hasn't been verified.

## Disk

| Item | Size |
|---|---|
| `target/debug` | 4.2 GB (`deps` 3.4 GB, `incremental` 651 MB) |
| `target/release` | 872 MB |
| `node_modules` (legacy) | 70 MB |
| `.git` (loose objects, no commits) | 49 MB |
| Source and assets | about 0.4 MB |
| User data (`%APPDATA%\promptman`) | 116 KB |

## Not yet measured

Memory and frame time with long chats (1,000+ messages), large batches (1,000+ rows), and long streaming replies; GPU and CPU cost of idle repaint; macOS and Linux.
