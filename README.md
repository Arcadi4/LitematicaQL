<div align="center">
  <img src="icon.png" alt="LitematicaQL icon" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>Quick Look preview for Minecraft schematics on macOS</strong></p>
  <!-- README-I18N:START -->

  **English** | [中文](./README.zh.md)

  <!-- README-I18N:END -->
</div>

LitematicaQL adds a macOS Quick Look preview extension for Minecraft schematics and structures. It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn`.

<div align=center>

<https://github.com/user-attachments/assets/3e70e28a-bba3-42e2-b3ca-256369c46f4c>

</div>

## Install

Install with Homebrew:

```sh
brew install --cask arcadi4/tap/litematicaql
```

Or install manually:

1. Download from the [release page](https://github.com/Arcadi4/LitematicaQL/releases)
2. Move `LitematicaQL.app` to `/Applications`.
3. Open the app once so macOS registers its preview extension.
4. Select a supported file in Finder and press Space.

This app requires macOS 13 Ventura or later.

> [!NOTE]
> If the preview doesn't show up, enable LitematicaQL under **System Settings → General → Login Items & Extensions → Quick Look**.

## Supported Formats

| Extension | File format |
| --- | --- |
| `.litematic` | Litematica |
| `.schem` | Sponge schematic |
| `.schematic` | MCEdit |
| `.nbt` | Java structure block |
| `.snbt` | Structure SNBT, brace or bracket block states |
| `.mcstructure` | Bedrock structure |
| `.nusn` | Nucleation snapshot |

## Development

Run all renderer checks and rebuild the artifact:

```sh
pnpm --prefix Renderer run check
```

Run the Swift tests:

```sh
swift test
```

Clean stale LitematicaQL Quick Look registrations while keeping the installed `/Applications` copy with:

```sh
./scripts/clean-quick-look-registrations.sh
```

### Build

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

## Acknowledgements

Great thanks to @Nano112 's project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.
