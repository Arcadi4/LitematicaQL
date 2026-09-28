# LitematicaQL toolchain.
#
# `just` is the only command to type. Run it with no arguments for the recipe
# list. Xcode, SwiftPM, cargo, XcodeGen, codesign and the demo generator are
# invoked from here and nowhere else, so a laptop build, a CI run and a release
# archive execute the same commands in the same order.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Names

app_name := "LitematicaQL"
project_file := app_name + ".xcodeproj"
scheme := app_name
preview_extension_id := "moe.arcadia.LitematicaQL.PreviewExtension"

# Output directories
#
# Everything generated lands under build/ so `just clean` is a single removal.
# The XcodeGen project stays the only tracked build input.

build_dir := justfile_directory() / "build"
derived_data := build_dir / "DerivedData"
release_dir := build_dir / "release"
app_bundle := derived_data / "Build" / "Products" / configuration / (app_name + ".app")

# Overridable build settings
#
# Read from the environment so the CI workflow and the release matrix drive
# these same recipes instead of retyping xcodebuild invocations.

configuration := env_var_or_default("CONFIGURATION", "Debug")
build_archs := env_var_or_default("ARCHS", "arm64 x86_64")
project_version := env_var_or_default("CURRENT_PROJECT_VERSION", "1")

# The project carries an empty DEVELOPMENT_TEAM and ships ad-hoc signed, so
# every invocation signs with "-" rather than depending on a machine's
# identities. This is what makes an unsigned checkout buildable at all.
signing := "CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY=- CODE_SIGNING_REQUIRED=YES CODE_SIGNING_ALLOWED=YES DEVELOPMENT_TEAM="

# Recipes
#
# A `[script]` recipe runs as a plain file, so `set shell` does not apply to
# it and every one of these bodies turns on `set -euo pipefail` itself. A
# recipe that keeps going after a failed xcodebuild is how a release job once
# published a tag with no builds attached.
#
# `just --list` prints the last line of each recipe's doc comment, so every
# doc comment is exactly one line and the longer notes sit above it as
# ordinary comments.

# List every recipe; this is what a bare `just` runs.
default:
    @just --list

# Build the ad-hoc signed app into build/DerivedData.
build: generate native
    xcodebuild \
      -project {{ project_file }} \
      -scheme {{ scheme }} \
      -configuration {{ configuration }} \
      -destination "generic/platform=macOS" \
      -derivedDataPath {{ derived_data }} \
      {{ signing }} \
      ARCHS="{{ build_archs }}" \
      ONLY_ACTIVE_ARCH=NO \
      build

# Build the app and open it.
run: build
    open {{ app_bundle }}

# Run every test suite: Swift file validation and the Rust bridge.
[parallel]
test: swift-test _native-test
    @echo "all test suites passed"

# Keep Cargo invocations sequential so they reuse one target directory without
# competing for its lock. SwiftPM has independent outputs and can overlap them.
_native-test: native rust-test

# Run the Swift file-validation tests.
swift-test:
    swift test --parallel

# Run the Rust bridge tests.
rust-test:
    cargo test --release --manifest-path Core/Cargo.toml

# Report throughput and peak memory for the two third-party large builds.
test-large:
    cargo test --release --manifest-path Core/Cargo.toml --test large_builds -- --nocapture

# Type-check the Rust bridge, tests included.
check:
    cargo check --manifest-path Core/Cargo.toml --tests --locked

# Reproduce the CI pipeline locally: every test suite, then the app build.
ci: test build
    codesign --verify --deep --strict {{ app_bundle }}
    @echo "codesign verified {{ app_bundle }}"

# Regenerate LitematicaQL.xcodeproj from project.yml.
generate:
    xcodegen generate

# Xcode's pre-build phase runs Core/build.sh itself so a project opened in
# Xcode still compiles the bridge; this recipe is the same script for the
# command line, and takes its architectures from $ARCHS.

# Build the Rust static libraries into Core/build/.
native:
    ./Core/build.sh {{ build_archs }}

# Archives a single architecture, the way the release matrix ships, then
# verifies the bundle and packages it. Produces
# build/release/LitematicaQL-VERSION-ARCH.zip with a .sha256 sidecar.
# CURRENT_PROJECT_VERSION comes from $CURRENT_PROJECT_VERSION.

# Archive, verify, and package a release build.
[script("bash")]
release version arch="arm64": generate
    set -euo pipefail

    version="{{ version }}"
    version="${version#v}"
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      echo "usage: just release MAJOR.MINOR.PATCH [ARCH]" >&2
      exit 1
    fi
    archive_path="{{ build_dir }}/{{ app_name }}-{{ arch }}.xcarchive"
    app_path="$archive_path/Products/Applications/{{ app_name }}.app"

    rm -rf "$archive_path"

    xcodebuild \
      -project {{ project_file }} \
      -scheme {{ scheme }} \
      -configuration Release \
      -destination "generic/platform=macOS" \
      -archivePath "$archive_path" \
      MARKETING_VERSION="$version" \
      CURRENT_PROJECT_VERSION="{{ project_version }}" \
      {{ signing }} \
      ARCHS="{{ arch }}" \
      ONLY_ACTIVE_ARCH=NO \
      archive

    test -d "$app_path"

    actual_version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app_path/Contents/Info.plist")
    test "$actual_version" = "$version"

    codesign --verify --deep --strict --verbose=2 "$app_path"
    lipo -info "$app_path/Contents/MacOS/{{ app_name }}" | grep -q "{{ arch }}"

    mkdir -p {{ release_dir }}
    zip_path="{{ release_dir }}/{{ app_name }}-$version-{{ arch }}.zip"
    ditto -c -k --keepParent "$app_path" "$zip_path"
    (cd {{ release_dir }} && shasum -a 256 "$(basename "$zip_path")" > "$(basename "$zip_path").sha256")

    echo "packaged $zip_path"

# Tags a release: bump the version, commit, and tag.
#
# Takes the component to raise, so the current version never has to be looked
# up: a bare `just bump` and `just bump patch` both give 1.2.6 from 1.2.5,
# `just bump minor` gives 1.3.0, and `just bump major` gives 2.0.0. Runs only
# on main from a clean workspace, so the tag always names exactly what was
# reviewed. Push the commit and the tag yourself; the release workflow fires
# on the tag.

# Bump major, minor, or patch, then commit and tag it.
[script("bash")]
bump level="patch":
    set -euo pipefail

    branch=$(git branch --show-current)
    if [[ "$branch" != "main" ]]; then
      echo "bump: versions are cut from main, not $branch" >&2
      exit 1
    fi

    if [[ -n "$(git status --porcelain)" ]]; then
      echo "bump: workspace has uncommitted changes:" >&2
      git status --short >&2
      exit 1
    fi

    current=$(sed -nE 's/^[[:space:]]*MARKETING_VERSION: ([0-9]+\.[0-9]+\.[0-9]+)$/\1/p' project.yml)
    if [[ -z "$current" ]]; then
      echo "bump: no MARKETING_VERSION found in project.yml" >&2
      exit 1
    fi

    case "{{ level }}" in
      major) next=$(awk -F. '{ printf "%d.0.0", $1 + 1 }' <<<"$current") ;;
      minor) next=$(awk -F. '{ printf "%d.%d.0", $1, $2 + 1 }' <<<"$current") ;;
      patch) next=$(awk -F. '{ printf "%d.%d.%d", $1, $2, $3 + 1 }' <<<"$current") ;;
      *)
        echo "usage: just bump major|minor|patch" >&2
        exit 1
        ;;
    esac

    if git rev-parse -q --verify "refs/tags/v$next" >/dev/null; then
      echo "bump: tag v$next already exists" >&2
      exit 1
    fi

    sed -i.bak -E "s/^([[:space:]]*)MARKETING_VERSION: .*/\1MARKETING_VERSION: $next/" project.yml
    rm -f project.yml.bak
    xcodegen generate
    grep -q "MARKETING_VERSION = $next;" "{{ project_file }}/project.pbxproj"

    # The workspace was clean on entry, so only the version and the project
    # XcodeGen regenerates from it may differ. Anything else is a surprise a
    # release should not carry, so stop before committing it.
    changed=$(git diff --name-only | LC_ALL=C sort)
    expected=$(printf '%s\n%s' "{{ project_file }}/project.pbxproj" project.yml | LC_ALL=C sort)
    if [[ "$changed" != "$expected" ]]; then
      echo "bump: refusing to commit changes beyond the version bump:" >&2
      git status --short >&2
      exit 1
    fi

    git add project.yml "{{ project_file }}/project.pbxproj"
    git commit -m "chore: bump to v$next"
    git tag -a "v$next" -m "v$next"

    echo "bumped $current to $next, committed and tagged v$next"
    echo "publish with: git push origin main v$next"

# macOS keeps every copy of the appex it has ever seen, and Quick Look may
# answer from an old DerivedData build. Set LITEMATICAQL_KEEP_EXTENSION_PATH to
# protect a copy other than the one in /Applications.

# Unregister stale Quick Look preview extensions, keeping the installed app.
[script("bash")]
unregister:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v pluginkit >/dev/null 2>&1; then
      echo "pluginkit is required to clean Quick Look registrations." >&2
      exit 1
    fi

    keep="${LITEMATICAQL_KEEP_EXTENSION_PATH:-/Applications/{{ app_name }}.app/Contents/PlugIns/LitematicaQLPreview.appex}"

    registered=$(pluginkit -m -D -v -p com.apple.quicklook.preview -i {{ preview_extension_id }})

    while IFS= read -r extension_path; do
      if [[ -z "$extension_path" || "$extension_path" == "$keep" ]]; then
        continue
      fi
      pluginkit -r "$extension_path"
      echo "unregistered: $extension_path"
    done < <(
      printf '%s\n' "$registered" \
        | awk -F $'\t' 'NF >= 4 && $NF != "" { print $NF }' \
        | sort -u \
        | grep -Fvx "$keep" \
        || true
    )

# The generator refuses to write a file it cannot read back, so a file format
# whose encoding drifts fails the run instead of shipping. This is optional
# authoring tooling, not part of the app build.
#
# Install with --ignore-scripts so no dependency postinstall runs: pnpm blocks
# esbuild's until the approval is recorded, and records it by writing a
# pnpm-workspace.yaml stub that the next install overwrites again. tsx resolves
# the esbuild binary from its platform package at runtime, so the script is not
# needed.

# Regenerate the bundled demo schematics in Fixtures/Demos.
demos:
    pnpm --dir scripts/demos install --frozen-lockfile --ignore-scripts
    pnpm --dir scripts/demos run generate

# Removes build/, the SwiftPM and cargo caches, and the compiled Rust
# libraries. The XcodeGen project and every tracked source stay in place.

# Delete every generated build artifact.
clean:
    rm -rf \
      {{ build_dir }} \
      {{ justfile_directory() }}/Core/build \
      {{ justfile_directory() }}/Core/target \
      {{ justfile_directory() }}/.build
    echo "removed build outputs; run `just build` to recreate them"

# The prerequisite check lives at the end of the file so the recipes read in
# the order a contributor uses them.

# Check that the toolchain prerequisites are installed.
[script("bash")]
doctor:
    #!/usr/bin/env bash
    set -euo pipefail

    for tool in cargo rustup xcodegen xcodebuild; do
      if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 1
      fi
    done

    xcodebuild -version
    rustc --version
    xcodegen --version
