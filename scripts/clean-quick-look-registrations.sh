#!/bin/zsh

set -euo pipefail

if ! command -v pluginkit >/dev/null 2>&1; then
  print -u2 'pluginkit is required to clean Quick Look registrations.'
  exit 1
fi

keep_extension_path="${LITEMATICAQL_KEEP_EXTENSION_PATH:-/Applications/LitematicaQL.app/Contents/PlugIns/LitematicaQLPreview.appex}"
typeset -A seen_paths=()

unregister() {
  local extension_path="$1"

  [[ -z "$extension_path" || "$extension_path" == "$keep_extension_path" ]] && return
  [[ "${seen_paths[$extension_path]-}" == 1 ]] && return
  seen_paths[$extension_path]=1

  pluginkit -r "$extension_path"
  print "unregistered: $extension_path"
}

for identifier in \
  moe.arcadia.LitematicaQL.PreviewExtension
do
  while IFS= read -r extension_path; do
    unregister "$extension_path"
  done < <(
    pluginkit -m -D -v \
      -p com.apple.quicklook.preview \
      -i "$identifier" \
      | awk -F $'\t' 'NF >= 4 { print $NF }'
  )
done
