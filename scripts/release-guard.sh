#!/usr/bin/env bash
# Refuse a release that is published, or whose build number does not increase.
#
# Usage: release-guard.sh VERSION [FEED_URL]
#
# Preconditions: GH_TOKEN can read the repository releases. GITHUB_RUN_NUMBER is
# the build number of this run.
set -euo pipefail

version="${1:-}"
feed_url="${2:-https://update.arcadia.moe/litematicaql/arm64/appcast.xml}"
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-guard.sh vMAJOR.MINOR.PATCH [FEED_URL]" >&2
  exit 1
fi
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_RUN_NUMBER:?GITHUB_RUN_NUMBER is required}"

source "$(dirname "$0")/release-http.sh"

# version_gt LEFT RIGHT
#
# Returns success when LEFT is greater than RIGHT. Both are dotted triples.
version_gt() {
  local -a left right
  IFS=. read -r -a left <<< "${1#v}"
  IFS=. read -r -a right <<< "${2#v}"
  local index
  for index in 0 1 2; do
    if (( left[index] != right[index] )); then
      (( left[index] > right[index] ))
      return
    fi
  done
  return 1
}

while IFS= read -r tag; do
  if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    continue
  fi
  if [[ "$tag" == "$version" ]] || version_gt "$tag" "$version"; then
    echo "Release $tag is published and is not older than $version." >&2
    exit 1
  fi
done < <(gh release list --repo "$GITHUB_REPOSITORY" \
  --exclude-drafts --exclude-pre-releases --limit 1000 \
  --json tagName --jq '.[].tagName')

work_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/litematicaql-guard.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

# The run identifier keeps the CDN from answering the check with an old feed.
feed="$feed_url?run=${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}"
status=$(fetch_public "$feed" "$work_dir/appcast.xml")
reject_unavailable "$feed" "$status"

item() {
  xmllint --xpath \
    "string((//*[local-name()=\"item\"])[$1]/*[local-name()=\"$2\"])" \
    "$work_dir/appcast.xml"
}

items=$(xmllint --xpath 'count(//*[local-name()="item"])' "$work_dir/appcast.xml")
index=1
while (( index <= items )); do
  published_version=$(item "$index" shortVersionString)
  if [[ ! "$published_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "The published appcast has an invalid version: ${published_version:-none}." >&2
    exit 1
  fi
  if version_gt "$published_version" "$version"; then
    echo "The published appcast version $published_version is newer than $version." >&2
    exit 1
  fi
  published_build=$(item "$index" version)
  if [[ ! "$published_build" =~ ^[0-9]+$ ]]; then
    echo "The published appcast has an invalid build number: ${published_build:-none}." >&2
    exit 1
  fi
  if (( published_build >= GITHUB_RUN_NUMBER )); then
    echo "Build number $GITHUB_RUN_NUMBER must exceed the published build number $published_build." >&2
    exit 1
  fi
  index=$((index + 1))
done
