#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT="$ROOT/.test-env"
SANDBOX="$TEST_ROOT/files"
XDG_ROOT="$TEST_ROOT/xdg"
MARKER="$TEST_ROOT/.fast-explorer-test-env"
PID_FILE="$TEST_ROOT/app.pid"
ROOT_TAG="$(printf '%s' "$ROOT" | cksum | awk '{print $1}')"
HOST_XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}"
SOCKET_DIR=""
SOCKET=""
IPC="$ROOT/scripts/fast-explorer-ipc.py"
CHECKLIST="$ROOT/docs/manual-testing.md"

usage() {
  cat <<'EOF'
Usage: ./scripts/test-env.sh [run|keep|release|reset|check|integration]

  run         reset sandbox, build debug, and launch (default)
  keep        keep current sandbox contents and launch debug
  release     reset sandbox, build release, and launch
  reset       recreate the disposable sandbox only
  check       run fmt, clippy, and unit tests
  integration run the full automated GUI integration test
EOF
}

test_instance_running() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  local command
  command="$(ps -ww -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == *"fast-explorer"* && "$command" == *"--ipc-socket"* ]]
}

setup_runtime_links() {
  mkdir -p "$TEST_ROOT/runtime"
  find "$TEST_ROOT/runtime" -maxdepth 1 -type l -delete
  if [[ -n "$HOST_XDG_RUNTIME_DIR" && -d "$HOST_XDG_RUNTIME_DIR" ]]; then
    if [[ -n "${WAYLAND_DISPLAY:-}" && "$WAYLAND_DISPLAY" != */* \
      && "$WAYLAND_DISPLAY" != "." && "$WAYLAND_DISPLAY" != ".." \
      && -S "$HOST_XDG_RUNTIME_DIR/${WAYLAND_DISPLAY}" ]]; then
      ln -s "$HOST_XDG_RUNTIME_DIR/${WAYLAND_DISPLAY}" "$TEST_ROOT/runtime/${WAYLAND_DISPLAY}"
    fi
    if [[ -S "$HOST_XDG_RUNTIME_DIR/bus" ]]; then
      ln -s "$HOST_XDG_RUNTIME_DIR/bus" "$TEST_ROOT/runtime/bus"
    fi
  fi
}

make_fixture() {
  if [[ -L "$TEST_ROOT" ]]; then
    echo "Refusing symlinked test root: $TEST_ROOT" >&2
    exit 1
  fi
  if [[ "$TEST_ROOT" != "$ROOT/.test-env" ]]; then
    echo "Refusing unsafe test root: $TEST_ROOT" >&2
    exit 1
  fi
  if [[ -f "$PID_FILE" ]]; then
    local active_pid
    active_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if test_instance_running "$active_pid"; then
      echo "FastExplorer test app is still running (PID $active_pid)." >&2
      exit 1
    fi
  fi
  if [[ -e "$TEST_ROOT" && ! -f "$MARKER" ]]; then
    echo "Refusing to clear unmarked directory: $TEST_ROOT" >&2
    exit 1
  fi
  mkdir -p "$TEST_ROOT"
  : > "$MARKER"
  find "$TEST_ROOT" -mindepth 1 -maxdepth 1 ! -name "$(basename "$MARKER")" -exec rm -rf -- {} +
  mkdir -p "$SANDBOX/00-empty"
  mkdir -p "$SANDBOX/01-folder/nested/deeper"
  mkdir -p "$SANDBOX/02-many-items"
  mkdir -p "$SANDBOX/03-search/deep"
  mkdir -p "$SANDBOX/04 folder with spaces"
  mkdir -p "$SANDBOX/05-日本語"
  printf 'alpha\n' > "$SANDBOX/alpha.txt"
  printf 'beta\n' > "$SANDBOX/beta.log"
  printf 'no extension\n' > "$SANDBOX/no-extension"
  printf 'hidden\n' > "$SANDBOX/.hidden-test.txt"
  printf 'collision\n' > "$SANDBOX/collision.txt"
  printf 'existing copy\n' > "$SANDBOX/collision - Copy.txt"
  printf 'nested\n' > "$SANDBOX/01-folder/nested/nested-file.txt"
  printf '日本語\n' > "$SANDBOX/05-日本語/日本語ファイル.txt"
  printf 'spaces\n' > "$SANDBOX/04 folder with spaces/file with spaces.txt"

  printf 'report root\n' > "$SANDBOX/03-search/report-root.txt"
  printf 'report deep\n' > "$SANDBOX/03-search/deep/report-deep.txt"
  printf 'unrelated\n' > "$SANDBOX/03-search/deep/other.txt"

  local index
  for index in {01..80}; do
    printf 'row %s\n' "$index" > "$SANDBOX/02-many-items/item-$index.txt"
  done

  local long_name="this-is-a-deliberately-long-file-name-for-layout-and-rename-testing-0123456789.txt"
  printf 'long name\n' > "$SANDBOX/$long_name"
  dd if=/dev/zero of="$SANDBOX/large-256KiB.bin" bs=1024 count=256 2>/dev/null

  ln -s "01-folder" "$SANDBOX/link-to-folder" 2>/dev/null || true
  mkdir -p "$XDG_ROOT/config" "$XDG_ROOT/state" "$XDG_ROOT/data" "$XDG_ROOT/cache"
  mkdir -p "$TEST_ROOT/runtime" "$TEST_ROOT/home/Documents" "$TEST_ROOT/home/Downloads"
  mkdir -p "$TEST_ROOT/tmp"
  printf 'home\n' > "$TEST_ROOT/home/home-file.txt"
  printf 'document\n' > "$TEST_ROOT/home/Documents/test-document.txt"
  printf 'download\n' > "$TEST_ROOT/home/Downloads/test-download.txt"
  chmod 700 "$TEST_ROOT/runtime" "$TEST_ROOT/home" "$TEST_ROOT/tmp"
  setup_runtime_links
  if [[ -z "${XAUTHORITY:-}" && -f "${HOME:-}/.Xauthority" ]]; then
    cp "${HOME}/.Xauthority" "$TEST_ROOT/home/.Xauthority"
  fi
  echo "Test sandbox reset: $SANDBOX"
}
run_checks() {
  cd "$ROOT"
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  # Prebuild the real GUI binary so integration timing measures startup, not first-link cost.
  cargo build --bin fast-explorer
}

print_checklist() {
  cat <<EOF

FastExplorer manual test sandbox
  Files:     $SANDBOX
  Socket:    $SOCKET
  Checklist: $CHECKLIST

Close FastExplorer to end this command.
Use './scripts/test-env.sh keep' to preserve your mutated fixture between runs.
EOF
  if [[ -f "$CHECKLIST" ]]; then
    awk '
      /^## Quick checklist/ { printing = 1; print; next }
      printing && /^## / { exit }
      printing { print }
    ' "$CHECKLIST"
  fi
}

launch() {
  local profile="$1"
  if [[ -L "$TEST_ROOT" || ! -f "$MARKER" ]]; then
    echo "Refusing invalid test root: $TEST_ROOT" >&2
    exit 1
  fi
  if [[ -f "$PID_FILE" ]]; then
    local active_pid
    active_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if test_instance_running "$active_pid"; then
      echo "A test instance is already running (PID $active_pid)." >&2
      exit 1
    fi
    rm -f "$PID_FILE"
  fi
  mkdir -p "$XDG_ROOT/config" "$XDG_ROOT/state" "$XDG_ROOT/data" "$XDG_ROOT/cache"
  mkdir -p "$TEST_ROOT/runtime" "$TEST_ROOT/home/Documents" "$TEST_ROOT/home/Downloads"
  mkdir -p "$TEST_ROOT/tmp"
  chmod 700 "$TEST_ROOT/runtime" "$TEST_ROOT/home" "$TEST_ROOT/tmp"
  setup_runtime_links

  cd "$ROOT"
  local -a cargo_args=(build --message-format=json)
  if [[ "$profile" == "release" ]]; then
    cargo_args+=(--release)
  fi
  local binary
  binary="$(cargo "${cargo_args[@]}" | python3 -c '
import json, sys
executable = None
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "compiler-message":
        rendered = message.get("message", {}).get("rendered")
        if rendered:
            sys.stderr.write(rendered)
    target = message.get("target", {})
    if (
        message.get("reason") == "compiler-artifact"
        and target.get("name") == "fast-explorer"
        and "bin" in target.get("kind", [])
        and message.get("executable")
    ):
        executable = message["executable"]
if not executable:
    raise SystemExit("cargo build did not report the fast-explorer executable")
print(executable)
')"

  local socket_base="${HOST_XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"
  if [[ ! -d "$socket_base" || ! -w "$socket_base" ]]; then
    socket_base="${TMPDIR:-/tmp}"
  fi
  SOCKET_DIR="$(mktemp -d "$socket_base/fast-explorer-test-${UID:-0}-${ROOT_TAG}.XXXXXX")"
  chmod 700 "$SOCKET_DIR"
  SOCKET="$SOCKET_DIR/control.sock"

  local -a app_command=("$binary" --ipc-socket "$SOCKET")
  local filesystem_guard="environment isolation only"
  if [[ "${FASTEXPLORER_TEST_NO_BWRAP:-0}" != "1" ]] \
    && command -v bwrap >/dev/null \
    && bwrap --die-with-parent --ro-bind / / --dev-bind /dev /dev --proc /proc true 2>/dev/null; then
    local -a bwrap_command=(
      bwrap --die-with-parent --ro-bind / / --bind "$TEST_ROOT" "$TEST_ROOT"
      --bind "$SOCKET_DIR" "$SOCKET_DIR"
    )
    bwrap_command+=(--dev-bind /dev /dev --proc /proc -- "${app_command[@]}")
    app_command=("${bwrap_command[@]}")
    filesystem_guard="bubblewrap: normal filesystem is read-only; disposable test dirs stay writable"
  fi

  HOME="$TEST_ROOT/home" TMPDIR="$TEST_ROOT/tmp" \
  XDG_CONFIG_HOME="$XDG_ROOT/config" XDG_STATE_HOME="$XDG_ROOT/state" \
  XDG_DATA_HOME="$XDG_ROOT/data" XDG_CACHE_HOME="$XDG_ROOT/cache" \
  XDG_RUNTIME_DIR="$TEST_ROOT/runtime" \
    "${app_command[@]}" &
  local app_pid=$!
  printf '%s\n' "$app_pid" > "$PID_FILE"
  echo "Safety: $filesystem_guard"

  cleanup() {
    if kill -0 "$app_pid" 2>/dev/null; then
      kill "$app_pid" 2>/dev/null || true
      local attempt
      for attempt in {1..20}; do
        kill -0 "$app_pid" 2>/dev/null || break
        sleep 0.1
      done
      if kill -0 "$app_pid" 2>/dev/null; then
        kill -KILL "$app_pid" 2>/dev/null || true
      fi
      wait "$app_pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE" "$SOCKET"
    rmdir "$SOCKET_DIR" 2>/dev/null || true
  }
  trap cleanup EXIT
  trap 'exit 130' INT TERM

  if ! python3 "$IPC" --socket "$SOCKET" wait --timeout 30 >/dev/null; then
    echo "FastExplorer exited or IPC did not become ready." >&2
    cleanup
    trap - INT TERM EXIT
    return 1
  fi
  python3 "$IPC" --socket "$SOCKET" navigate "$SANDBOX" >/dev/null
  print_checklist
  wait "$app_pid"
  rm -f "$PID_FILE" "$SOCKET"
  rmdir "$SOCKET_DIR" 2>/dev/null || true
  trap - INT TERM EXIT
}

command="${1:-run}"
case "$command" in
  run)
    make_fixture
    launch debug
    ;;
  keep)
    if [[ ! -f "$MARKER" || ! -d "$SANDBOX" ]]; then
      make_fixture
    fi
    launch debug
    ;;
  release)
    make_fixture
    launch release
    ;;
  reset)
    make_fixture
    ;;
  check)
    run_checks
    ;;
  integration)
    exec "$ROOT/scripts/integration-test.sh"
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "Unknown command: $command" >&2
    usage >&2
    exit 2
    ;;
esac
