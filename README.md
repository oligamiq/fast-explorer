# FastExplorer

FastExplorer is an Explorer-inspired file manager built with Xilem.

## Implemented

- Client-side integrated title bar: back / forward / up / home navigation, tabs, new-tab (`+`), draggable caption area, minimize, maximize/restore, and window close share one bar
- Per-tab close (`x`) is contained inside each tab surface
- Search field beside the address bar with `Default` recursive search and Windows `Everything` integration
- Closing the final tab closes FastExplorer itself
- Per-tab directory, address, hidden-file setting, and back/forward history
- Session restore for tabs, active tab, paths, hidden-file state, and navigation history
- Progressive disclosure: frequent navigation/search/file actions stay on the main surface; hidden-file, search-backend, appearance, Tailscale, and external-control options live behind the settings (`⚙`) button
- Primary file actions: New folder, Cut, Copy, Paste, Rename, and Delete-to-Trash; New folder immediately enters rename mode
- Desktop selection behavior: single click selects, double click opens/navigates, and `Enter` opens the selected item with the OS default handler
- Explorer-style file-list keyboard layer: one list focus target (rows are not individual Tab stops), `↑` / `↓`, Home / End, Page Up / Page Down, Enter, F2, Delete, Ctrl+C / X / V, Ctrl+Shift+N, Backspace, F5, and Alt+Left / Right / Up where the focused control does not reserve those keys
- `System` / `Light` / `Dark` appearance modes
- Direct color themes: Blue, Red, Green, Purple, Orange, Teal, Pink, and Neutral
- Configurable theme intensity (`0..100`, default `72`) that strongly tints chrome, sidebar, headers, tabs, surfaces, borders, and selections
- Automatic semantic accent palette generation from one theme color using Oklab interpolation
- Contrast-aware accent text selection with a WCAG 4.5:1 minimum test target
- Temporary startup overrides for appearance, color, and intensity
- Versioned local JSON-lines control protocol for live external settings/navigation control
- Multiple simultaneous embedded Tailscale (`tsnet`) connections, each with independent persistent identity, browser sign-in, tailnet peer discovery, and FastExplorer-to-FastExplorer connectivity tests
- Explorer-like command bar and editable address bar
- Back / forward / up / home / refresh navigation
- Quick-access sidebar for Home, Desktop, Documents, Downloads, Pictures, and available Windows drive roots
- Real directory enumeration with folders-first sorting
- Name / type / size list columns
- Hidden-file toggle
- Scrollable file list and draggable sidebar split
- Navigation history and status bar

## Structure

- `src/main.rs` — window and Xilem event loop
- `src/app.rs` — navigation, persistence, filesystem behavior, theme selection, and networking UI state
- `src/tailscale.rs` — Rust-side embedded Tailscale FFI, worker, status, peer, and connection-test integration
- `tailscale-bridge/` — pinned Go `tsnet` bridge and tailnet-only FastExplorer service
- `build.rs` — builds/links the Go bridge for Linux, Android ARM64, and Windows x64
- `vendor/masonry_winit/` — Xilem window/render bridge with FastExplorer CPU/software fallback patch
- `src/theme.rs` — color-theme seeds, semantic palette generation, contrast logic, and layout tokens
- `src/ui/mod.rs` — top-level composition only
- `src/ui/components.rs` — replaceable Explorer-like UI regions
- `src/ui/file_shortcuts.rs` — Explorer-style list/browser keyboard command routing
- `src/ui/file_row.rs` — non-Tab-stop file-row pointer and AccessKit interaction
- `src/ui/window_chrome.rs` — native Masonry window drag, caption controls, AccessKit labels, and edge/corner resize regions

## Session data

FastExplorer saves session state when the application closes. On Linux it uses
`$XDG_STATE_HOME/fast-explorer/session.json`, falling back to
`~/.local/state/fast-explorer/session.json` when `XDG_STATE_HOME` is unset. On Windows it uses `%LOCALAPPDATA%\FastExplorer\session.json`.
The live file listing is rebuilt from the filesystem on startup rather than serialized.

## Settings

Appearance, search, and Tailnet profile preferences are global configuration, not session data. On Linux they are stored at
`$XDG_CONFIG_HOME/fast-explorer/config.json`, falling back to
`~/.config/fast-explorer/config.json`; Windows uses `%APPDATA%\FastExplorer\config.json`. `System` mode detects the desktop preference at startup
and whenever System mode is selected. Intensity accepts any integer from `0` through `100`.

```json
{"appearance":"dark","color":"red","intensity":80,"search_mode":"default"}
```

The Settings page changes these values persistently; intensity can be adjusted with `-` and `+`.

## Embedded Tailscale

Open Settings (`⚙`) → Tailscale and choose `Add Tailnet`. Every entry owns an independent embedded `tsnet` node, state directory, login URL, peer list, and FastExplorer service, so multiple Tailnets can remain connected at the same time. No external `tailscaled` process or installed Tailscale client is required. `Disconnect` stops that embedded node while preserving its identity for later reconnect; `Sign out` explicitly logs the node out. `Test` performs a real FastExplorer-to-FastExplorer request through the selected Tailnet.

The tailnet service exposes protocol/version/identity discovery at `fast-explorer-tailnet/1` and WebDAV on its tsnet-only listener. Tailscale policy controls reachability and every request is identified with Tailscale `WhoIs`; WebDAV additionally requires the caller to belong to the same Tailscale user as the local FastExplorer node. WebDAV filesystem access is available under `/dav/`, and its filesystem is constrained with Go `os.Root` so symlinks inside the share cannot escape the configured root. The shared root is the current user home directory on desktop and shared storage (`/storage/emulated/0` on standard Android devices) on Android.

The embedded bridge is built for Linux desktop, Android ARM64/x86_64, and Windows x64. Linux stores profile state below `$XDG_CONFIG_HOME/fast-explorer/tailscale`; Windows uses `%LOCALAPPDATA%\FastExplorer\tailscale`; Android uses the app's private internal data directory. Every Tailnet profile receives its own locked subdirectory and identity. Removing a profile removes its configuration but intentionally leaves its identity directory untouched.

## Temporary startup overrides

```bash
fast-explorer --appearance dark --theme-color red --theme-intensity 100 --search-mode everything
```

Startup overrides change only the running process and do not rewrite `config.json`. Use `--help` for all flags.

## Search backends

- `Default` — FastExplorer recursively searches names below the active directory, with a 500-result cap.
- `Everything` — Windows integration through the official Everything `ES` command-line interface. `es.exe` must be available, Everything must be running, and ES must support `-argv` (ES 1.1.0.37 or newer). FastExplorer uses `-argv` plus `--` so the user's Everything search expression is preserved while option parsing is disabled for the search text. The active directory is passed as the search path, so results remain scoped to the current tab location.

Search text and active search results are tab-local. Changing directories clears that tab's search.

## External control

FastExplorer exposes the versioned `fast-explorer/1` JSON-lines protocol over a local Unix socket on Unix systems and a local-only Windows Named Pipe on Windows. The default Windows pipe name contains the current SID plus a fresh 128-bit CSPRNG nonce on every launch. FastExplorer binds that unpredictable pipe first and only then publishes the current endpoint in `%LOCALAPPDATA%\FastExplorer\control-endpoint`; the directory, endpoint file, and pipe all use the current-user/SYSTEM/Administrators protected ACL. Rotating the nonce prevents reuse of an observed endpoint, while bind-before-publish prevents same-user pipe squatting during startup.
See `docs/control-protocol.md` for socket locations, security rules, request envelopes, and methods.

## UI/UX references

Project-local UI/UX Agent Skills are installed under `.agents/skills/` for Codex and Antigravity-compatible tooling. The project workflow and native-Xilem adaptation rules are documented in `AGENTS.md` and `docs/ui-ux-skills.md`.

Current references: Anthropic `frontend-design`, Vercel `web-design-guidelines`, `make-interfaces-feel-better`, and `fixing-accessibility`.

## Manual testing

Create an isolated disposable fixture and launch FastExplorer directly into it:

```bash
./scripts/test-env.sh
```

The launcher keeps config/state/data/cache/home/temp data under `.test-env/`. On Linux
with `bubblewrap`, normal filesystem paths are read-only; only the disposable test area
and a securely-created test IPC directory stay writable. XDG runtime data is isolated too.
Use `keep` to preserve fixture mutations or `reset` to recreate the fixture.

Run the complete automated integration check with one command:

```bash
./scripts/test-env.sh integration
```

It runs fmt/clippy/unit tests, launches the isolated GUI, drives real X11 keyboard/mouse
input, verifies navigation and file mutations on disk, closes the app, and resets the fixture.
See `docs/manual-testing.md` for details and the manual fallback checklist.

## Android build

Build the ARM64 debug APK with:

```bash
./scripts/android-build.sh
```

The generated APK is copied to `dist/android/fast-explorer-arm64-debug.apk`. The package includes the ARM64 Go `tsnet` shared library automatically, plus Android `INTERNET` / network-state permissions. Android 11+ all-files access and system-bar/back handling are implemented; system file opening and Trash integration still need platform adapters.

For emulator testing without consuming SSD space, FastExplorer keeps both the API 35 x86_64 system image and AVD data below `/mnt/hdd/fast-explorer-android` by default:

```bash
./scripts/android-emulator-setup.sh
./scripts/android-emulator-run.sh -no-window -no-audio
FASTEXPLORER_ANDROID_TARGET=x86_64-linux-android ./scripts/android-build.sh
```

Override the HDD location with `FASTEXPLORER_ANDROID_HDD=/path/on/hdd`. The prepared AVD is named `FastExplorer_API35`.

## Windows build

On x64 Windows, install the Rust MSVC toolchain, Go, and a GCC-compatible C compiler for cgo, then run `./scripts/windows-build.ps1` from PowerShell. From Linux, `./scripts/windows-cross-build.sh` uses `cargo-xwin` plus Zig/cgo to produce the same two-file x64 package. Both paths place `fast-explorer.exe` and the embedded-Tailscale `fast_explorer_tsnet.dll` in `dist/windows`. FastExplorer dynamically loads that DLL with restricted dependency-search flags, so no MSVC import library is required. Local external control uses a per-launch SID-and-nonce Windows Named Pipe; read the current name from `%LOCALAPPDATA%\FastExplorer\control-endpoint`.

## Software rendering fallback

FastExplorer keeps Xilem/wgpu’s normal hardware-adapter selection first. If that adapter request fails, the locally patched `masonry_winit` retries with `force_fallback_adapter = true`, enabling the platform software/CPU adapter where available. Set `FASTEXPLORER_FORCE_CPU=1` to request the fallback adapter directly for testing. No process-wide graphics environment variables are mutated after driver initialization.

## Screenshots

- `screenshots/review/` — screenshots intended for user review
- `screenshots/internal/` — development/debug screenshots

Review screenshots should never be written directly into `screenshots/`.

## Current performance boundary

Directory enumeration is intentionally synchronous in this first functional scaffold.
Before targeting very large, network, or virtual directories, move scanning and metadata reads behind a background filesystem service while keeping `AppState` and the UI component API stable.

## Validation

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
