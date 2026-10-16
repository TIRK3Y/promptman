# Icons

| File | Used by |
|---|---|
| `app-icon.png` | Window icon, embedded at build time (`src/main.rs`) |
| `icon.ico` | Windows executables (`build.rs`) and the Windows installer |
| `icon.icns` | macOS bundle |

`icon.ico` holds 16 to 256 px sizes; `icon.icns` goes up to 1024 px. Regenerate both from `app-icon.png` with any icon tool when the artwork changes.
