#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK_VERSION="27.3.13750724"
NDK="$SDK/ndk/$NDK_VERSION"
NDK_BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
BUILD_TOOLS="$SDK/build-tools/${FASTEXPLORER_ANDROID_BUILD_TOOLS:-36.0.0}"
TARGET="${FASTEXPLORER_ANDROID_TARGET:-aarch64-linux-android}"

case "$TARGET" in
  aarch64-linux-android)
    ABI_NAME="arm64"
    JNI_ABI="arm64-v8a"
    TOOL_PREFIX="aarch64-linux-android"
    TARGET_ENV="AARCH64_LINUX_ANDROID"
    ;;
  x86_64-linux-android)
    ABI_NAME="x86_64"
    JNI_ABI="x86_64"
    TOOL_PREFIX="x86_64-linux-android"
    TARGET_ENV="X86_64_LINUX_ANDROID"
    ;;
  *) echo "Unsupported Android target: $TARGET" >&2; exit 2 ;;
esac

HDD_ROOT="${FASTEXPLORER_ANDROID_HDD:-/mnt/hdd/fast-explorer-android}"
APP_TARGET="$HDD_ROOT/game-activity-target"
JNI_ROOT="$HDD_ROOT/gradle-jni-$ABI_NAME"
TSNET_ARTIFACT="$HDD_ROOT/tsnet-artifacts/$TARGET/libfastexplorer_tsnet.so"
GRADLE_ROOT="$ROOT/android-gradle"
DIST="$ROOT/dist/android"


for tool in apksigner zipalign; do
  [[ -x "$BUILD_TOOLS/$tool" ]] || {
    echo "Missing $BUILD_TOOLS/$tool" >&2
    exit 1
  }
done
[[ -d "$NDK" ]] || { echo "Missing Android NDK $NDK_VERSION" >&2; exit 1; }
[[ -x "$GRADLE_ROOT/gradlew" ]] || { echo "Missing Gradle wrapper" >&2; exit 1; }

mkdir -p "$APP_TARGET" "$JNI_ROOT/$JNI_ABI" "$(dirname "$TSNET_ARTIFACT")" "$DIST"
export FASTEXPLORER_TSNET_ARTIFACT="$TSNET_ARTIFACT"
export ANDROID_HOME="$SDK"
export ANDROID_SDK_ROOT="$SDK"
export ANDROID_NDK_HOME="$NDK"
export CARGO_TARGET_DIR="$APP_TARGET"
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C strip=debuginfo -C link-arg=-Wl,-z,max-page-size=16384"

LINKER="$NDK_BIN/${TOOL_PREFIX}30-clang"
CXX="$NDK_BIN/${TOOL_PREFIX}30-clang++"
[[ -x "$LINKER" && -x "$CXX" ]] || { echo "Missing Android compiler for $TARGET" >&2; exit 1; }
export "CARGO_TARGET_${TARGET_ENV}_LINKER=$LINKER"
TARGET_ENV_LOWER="${TARGET//-/_}"
export "CC_${TARGET_ENV_LOWER}=$LINKER"
export "CXX_${TARGET_ENV_LOWER}=$CXX"
export "AR_${TARGET_ENV_LOWER}=$NDK_BIN/llvm-ar"

rustup target add "$TARGET" >/dev/null
cd "$ROOT"
cargo build --target "$TARGET" --example fast_explorer_android

RUST_SO="$APP_TARGET/$TARGET/debug/examples/libfast_explorer_android.so"
[[ -f "$RUST_SO" ]] || { echo "Rust Android library not found: $RUST_SO" >&2; exit 1; }
TSNET_SO="$TSNET_ARTIFACT"
[[ -f "$TSNET_SO" ]] || {
  echo "Embedded Tailscale Android library not found: $TSNET_SO" >&2
  exit 1
}

rm -rf "$JNI_ROOT"
mkdir -p "$JNI_ROOT/$JNI_ABI"
"$NDK_BIN/llvm-strip" --strip-debug -o \
  "$JNI_ROOT/$JNI_ABI/libfast_explorer_android.so" "$RUST_SO"
install -m 0755 "$TSNET_SO" "$JNI_ROOT/$JNI_ABI/libfastexplorer_tsnet.so"
AAPT2_SO="$GRADLE_ROOT/app/src/main/jniLibs/$JNI_ABI/libaapt2.so"
[[ -f "$AAPT2_SO" ]] || { echo "Bundled Android aapt2 not found: $AAPT2_SO" >&2; exit 1; }
install -m 0755 "$AAPT2_SO" "$JNI_ROOT/$JNI_ABI/libaapt2.so"

cd "$GRADLE_ROOT"
FASTEXPLORER_JNI_LIBS="$JNI_ROOT" ./gradlew --no-daemon clean assembleDebug
GRADLE_APK="$GRADLE_ROOT/app/build/outputs/apk/debug/app-debug.apk"
[[ -f "$GRADLE_APK" ]] || { echo "Gradle APK output not found" >&2; exit 1; }

TMP="$(mktemp -d "$HDD_ROOT/android-package-$ABI_NAME.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
ALIGNED="$TMP/fast-explorer-aligned.apk"
"$BUILD_TOOLS/zipalign" -f -P 16 4 "$GRADLE_APK" "$ALIGNED"

OUTPUT="$DIST/fast-explorer-${ABI_NAME}-debug.apk"
KEYSTORE="${ANDROID_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}"
[[ -f "$KEYSTORE" ]] || { echo "Android debug keystore not found: $KEYSTORE" >&2; exit 1; }
"$BUILD_TOOLS/apksigner" sign \
  --ks "$KEYSTORE" --ks-key-alias androiddebugkey \
  --ks-pass pass:android --key-pass pass:android \
  --out "$OUTPUT" "$ALIGNED"

"$BUILD_TOOLS/apksigner" verify --verbose "$OUTPUT" >/dev/null
"$BUILD_TOOLS/zipalign" -c -P 16 -v 4 "$OUTPUT" >/dev/null
printf 'APK: %s\n' "$OUTPUT"
