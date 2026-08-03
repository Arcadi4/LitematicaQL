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

https://github.com/user-attachments/assets/18cfaf53-2068-4995-adcd-4de2e11b8f07

</div>

## Features

- Native macOS app with an embedded Quick Look preview extension
- Textured Three.js/WebGL rendering via [`schematic-renderer`](https://www.npmjs.com/package/schematic-renderer)
- Interactive orbit, zoom, and pan controls

## Requirements

- macOS 13 Ventura or later

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

## Install

1. Move `LitematicaQL.app` to `/Applications`.
2. Open the app once so macOS registers its preview extension.
3. Select a `.litematic` file in Finder and press Space.

Try it with [`Fixtures/LitematicaQL-Demo.litematic`](Fixtures/LitematicaQL-Demo.litematic), or click **View Example** in the app.

> [!NOTE]
> If the preview doesn't show up, enable LitematicaQL under **System Settings → General → Login Items & Extensions → Quick Look**.

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
