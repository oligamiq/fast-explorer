#!/usr/bin/env bash
set -euo pipefail
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
HDD_ROOT="${FASTEXPLORER_ANDROID_HDD:-/mnt/hdd/fast-explorer-android}"
AVD_HOME="$HDD_ROOT/avd"
IMAGE='system-images;android-35;google_apis;x86_64'
mkdir -p "$HDD_ROOT/system-images" "$AVD_HOME" "$HDD_ROOT/cache"
if [[ ! -L "$SDK/system-images" ]]; then
  [[ ! -e "$SDK/system-images" ]] || { echo "$SDK/system-images must be moved before setup" >&2; exit 1; }
  ln -s "$HDD_ROOT/system-images" "$SDK/system-images"
fi
export ANDROID_HOME="$SDK" ANDROID_AVD_HOME="$AVD_HOME"
set +o pipefail
yes | "$SDK/cmdline-tools/latest/bin/sdkmanager" "emulator" "$IMAGE" >/dev/null
SDKMANAGER_STATUS=${PIPESTATUS[1]}
set -o pipefail
(( SDKMANAGER_STATUS == 0 )) || exit "$SDKMANAGER_STATUS"
if [[ ! -d "$AVD_HOME/FastExplorer_API35.avd" ]]; then
  printf 'no\n' | "$SDK/cmdline-tools/latest/bin/avdmanager" create avd \
    -n FastExplorer_API35 -k "$IMAGE" -d pixel_6 --force
fi
AVD_DIR="$AVD_HOME/FastExplorer_API35.avd"
printf 'AVD: %s\nAll writable emulator images stay under this HDD-backed AVD directory.\n' "$AVD_DIR"
df -h "$HDD_ROOT"
