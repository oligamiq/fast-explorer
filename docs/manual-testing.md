# Manual testing

FastExplorer has a disposable manual-test environment under `.test-env/`.
The normal development workflow is one command:

```bash
./scripts/test-env.sh
```

This recreates the fixture, builds the debug binary, launches FastExplorer with isolated
config/state/data/cache/home/temp directories, waits for IPC, and opens the fixture automatically.
When `bubblewrap` is available, normal filesystem paths are mounted read-only; only the
disposable test area and a securely-created test IPC directory stay writable. XDG runtime
is redirected into `.test-env/runtime`, with desktop sockets linked in only when needed.

## Commands

```bash
./scripts/test-env.sh run          # reset + debug launch; default
./scripts/test-env.sh keep         # launch without resetting your mutations
./scripts/test-env.sh release      # reset + release launch
./scripts/test-env.sh reset        # recreate fixture only
./scripts/test-env.sh check        # fmt + clippy + cargo test
./scripts/test-env.sh integration  # full automated GUI integration test
```

`integration` is the normal one-command health check. It runs static/unit validation,
launches the sandboxed GUI, injects real X11/XTest input, verifies resulting IPC state and
filesystem mutations, closes FastExplorer with Alt+F4, and resets the fixture on success.
Regular-file `Enter` is verified through an integration-only `xdg-open` stub, so the test
checks OS-opener delegation without launching a real editor/viewer. It currently requires
an X11/XWayland display (`DISPLAY` must be set).

Set `FASTEXPLORER_TEST_NO_BWRAP=1` only when the local desktop environment cannot run
FastExplorer inside bubblewrap. HOME and XDG config/state/data/cache plus temp data remain
isolated, but the filesystem read-only guard is disabled in that mode.

## Fixture map

- `00-empty/` — empty-directory navigation and Backspace/Alt navigation
- `01-folder/nested/deeper/` — navigation history and nested directories
- `02-many-items/` — 80 rows for scrolling, Home/End, PageUp/PageDown
- `03-search/` — nested `report-*` files for recursive search testing
- `04 folder with spaces/` — paths and names containing spaces
- `05-日本語/` — Unicode directory and filename handling
- `.hidden-test.txt` — hidden-file visibility
- `alpha.txt`, `beta.log`, `no-extension` — ordinary rename/open cases
- `collision.txt` + `collision - Copy.txt` — duplicate-name handling
- `link-to-folder` — symlink behavior when the host supports symlinks
- long filename + 256 KiB file — layout and size display
- isolated `home/`, `home/Documents/`, `home/Downloads/` — safe Quick access targets

The whole `.test-env/` directory is ignored by Git. `reset` refuses to clear an existing
test directory unless the expected marker exists and no recorded test process is still running.

## Quick checklist

1. Click `alpha.txt`, then use ↑/↓, Home/End, PageUp/PageDown.
2. Press F2, type a new stem, then Enter. The `.txt` extension should remain.
3. Press Ctrl+C then Ctrl+V. Exactly one copied file should appear.
4. Press Ctrl+X then Ctrl+V in the same directory. It should be a no-op.
5. Press Ctrl+Shift+N, type a name, Enter; repeat and use Esc to cancel.
6. Enter `00-empty`, then use Backspace and Alt+Left/Right/Up.
7. Enter `02-many-items` and verify keyboard selection stays visible while scrolling.
8. Enter `03-search`, search `report`, then test refresh/rename/copy/delete behavior.
9. Right-click an item: it may select, but must not trigger double-click/open behavior.
10. Toggle hidden files and verify `.hidden-test.txt` appears/disappears.
11. Test `file with spaces.txt`, `日本語ファイル.txt`, the long filename, and `no-extension`.
12. Close the FastExplorer window normally, then relaunch with `keep` to inspect session/config behavior without resetting files.

For session-restore tests, close the app with its window close button. Ctrl+C is an abort path
for the launcher and may terminate the sandbox before FastExplorer's normal close callback runs.

## IPC helper

The launcher uses the stdlib-only helper below, which is also useful while debugging:

Copy the socket path printed by `test-env.sh` into `SOCKET`:

```bash
SOCKET='/run/user/1000/fast-explorer-test-1000-123456789.A1b2C3/control.sock' # example; replace it
python3 scripts/fast-explorer-ipc.py --socket "$SOCKET" state
python3 scripts/fast-explorer-ipc.py --socket "$SOCKET" navigate /absolute/path
python3 scripts/fast-explorer-ipc.py --socket "$SOCKET" search report
python3 scripts/fast-explorer-ipc.py --socket "$SOCKET" clear-search
```

The actual socket path is printed by `test-env.sh`. Each launch uses a securely-created
private temporary directory, so stale sockets and concurrent normal FastExplorer instances do not collide.
