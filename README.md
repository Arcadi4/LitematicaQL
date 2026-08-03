<div align="center">
  <img src="icon.png" alt="LitematicaQL icon" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>Quick Look preview for Litematica schematics on macOS</strong></p>
  <!-- README-I18N:START -->

  **English** | [中文](./README.zh.md)

  <!-- README-I18N:END -->
</div>

LitematicaQL adds a Quick Look preview extension for `.litematic` files. Select a schematic in Finder, press Space, then orbit, zoom, and pan around the rendered blocks — no Minecraft or Litematica required.

<div align=center>

<https://github.com/user-attachments/assets/18cfaf53-2068-4995-adcd-4de2e11b8f07>

</div>

## Install

1. Download from the [release page](https://github.com/Arcadi4/LitematicaQL/releases)
2. Move `LitematicaQL.app` to `/Applications`.
3. Open the app once so macOS registers its preview extension.
4. Select a `.litematic` file in Finder and press Space.

This app requires macOS 13 Ventura or later.

> [!NOTE]
> If the preview doesn't show up, enable LitematicaQL under **System Settings → General → Login Items & Extensions → Quick Look**.

## Build

- Xcode 26 or later
- Node.js and pnpm
- [XcodeGen](https://github.com/yonaskolb/XcodeGen)

```sh
git clone https://github.com/Arcadi4/LitematicaQL.git
cd LitematicaQL

pnpm --prefix Renderer ci
pnpm --prefix Renderer run build
xcodegen generate

xcodebuild \
  -project LitematicaQL.xcodeproj \
  -scheme LitematicaQL \
  -configuration Debug \
  -derivedDataPath DerivedData \
  build
```

## Development

Run all renderer checks and rebuild its distributable assets:

```sh
pnpm --prefix Renderer run check
```

Run the native file-validation tests:

```sh
swift test
```

Clean stale LitematicaQL Quick Look registrations while keeping the installed
`/Applications` copy with:

```sh
./scripts/clean-quick-look-registrations.sh
```

## Project structure

| Path | Purpose |
| --- | --- |
| `App/` | SwiftUI host app and document integration |
| `PreviewExtension/` | Quick Look extension entry point |
| `Shared/` | File validation and shared WKWebView controller |
| `Renderer/` | Vite+/TypeScript renderer source and tests |
| `Resources/Renderer/` | Generated self-contained renderer bundle |
| `Fixtures/` | Sample `.litematic` fixture |
| `Tests/` | Swift package tests |
