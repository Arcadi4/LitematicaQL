#!/usr/bin/env bash
# LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
# Copyright (c) 2026 4rcadia
# SPDX-License-Identifier: MIT
#
# Builds the native schematic bridge as a static library per architecture.
# Each architecture gets its own artifact, `build/liblitematicaql_native_arm64.a`
# and `build/liblitematicaql_native_x86_64.a`. There is deliberately no fat
# library; the Xcode build links the one matching the current architecture.
#
# Usage is `build.sh [arch ...]`, defaulting to $ARCHS when set by Xcode, else
# the host architecture.

set -euo pipefail
cd "$(dirname "$0")"

if [[ $# -gt 0 ]]; then
  archs=("$@")
elif [[ -n "${ARCHS:-}" ]]; then
  read -ra archs <<<"$ARCHS"
else
  archs=("$(uname -m)")
fi

# A Homebrew rust on PATH ships only its host standard library, and a toolchain
# cargo invoked directly resolves `rustc` from PATH, so pin RUSTC to the
# toolchain that `target add` installed the cross std into.
if command -v rustup >/dev/null 2>&1; then
  export RUSTC="$(rustup which rustc)"
fi

for arch in "${archs[@]}"; do
  case "$arch" in
    arm64) triple="aarch64-apple-darwin" ;;
    x86_64) triple="x86_64-apple-darwin" ;;
    *)
      echo "build.sh: unsupported architecture '$arch'" >&2
      exit 1
      ;;
  esac
  if ! rustup target list --installed | grep -qx "$triple"; then
    rustup target add "$triple"
  fi

  cargo build --release --target "$triple"

  mkdir -p build
  # Same-volume move, so concurrent target builds never observe a partial copy.
  cp -f "target/$triple/release/liblitematicaql_native.a" "build/.tmp_$arch.a"
  mv -f "build/.tmp_$arch.a" "build/liblitematicaql_native_$arch.a"
done
