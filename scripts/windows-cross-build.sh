#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TARGET=x86_64-pc-windows-msvc
DIST="$ROOT/dist/windows"
TOOLS="${HOME}/.cache/fast-explorer-tools"

CARGO_XWIN=${CARGO_XWIN:-$(command -v cargo-xwin || true)}
ZIG=${ZIG:-$(command -v zig || true)}
[[ -n "$CARGO_XWIN" ]] || CARGO_XWIN="$TOOLS/cargo-xwin"
[[ -n "$ZIG" ]] || ZIG="$TOOLS/zig-x86_64-linux-0.16.0/zig"
[[ -x "$CARGO_XWIN" ]] || { echo "cargo-xwin not found" >&2; exit 1; }
[[ -x "$ZIG" ]] || { echo "zig not found" >&2; exit 1; }

mkdir -p "$DIST"
cd "$ROOT/tailscale-bridge"
CGO_ENABLED=1 GOOS=windows GOARCH=amd64 \
  CC="$ZIG cc -target x86_64-windows-gnu" GOTOOLCHAIN=go1.26.5 \
  go build -buildmode=c-shared -o "$DIST/fast_explorer_tsnet.dll" .

cd "$ROOT"
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static" \
  "$CARGO_XWIN" build --release --target "$TARGET" --bin fast-explorer
cp "target/$TARGET/release/fast-explorer.exe" "$DIST/fast-explorer.exe"
echo "Windows package: $DIST"
