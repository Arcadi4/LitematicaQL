#!/usr/bin/env bash
# Resolve the released core normally; explicitly opt into a local crate for development.
set -euo pipefail

if [[ -n "${NQL_CORE_PATH:-}" ]]; then
  core_path="$(cd "$NQL_CORE_PATH" && pwd)"
  core_path="${core_path//\\/\\\\}"
  core_path="${core_path//\"/\\\"}"
  exec cargo --config "patch.crates-io.litematica-preview-core.path=\"$core_path\"" "$@"
fi

exec cargo "$@"
