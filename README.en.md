# DeepSeekChat

[中文](README.md) · **English**

> Mirrored translation of [README.md](README.md) — same structure, same facts. If you change one, change both. The product specification [PRD.md](PRD.md) is currently Chinese-only.

---

> **Unofficial disclaimer**: DeepSeekChat is a community-built **unofficial third-party client**. It has **no affiliation with DeepSeek** and is not supported by them. The DeepSeek name and whale mark belong to DeepSeek and are used here only to identify the service this client wraps — no endorsement is implied. Using this project means you agree to follow DeepSeek's official terms of service.

Wraps `chat.deepseek.com` in a tiny native shell: **tray-resident, login that never drops, extremely low memory**. 100% of the chat experience comes from the official page — this project is only the "shell".

## Download

| Form | File | Notes |
| --- | --- | --- |
| Portable single exe | `DeepSeekChat-v0.1.0-portable.exe` (~6.7MB) | Drop into any writable folder and run; delete to uninstall |
| NSIS installer | `DeepSeekChat-v0.1.0-setup.exe` (~3.3MB) | When you want Start-menu / uninstall entries |

- First-run SmartScreen prompt: click **More info** → **Run anyway** (no code signing; happens once).
- Data lives in `data\` next to the exe; falls back to `%APPDATA%\DeepSeekChat\` if that folder is not writable.
- If the WebView2 Runtime is missing, the app asks for consent and installs it automatically online.

## Features

**Window**: official page + system title bar; position/size/maximize state remembered; auto-recovers when a monitor is unplugged; Ctrl+wheel zoom; drag images straight into the chat.

**Refresh (F5 / Ctrl+R / Ctrl+F5)**: kernel-level reload — login and conversation context survive. There is also a refresh button at the bottom-right of the page.

**Fullscreen (F11)**: toggle anytime.

**Conversation export**: floating toolbar at the bottom-right of the page → 5 formats (Markdown / plain text / HTML / Word / JSON); file name `Conversation_YYYY-MM-DD_HH-MM-SS`; saved to `data\Export session\` and the folder opens automatically. Exporting triggers the site's print mode so every message is mounted (complete conversations even under virtual scrolling); attachments are formatted as `📎 filename TYPE size`; thinking blocks are kept strictly separate from the answer body.

**Tray**: left-click toggles the window; right-click opens a menu (Open / Settings / Exit) that follows the site's light/dark theme.

**Settings** (four items): launch at startup, close-button behavior (minimize to tray / quit), ultra-low-memory mode, data directory.

**Theme following**: measures the page background color and keeps the title bar, borders, settings page and tray menu in sync in real time (including transition colors while switching).

**Reliability**: local error page on network failure (reason + retry + 20s auto-retry); top-level navigation allow-list (DeepSeek domains stay in-app, everything else goes to the system browser — except OAuth popups); renderer-process crashes self-heal in 5 seconds; the site's "environment anomaly" risk dialog is auto-dismissed.

**Privacy**: zero telemetry, zero reporting, all data local; production logs are sanitized; DevTools disabled in release builds.

## Memory (measured)

| State | Budget | Measured |
| --- | --- | --- |
| Foreground idle | ≤180MB | **20MB** |
| Suspended in tray | ≤80MB | **4MB** |

How: event-driven working-set trimming + WebView2 TrySuspend (auto-degrades when refused) + destroy-and-rebuild after 30 minutes hidden. Re-verify with `.\scripts\qa-memory.ps1`.

## Building from source

Requires Windows 10 1809+ x64, Rust stable (MSVC), Node LTS.

```powershell
.\build-portable.ps1      # portable single exe
.\build-installer.ps1     # NSIS installer
```

Builds remap local paths automatically and run a privacy scan (local paths / debug entries / credential signatures fail the build). For development: `settings-ui; npm run dev` plus `src-tauri; cargo run` (calibration and QA entry points: PRD.md Appendix C).

## Repository layout

```
src-tauri/          Rust shell (window / tray / memory / theme / navigation / export / WebView2 bootstrap)
settings-ui/        Local pages: settings, tray menu, error page (Vite + TS, no framework)
scripts/            Icon generation, NSIS patching, memory & UI QA scripts
build-*.ps1         One-shot builds
measure-memory.ps1  Memory acceptance test
PRD.md              As-built spec (every requirement annotated with its implementation location)
```

The repository tracks source and config only; runtime data (`data\`) and build artifacts (`dist\`) are local files — distribute via Git.

## Known limitations

- Windows x64 only; no multi-instance (hard WebView2 user-data-folder constraint).
- Theme sampling and export selectors depend on the site's current DOM and may need adjustments after a redesign (calibrate with `DSCHAT_EXPORT_PROBE=1` in debug builds).
- No code signing; UI text is Chinese only.

## License

MIT — see [LICENSE](LICENSE). Icon art is derived from DeepSeek's public favicon; the trademark belongs to DeepSeek.
