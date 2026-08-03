#!/bin/zsh

# LitematicaQL: macOS Quick Look plugin for Litematica schematics.
# Copyright (C) 2026 4rcadia
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as published
# by the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.
#
# See the LICENSE file for the full license text.

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
