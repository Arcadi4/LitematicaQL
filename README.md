<div align=center>
<img src=icon.png alt=icon width=200px/>
<h1 align="center">LitematicaQL</h1>
<h4>
  Quick Look preview for Litematica schematics on macOS.
</h4>
</div>

LitematicaQL adds a Quick Look preview extension for `.litematic` files. Select a schematic in Finder, press Space, then orbit, zoom, and pan around the rendered blocks without opening Minecraft or Litematica.

## Features

- Native macOS app with an embedded Quick Look preview extension
- Textured Three.js/WebGL rendering through [`schematic-renderer`](https://www.npmjs.com/package/schematic-renderer)
- Interactive orbit, zoom, and pan controls

## Requirements

- macOS 13 Ventura or newer
- Xcode 26 or newer
- Node.js and pnpm
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) 2.46 or newer

Install the supporting command-line tools with Homebrew:

```sh
brew install xcodegen
```

## Build

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

## Install and test

1. Move `LitematicaQL.app` to `/Applications`.
2. Open the app once so macOS registers its preview extension.
3. Select a `.litematic` file in Finder and press Space.

You can test with [`Fixtures/LitematicaQL-Demo.litematic`](Fixtures/LitematicaQL-Demo.litematic), or click **View Example** in the app.

If the extension was disabled manually, enable LitematicaQL under **System Settings → General → Login Items & Extensions → Quick Look**.

## Development

Run all renderer checks and rebuild its distributable assets:

```sh
pnpm --prefix Renderer run check
```

Run the native file-validation tests:

```sh
swift test
```

## Layout

| Path | Purpose |
| --- | --- |
| `App/` | SwiftUI host app and document integration |
| `PreviewExtension/` | Quick Look extension entry point |
| `Shared/` | File validation and shared WKWebView controller |
| `Renderer/` | Vite+/TypeScript renderer source and tests |
| `Resources/Renderer/` | Generated self-contained renderer bundle |
| `Fixtures/` | Testable `.litematic` fixture |
| `Tests/` | Swift package tests |
