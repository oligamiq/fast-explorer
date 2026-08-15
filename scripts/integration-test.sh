#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ENV="$ROOT/scripts/test-env.sh"
IPC="$ROOT/scripts/fast-explorer-ipc.py"
X11="$ROOT/scripts/x11-test-driver.py"
TEST_ROOT="$ROOT/.test-env"
SANDBOX="$TEST_ROOT/files"
TEST_BIN="$TEST_ROOT/test-bin"
OPEN_LOG="$TEST_ROOT/opened-paths.log"
LOG="$(mktemp "${TMPDIR:-/tmp}/fast-explorer-integration.XXXXXX.log")"
LAUNCHER_PID=""
WINDOW=""
SOCKET=""
PASS_COUNT=0

pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf '  PASS  %s\n' "$1"
}

fail() {
  printf '  FAIL  %s\n' "$1" >&2
  printf '\nLauncher log: %s\n' "$LOG" >&2
  tail -80 "$LOG" >&2 || true
  exit 1
}

cleanup() {
  if [[ -n "$LAUNCHER_PID" ]] && kill -0 "$LAUNCHER_PID" 2>/dev/null; then
    kill -TERM "$LAUNCHER_PID" 2>/dev/null || true
    wait "$LAUNCHER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM
ipc() {
  python3 "$IPC" --socket "$SOCKET" "$@"
}

state_value() {
  local field="$1"
  ipc state | python3 -c '
import json, sys
value = json.load(sys.stdin)["result"].get(sys.argv[1])
if isinstance(value, bool):
    print("true" if value else "false")
elif value is not None:
    print(value)
' "$field"
}

wait_state() {
  local field="$1" expected="$2" label="$3"
  local attempt actual=""
  for attempt in {1..80}; do
    actual="$(state_value "$field" 2>/dev/null || true)"
    if [[ "$actual" == "$expected" ]]; then
      pass "$label"
      return 0
    fi
    sleep 0.05
  done
  fail "$label (expected '$expected', got '$actual')"
}

wait_exists() {
  local path="$1" label="$2"
  local attempt
  for attempt in {1..80}; do
    [[ -e "$path" ]] && { pass "$label"; return 0; }
    sleep 0.05
  done
  fail "$label (missing: $path)"
}

wait_missing() {
  local path="$1" label="$2"
  local attempt
  for attempt in {1..80}; do
    [[ ! -e "$path" ]] && { pass "$label"; return 0; }
    sleep 0.05
  done
  fail "$label (still exists: $path)"
}

wait_opened() {
  local path="$1" label="$2"
  local attempt
  for attempt in {1..80}; do
    if [[ -f "$OPEN_LOG" ]] && grep -Fxq -- "$path" "$OPEN_LOG"; then
      pass "$label"
      return 0
    fi
    sleep 0.05
  done
  fail "$label (default-open stub was not called for: $path)"
}
wait_socket_from_log() {
  local attempt
  for attempt in {1..800}; do
    SOCKET="$(awk '/^  Socket:/ { print $2; exit }' "$LOG" 2>/dev/null || true)"
    [[ -n "$SOCKET" && -S "$SOCKET" ]] && return 0
    if [[ -n "$LAUNCHER_PID" ]] && ! kill -0 "$LAUNCHER_PID" 2>/dev/null; then
      fail "FastExplorer launcher exited before IPC became ready"
    fi
    sleep 0.05
  done
  fail "timed out waiting for FastExplorer IPC socket"
}

xkey() {
  "$X11" key --window "$WINDOW" "$1"
}

xtype() {
  "$X11" type --window "$WINDOW" "$1"
}

click_first_row() {
  local button="${1:-primary}"
  # Default 1180x760 client layout: list begins below the 150px chrome/header region.
  "$X11" click --window "$WINDOW" --x 420 --y 166 --button "$button"
}

printf 'FastExplorer integration test\n'
printf '=============================\n'

[[ -n "${DISPLAY:-}" ]] || fail "DISPLAY is not set; GUI integration tests require X11/XWayland"
python3 -m py_compile "$IPC" "$X11"
chmod +x "$TEST_ENV" "$IPC" "$X11"

printf '\n[1/3] Static/unit validation\n'
"$TEST_ENV" check
pass "fmt + clippy + unit tests"

printf '\n[2/3] Isolated GUI launch\n'
"$TEST_ENV" reset >/dev/null
mkdir -p "$TEST_BIN"
cat > "$TEST_BIN/xdg-open" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$FASTEXPLORER_TEST_OPEN_LOG"
EOF
chmod +x "$TEST_BIN/xdg-open"
: > "$OPEN_LOG"
before_windows="$($X11 windows --title FastExplorer | paste -sd, -)"
PATH="$TEST_BIN:$PATH" FASTEXPLORER_TEST_OPEN_LOG="$OPEN_LOG" \
  WINIT_UNIX_BACKEND=x11 "$TEST_ENV" keep >"$LOG" 2>&1 &
LAUNCHER_PID=$!
wait_socket_from_log
WINDOW="$($X11 wait-window --title FastExplorer --exclude "$before_windows" --timeout 5)" || fail "FastExplorer X11 window did not appear"
wait_state path "$SANDBOX" "startup opens disposable fixture"
printf '\n[3/3] End-to-end Explorer operations\n'

click_first_row
wait_state selected_path "$SANDBOX/00-empty" "mouse click selects first row"
xkey Down
wait_state selected_path "$SANDBOX/01-folder" "Down moves selection"
xkey Up
wait_state selected_path "$SANDBOX/00-empty" "Up moves selection"
xkey Return
wait_state path "$SANDBOX/00-empty" "Enter opens selected folder"
xkey BackSpace
wait_state path "$SANDBOX" "Backspace navigates back"
xkey alt+Right
wait_state path "$SANDBOX/00-empty" "Alt+Right navigates forward"
xkey alt+Left
wait_state path "$SANDBOX" "Alt+Left navigates back"
click_first_row secondary
wait_state selected_path "$SANDBOX/00-empty" "right-click selects without opening"
xkey Down
wait_state selected_path "$SANDBOX/01-folder" "keyboard focus survives right-click selection"

ipc search beta >/dev/null
wait_state selected_path "$SANDBOX/beta.log" "async search completes and selects first result"
xkey Return
wait_opened "$SANDBOX/beta.log" "Enter delegates regular file to the OS opener"
ipc clear-search >/dev/null

ipc navigate "$SANDBOX/02-many-items" >/dev/null
wait_state path "$SANDBOX/02-many-items" "IPC navigation reaches scrolling fixture"
click_first_row
wait_state selected_path "$SANDBOX/02-many-items/item-01.txt" "first row selection in long list"
xkey Page_Down
wait_state selected_path "$SANDBOX/02-many-items/item-11.txt" "PageDown advances ten rows"
xkey End
wait_state selected_path "$SANDBOX/02-many-items/item-80.txt" "End selects final row"
xkey Home
wait_state selected_path "$SANDBOX/02-many-items/item-01.txt" "Home selects first row"

ipc navigate "$SANDBOX" >/dev/null
ipc search alpha >/dev/null
wait_state selected_path "$SANDBOX/alpha.txt" "search result can be selected"
xkey F2
xtype renamed
xkey Return
wait_exists "$SANDBOX/renamed.txt" "F2 direct typing renames and preserves extension"
wait_missing "$SANDBOX/alpha.txt" "rename removes old path"
ipc clear-search >/dev/null
ipc search beta >/dev/null
wait_state selected_path "$SANDBOX/beta.log" "copy source selected"
xkey ctrl+c
xkey ctrl+v
wait_exists "$SANDBOX/beta - Copy.log" "Ctrl+C / Ctrl+V creates one copy"
sleep 0.35
[[ ! -e "$SANDBOX/beta - Copy 2.log" ]] || fail "Ctrl+V fired more than once"
pass "paste fires exactly once"

ipc clear-search >/dev/null
click_first_row
xkey ctrl+shift+n
xtype integrationfolder
xkey Return
wait_exists "$SANDBOX/integrationfolder" "Ctrl+Shift+N creates and renames folder"

ipc search no-extension >/dev/null
wait_state selected_path "$SANDBOX/no-extension" "delete source selected"
xkey Delete
wait_missing "$SANDBOX/no-extension" "Delete moves selected file out of directory"

ipc clear-search >/dev/null
ipc search refreshprobe >/dev/null
wait_state selected_path "" "no results initially" || true
printf 'refresh\n' > "$SANDBOX/refreshprobe.txt"
xkey F5
wait_state selected_path "$SANDBOX/refreshprobe.txt" "F5 reruns active search and discovers external changes"
rm -f "$SANDBOX/refreshprobe.txt"
ipc clear-search >/dev/null

printf '\nClosing FastExplorer through the desktop shortcut...\n'
xkey alt+F4
for _ in {1..100}; do
  if ! kill -0 "$LAUNCHER_PID" 2>/dev/null; then
    wait "$LAUNCHER_PID" || fail "launcher returned failure after window close"
    LAUNCHER_PID=""
    pass "Alt+F4 closes app and launcher cleanly"
    break
  fi
  sleep 0.05
done
[[ -z "$LAUNCHER_PID" ]] || fail "FastExplorer did not close after Alt+F4"

trap - EXIT INT TERM
"$TEST_ENV" reset >/dev/null
rm -f "$LOG"
printf '\nSUCCESS: %d integration checks passed. Fixture reset to a clean state.\n' "$PASS_COUNT"
