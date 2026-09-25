#!/usr/bin/env bash
#
# Build one static library per requested architecture. Xcode supplies `$ARCHS`;
# direct invocation defaults to the host architecture and rejects other values.

set -euo pipefail
cd "$(dirname "$0")"

if [[ $# -gt 0 ]]; then
  archs=("$@")
elif [[ -n "${ARCHS:-}" ]]; then
  read -ra archs <<<"$ARCHS"
else
  archs=("$(uname -m)")
fi

# Homebrew's Rust exposes only the host standard library, while cross targets
# come from rustup. Pin `RUSTC` to the rustup toolchain that owns those targets.
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
  # Publish by same-volume rename so readers never see a partial artifact.
  cp -f "target/$triple/release/liblitematicaql_native.a" "build/.tmp_$arch.a"
  mv -f "build/.tmp_$arch.a" "build/liblitematicaql_native_$arch.a"
done
