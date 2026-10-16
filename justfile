# LitematicaQL toolchain.
#
# `just` is the only command to type. Run it with no arguments for the recipe
# list. These recipes invoke Xcode, SwiftPM, cargo, XcodeGen, codesign and the
# demo generator. A laptop build, a CI run and a release archive therefore run
# the same commands.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Names

app_name := "LitematicaQL"
project_file := app_name + ".xcodeproj"
scheme := app_name
preview_extension_id := "moe.arcadia.LitematicaQL.PreviewExtension"

# Output directories
#
# Everything generated goes under build/, so `just clean` is one removal.
# The XcodeGen project is the only tracked build input.

build_dir := justfile_directory() / "build"
derived_data := build_dir / "DerivedData"
release_dir := build_dir / "release"
app_bundle := derived_data / "Build" / "Products" / configuration / (app_name + ".app")

# Overridable build settings
#
# Read from the environment, so CI and the release matrix set these values
# for the same recipes instead of repeating xcodebuild invocations.

configuration := env_var_or_default("CONFIGURATION", "Debug")
build_archs := env_var_or_default("ARCHS", "arm64 x86_64")
project_version := env_var_or_default("CURRENT_PROJECT_VERSION", "1")

# DEVELOPMENT_TEAM is empty, so every build signs ad-hoc with "-". No machine
# identity is needed, and an unsigned checkout builds.
signing := "CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY=- CODE_SIGNING_REQUIRED=YES CODE_SIGNING_ALLOWED=YES DEVELOPMENT_TEAM="

# Xcode refuses ad-hoc signatures for entitlements with the app-group
# capability. xcodebuild therefore embeds a stripped copy, and codesign
# re-signs with the real entitlements after the build. The order is extension
# first, then the app, the same order sign-release.sh uses.
adhoc_entitlements := build_dir / "adhoc" / "entitlements.plist"
codesign_entitlements := "CODE_SIGN_ENTITLEMENTS=" + adhoc_entitlements

# Recipes
#
# A `[script]` recipe runs as a plain file, so `set shell` does not apply to
# it. Every one of these bodies turns on `set -euo pipefail` itself.
#
# `just --list` prints the last line of each recipe's doc comment. Every doc
# comment is therefore one line, and the longer notes sit above it as
# ordinary comments.

# List every recipe. A bare `just` runs this one.
default:
    @just --list

# Build the ad-hoc signed app into build/DerivedData.
build: generate native prepare-adhoc-entitlements
    xcodebuild \
      -project {{ project_file }} \
      -scheme {{ scheme }} \
      -configuration {{ configuration }} \
      -destination "generic/platform=macOS" \
      -derivedDataPath {{ derived_data }} \
      {{ signing }} \
      {{ codesign_entitlements }} \
      ARCHS="{{ build_archs }}" \
      ONLY_ACTIVE_ARCH=NO \
      build
    codesign --force --sign - \
      --entitlements PreviewExtension/LitematicaQLPreview.entitlements \
      "{{ app_bundle }}/Contents/PlugIns/LitematicaQLPreview.appex"
    codesign --force --sign - \
      --entitlements App/LitematicaQL.entitlements \
      "{{ app_bundle }}"

# Write the entitlements copy that xcodebuild embeds. The codesign step in
# `build` applies the real files. PlistBuddy deletes the app-group key from
# the copy only. The tracked source keeps the real entitlements.

# Write the app-group-free entitlements copy for xcodebuild.
prepare-adhoc-entitlements:
    mkdir -p {{ build_dir }}/adhoc
    cp App/LitematicaQL.entitlements {{ adhoc_entitlements }}
    /usr/libexec/PlistBuddy -c 'Delete :com.apple.security.application-groups' \
      {{ adhoc_entitlements }} || true

# Build the app and open it.
run: build
    open {{ app_bundle }}

# Run every test suite: Swift file validation and the Rust bridge.
[parallel]
test: swift-test _native-test
    @echo "all test suites passed"

# Run opt-in Metal readback and borrowed-batch lifetime tests.
test-metal: native
    mkdir -p {{ build_dir }}/metal-tests
    swiftc -O -I Core/include Shared/NativeSchematic.swift Shared/MetalMesh.swift Tests/MetalTests/main.swift "Core/build/liblitematicaql_native_$(uname -m).a" -framework Metal -framework Foundation -o {{ build_dir }}/metal-tests/test-metal
    {{ build_dir }}/metal-tests/test-metal

# Keep the cargo invocations sequential. They share one target directory, and
# concurrent runs compete for its lock. SwiftPM outputs are independent, so
# the SwiftPM test can run in parallel.
_native-test: native rust-test

# Run the Swift file-validation tests.
swift-test:
    swift test --parallel

# The tests build for the host triple on purpose. `native` builds the same
# triple into the same target directory, so both recipes use one release
# build of the dependency tree. If the two invocations resolve
# different toolchains, cargo rebuilds and does not reuse the artifacts.

# Run the Rust bridge tests.
rust-test:
    ./Core/cargo.sh test --release --target "$(rustc -vV | sed -n 's/^host: //p')" --manifest-path Core/Cargo.toml

# Report throughput and peak memory for the two third-party large builds.
test-large:
    ./Core/cargo.sh test --release --target "$(rustc -vV | sed -n 's/^host: //p')" --manifest-path Core/Cargo.toml --test large_builds -- --nocapture

# Type-check the Rust bridge, tests included.
check:
    ./Core/cargo.sh check --manifest-path Core/Cargo.toml --tests --locked


# Resolve the Rust dependency lockfile after a dependency change.
resolve:
    ./Core/cargo.sh update --manifest-path Core/Cargo.toml -p litematicaql-native

# Format the Rust bridge and its tests.
format-rust:
    ./Core/cargo.sh fmt --manifest-path Core/Cargo.toml

# Reproduce the CI pipeline locally: every test suite, then the app build.
ci: test build
    codesign --verify --deep --strict {{ app_bundle }}
    @echo "codesign verified {{ app_bundle }}"

# Regenerate LitematicaQL.xcodeproj from project.yml.
generate:
    xcodegen generate

# The Xcode pre-build phase runs Core/build.sh, so a project opened in Xcode
# also compiles the bridge. This recipe runs the same script on the command
# line and takes its architectures from $ARCHS.

# Build the Rust static libraries into Core/build/.
native:
    ./Core/build.sh {{ build_archs }}

# Archives one architecture, because the release matrix builds one
# architecture per job. Verifies the bundle and packages it. Produces
# build/release/LitematicaQL-VERSION-ARCH.zip with a .sha256 sidecar.
# CURRENT_PROJECT_VERSION comes from $CURRENT_PROJECT_VERSION.

# Archive, verify, and package a release build.
[script("bash")]
release version arch="arm64": generate prepare-adhoc-entitlements
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
      {{ codesign_entitlements }} \
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

# Sign a packaged candidate. dry_run=true skips notarization for testing.
sign-release version arch dry_run="false":
    ./scripts/sign-release.sh "{{ version }}" "{{ arch }}" "{{ dry_run }}"

# Tags a release: bump the version, commit, and tag.
#
# The argument is the component to raise. patch is the default, and minor and
# major are also valid. The recipe runs only on main from a clean workspace,
# so the tag names exactly the reviewed code. Push the commit and the tag
# yourself. The release workflow starts on the tag.

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

    # The workspace was clean on entry, so only project.yml and the project
    # file that XcodeGen regenerates from it can differ. Stop before the
    # commit if anything else changed.
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

# macOS keeps every registered copy of the appex, and Quick Look can use an
# old DerivedData build. Set LITEMATICAQL_KEEP_EXTENSION_PATH to keep a copy
# other than the installed app.

# Unregister stale Quick Look preview extensions. The installed app stays registered.
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
# whose encoding drifts fails the run. This tooling is optional and is not
# part of the app build.
#
# The install uses --ignore-scripts, so no dependency postinstall runs. pnpm
# blocks the esbuild postinstall until the user approves it, and it records
# the approval as a pnpm-workspace.yaml stub that the next install
# overwrites. tsx resolves the esbuild binary from its platform package at
# runtime, so the script is not needed.

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

# This check is last so the recipes appear in the order a contributor uses
# them.

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
