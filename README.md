<div align="center">
  <img src="icon.png" alt="LitematicaQL icon" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>Quick Look preview for Minecraft schematics on macOS</strong></p>
  <!-- README-I18N:START -->

**English** | [中文](./README.zh.md)

  <!-- README-I18N:END -->
</div>

LitematicaQL adds a macOS Quick Look preview extension for Minecraft schematics and structures. It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.nusn`, and `.mca`.

If you are on Windows, check out the sister project [LitematicaPreview](https://github.com/Arcadi4/LitematicaPreview) for rendering speed as fast as LitematicaQL!

<table>
<tr>
<th>Previewing Schematics</th>
<th>Previewing World Saves</th>
</tr>
<tr>
<td>https://github.com/user-attachments/assets/3e70e28a-bba3-42e2-b3ca-256369c46f4c</td>
<td>https://github.com/user-attachments/assets/d8b034f7-44b6-4940-8fc5-be5c5af25cb4</td>
</tr>
</table>

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

This app requires macOS 13 Ventura or later and a Metal-capable GPU.

Previews use native Metal with streamed chunk meshing, compact GPU buffers,
and on-demand drawing. See [the rendering architecture](docs/rendering.md).

> [!NOTE]
> If the preview doesn't show up, enable LitematicaQL under **System Settings → General → Login Items & Extensions → Quick Look**.

## Supported Formats

| Extension      | File format                                     |
| -------------- | ----------------------------------------------- |
| `.litematic`   | Litematica                                      |
| `.schem`       | Sponge schematic                                |
| `.schematic`   | MCEdit                                          |
| `.nbt`         | Java structure block                            |
| `.snbt`        | Structure SNBT, brace or bracket block states   |
| `.mcstructure` | Bedrock structure                               |
| `.nusn`        | Nucleation snapshot                             |
| `.mca`         | Minecraft Anvil world saves (4 chunks per file) |

## Development

Everything is driven by [just](https://github.com/casey/just). Run it with no
arguments to list every recipe:

```sh
just
```

| Command | What it does |
| --- | --- |
| `just doctor` | Check that the toolchain prerequisites are installed |
| `just build` | Build the ad-hoc signed app for Apple silicon and Intel |
| `just run` | Build the app and open it |
| `just test` | Run the Swift file-validation tests and the Rust bridge tests |
| `just check` | Type-check the Rust bridge |
| `just clean` | Delete every generated build artifact |
| `just ci` | Reproduce the CI pipeline locally |
| `just unregister` | Clean stale LitematicaQL Quick Look registrations, keeping the installed `/Applications` copy |
| `just bump patch` | Bump the version on `main` (`patch` by default, or `minor`, `major`), commit it, and tag it |
| `just release 1.2.6 arm64` | Archive, verify, and package one architecture |
| `just demos` | Regenerate the bundled demo schematics |

`just` honors `CONFIGURATION`, `ARCHS`, and `CURRENT_PROJECT_VERSION` from the
environment, which is how CI and the release matrix drive these same recipes.

### Build

Prerequisites:

- [just](https://github.com/casey/just)
- Xcode 26 or later, including the Metal Toolchain component
- Rust (rustup)
- [XcodeGen](https://github.com/yonaskolb/XcodeGen)

```sh
git clone https://github.com/Arcadi4/LitematicaQL.git
cd LitematicaQL

just build
```

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.

Thanks to my generous friend [@johnbean393](https://github.com/johnbean393) for sharing their Apple signature to notarize the app, providing better unboxing experience while saving me $100 in Apple tax! Consider looking into their project [Chiboard](https://www.chiboard.app), an ML-based Pinyin input method that speeds up Chinese typing manifold.
