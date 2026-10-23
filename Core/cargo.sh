#!/usr/bin/env bash
# Resolve the git-pinned core normally; explicitly opt into a local crate for development.
set -euo pipefail

if [[ -n "${LP_CORE_PATH:-}" ]]; then
  core_path="$(cd "$LP_CORE_PATH" && pwd)"
  core_path="${core_path//\\/\\\\}"
  core_path="${core_path//\"/\\\"}"
  args=()
  for arg in "$@"; do
    [[ "$arg" == "--locked" ]] || args+=("$arg")
  done
  exec cargo --config "patch.\"https://github.com/Arcadi4/litematica-preview-core\".litematica-preview-core.path=\"$core_path\"" "${args[@]}"
fi

exec cargo "$@"
