#!/usr/bin/env bash
# Refuse a release that is already published.
#
# Usage: release-guard.sh VERSION
#
# Preconditions: GH_TOKEN can read the repository releases.
set -euo pipefail

version="${1:-}"
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-guard.sh vMAJOR.MINOR.PATCH" >&2
  exit 1
fi
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

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
