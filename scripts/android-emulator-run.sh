#!/usr/bin/env bash
set -euo pipefail
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
HDD_ROOT="${FASTEXPLORER_ANDROID_HDD:-/mnt/hdd/fast-explorer-android}"
export ANDROID_HOME="$SDK" ANDROID_AVD_HOME="$HDD_ROOT/avd"
exec "$SDK/emulator/emulator" -avd FastExplorer_API35 \
  -no-snapshot -no-boot-anim -gpu lavapipe "$@"
