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

repo_root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_root"

copyright_marker='Copyright (C) 2026 4rcadia'
notice_lines=(
  'LitematicaQL: macOS Quick Look plugin for Litematica schematics.'
  "$copyright_marker"
  'SPDX-License-Identifier: AGPL-3.0-or-later'
  ''
  'This program is free software: you can redistribute it and/or modify'
  'it under the terms of the GNU Affero General Public License as published'
  'by the Free Software Foundation, either version 3 of the License, or'
  '(at your option) any later version.'
  ''
  'This program is distributed in the hope that it will be useful,'
  'but WITHOUT ANY WARRANTY; without even the implied warranty of'
  'MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the'
  'GNU Affero General Public License for more details.'
  ''
  'You should have received a copy of the GNU Affero General Public License'
  'along with this program. If not, see <https://www.gnu.org/licenses/>.'
  ''
  'See the LICENSE file for the full license text.'
)

render_header() {
  local style="$1"
  local line

  case "$style" in
    hash)
      for line in "${notice_lines[@]}"; do
        if [[ -n "$line" ]]; then
          print -r -- "# $line"
        else
          print -r -- '#'
        fi
      done
      ;;
    slash)
      for line in "${notice_lines[@]}"; do
        if [[ -n "$line" ]]; then
          print -r -- "// $line"
        else
          print -r -- '//'
        fi
      done
      ;;
    block)
      print -r -- '/*'
      for line in "${notice_lines[@]}"; do
        if [[ -n "$line" ]]; then
          print -r -- " * $line"
        else
          print -r -- ' *'
        fi
      done
      print -r -- ' */'
      ;;
    xml)
      print -r -- '<!--'
      for line in "${notice_lines[@]}"; do
        if [[ -n "$line" ]]; then
          print -r -- "  $line"
        else
          print -r -- ' '
        fi
      done
      print -r -- '-->'
      ;;
    *)
      print -u2 -- "unsupported header style: $style"
      return 1
      ;;
  esac

  print
}

style_for() {
  case "$1" in
    *.sh|*.yml|*.yaml)
      print -r -- hash
      ;;
    *.swift|*.ts|*.mjs)
      print -r -- slash
      ;;
    *.css)
      print -r -- block
      ;;
    *.html|*.plist|*.entitlements)
      print -r -- xml
      ;;
    *)
      return 1
      ;;
  esac
}

has_header() {
  sed -n '1,24p' "$1" | grep -Fq -- "$copyright_marker"
}

prepend_file() {
  local file="$1"
  local style
  local first_line
  local second_line
  local prologue_lines=0
  local temp_file
  local mode

  if has_header "$file"; then
    print -r -- "skipped: $file"
    return
  fi

  style="$(style_for "$file")"
  first_line="$(sed -n '1p' "$file")"
  second_line="$(sed -n '2p' "$file")"

  case "$file" in
    *.sh)
      [[ "$first_line" == '#!'* ]] && prologue_lines=1
      ;;
    *.swift)
      [[ "$first_line" == '// swift-tools-version:'* ]] && prologue_lines=1
      ;;
    *.ts)
      [[ "$first_line" == '/// <reference '* ]] && prologue_lines=1
      ;;
    *.html)
      [[ "$first_line" == '<!doctype '* || "$first_line" == '<!DOCTYPE '* ]] && prologue_lines=1
      ;;
    *.plist|*.entitlements)
      if [[ "$first_line" == '<?xml '* ]]; then
        prologue_lines=1
        [[ "$second_line" == '<!doctype '* || "$second_line" == '<!DOCTYPE '* ]] && prologue_lines=2
      fi
      ;;
  esac

  temp_file="$(mktemp "${temp_dir}/header.XXXXXX")"
  {
    if (( prologue_lines > 0 )); then
      sed -n "1,${prologue_lines}p" "$file"
    fi

    render_header "$style"

    if (( prologue_lines > 0 )); then
      tail -n +$((prologue_lines + 1)) "$file"
    else
      cat < "$file"
    fi
  } > "$temp_file"

  mode="$(stat -f '%Lp' "$file")"
  chmod "$mode" "$temp_file"
  mv "$temp_file" "$file"
  print -r -- "updated: $file"
}

temp_dir="$(mktemp -d)"
cleanup() {
  [[ -d "$temp_dir" ]] && rm -rf "$temp_dir"
}
trap cleanup EXIT

while IFS= read -r -d '' file; do
  prepend_file "$file"
done < <(
  find . -type f \
    \( \
      -name '*.swift' -o \
      -name '*.ts' -o \
      -name '*.mjs' -o \
      -name '*.css' -o \
      -name '*.sh' -o \
      -name '*.plist' -o \
      -name '*.entitlements' \
    \) \
    ! -path './.git/*' \
    ! -path './.build/*' \
    ! -path './DerivedData/*' \
    ! -path './Renderer/node_modules/*' \
    ! -path './Renderer/third-party/*' \
    ! -path './Resources/Renderer/*' \
    -print0
)
