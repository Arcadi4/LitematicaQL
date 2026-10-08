#!/usr/bin/env bash
# Re-sign the ad-hoc archive produced by `just release`. Never build or run it.
set -euo pipefail

version="${1:-}"
version="${version#v}"
arch="${2:-}"
dry_run="${3:-false}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
   [[ "$arch" != arm64 && "$arch" != x86_64 ]] ||
   [[ "$dry_run" != true && "$dry_run" != false ]]; then
  echo "usage: sign-release.sh MAJOR.MINOR.PATCH arm64|x86_64 [true|false]" >&2
  exit 1
fi

: "${APPLE_TEAM_ID:?APPLE_TEAM_ID is required}"
if [[ ! "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]]; then
  echo "APPLE_TEAM_ID must be a ten-character Apple Team ID." >&2
  exit 1
fi
if [[ "$dry_run" == false ]]; then
  : "${APPLE_ID:?APPLE_ID is required for notarization}"
  : "${APPLE_APP_SPECIFIC_PASSWORD:?APPLE_APP_SPECIFIC_PASSWORD is required for notarization}"
fi

cd "$(dirname "$0")/.."
release_dir="$PWD/build/release"
filename="LitematicaQL-$version-$arch.zip"
candidate="$release_dir/$filename"
test -f "$candidate"
work_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/litematicaql-sign.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

# Downloaded artifacts belong to this workflow run. Their checksum is checked
# again before extraction, and the original ZIP is replaced only on success.
(cd "$release_dir" && shasum -a 256 -c "$filename.sha256")
ditto -x -k "$candidate" "$work_dir"
app="$work_dir/LitematicaQL.app"
extension="$app/Contents/PlugIns/LitematicaQLPreview.appex"

check_bundle() {
  local bundle="$1" expected_id="$2" executable="$3"
  test -d "$bundle"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Contents/Info.plist")" = "$expected_id"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$bundle/Contents/Info.plist")" = "$version"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$bundle/Contents/Info.plist")" = "$executable"
  test "$(lipo -archs "$bundle/Contents/MacOS/$executable")" = "$arch"
}
check_bundle "$app" moe.arcadia.LitematicaQL LitematicaQL
check_bundle "$extension" moe.arcadia.LitematicaQL.PreviewExtension LitematicaQLPreview
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$app/Contents/Info.plist")" = \
  "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$extension/Contents/Info.plist")"

# Select exactly one Developer ID Application identity for the intended team.
# A development certificate or an identity for another team must not be used.
identities=$(security find-identity -v -p codesigning | \
  awk -v team="($APPLE_TEAM_ID)" '/"Developer ID Application:/ && index($0, team) { print $2 }')
if [[ -z "$identities" || "$(printf '%s\n' "$identities" | wc -l | tr -d ' ')" != 1 ]]; then
  echo "Expected exactly one valid Developer ID Application identity for APPLE_TEAM_ID." >&2
  exit 1
fi

# The Rust bridge is statically linked into these two executables. If new
# nested code is added, extend this explicit signing order before releasing.
codesign --force --sign "$identities" --options runtime --timestamp \
  --entitlements PreviewExtension/LitematicaQLPreview.entitlements "$extension"
codesign --force --sign "$identities" --options runtime --timestamp \
  --entitlements App/LitematicaQL.entitlements "$app"
for bundle in "$extension" "$app"; do
  codesign --verify --deep --strict --verbose=2 "$bundle"
  details=$(codesign --display --verbose=4 "$bundle" 2>&1)
  grep -q "^TeamIdentifier=$APPLE_TEAM_ID$" <<<"$details"
  grep -q '^Authority=Developer ID Application:' <<<"$details"
done

if [[ "$dry_run" == false ]]; then
  # Apple credentials are supplied through environment variables, not shell
  # interpolation in YAML. --wait alone is insufficient: require Accepted.
  submission="$work_dir/submission.zip"
  ditto -c -k --keepParent "$app" "$submission"
  xcrun notarytool submit "$submission" \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_SPECIFIC_PASSWORD" --wait --output-format json \
    > "$work_dir/notary-result.json"
  cat "$work_dir/notary-result.json"
  if [[ "$(plutil -extract status raw -o - "$work_dir/notary-result.json")" != Accepted ]]; then
    echo "Notarization was not accepted; release files will not be replaced." >&2
    exit 1
  fi
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  codesign --verify --deep --strict --verbose=2 "$app"
  spctl --assess --type execute --verbose=4 "$app"
else
  echo "Dry run: signed only; not notarized, stapled, or approved by Gatekeeper."
fi

# Staple the app, then zip it again: ZIPs cannot themselves be stapled.
ditto -c -k --keepParent "$app" "$work_dir/$filename"
(cd "$work_dir" && shasum -a 256 "$filename" > "$filename.sha256")
mv "$work_dir/$filename" "$candidate"
mv "$work_dir/$filename.sha256" "$candidate.sha256"
echo "packaged signed $candidate"
