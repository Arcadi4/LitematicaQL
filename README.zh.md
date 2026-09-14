<div align="center">
  <img src="icon.png" alt="LitematicaQL 图标" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>在 macOS 上快速预览 Minecraft 原理图</strong></p>
  <!-- README-I18N:START -->

  [English](./README.md) | **中文**

  <!-- README-I18N:END -->
</div>

LitematicaQL 为 Minecraft 原理图与结构提供了 macOS 快速查看预览扩展，支持 `.litematic`、`.schem`、`.schematic`、`.nbt`、`.snbt`、`.mcstructure` 与 `.nusn` 格式。

<div align=center>

<https://github.com/user-attachments/assets/18cfaf53-2068-4995-adcd-4de2e11b8f07>

</div>

## 安装

使用 Homebrew：

```sh
brew install --cask arcadi4/tap/litematicaql
```

或者手动安装：

1. 从[发布页面](https://github.com/Arcadi4/LitematicaQL/releases)下载
2. 将 `LitematicaQL.app` 移动到 `/Applications`。
3. 打开一次应用，让 macOS 注册其预览扩展。
4. 在访达中选中受支持的文件，按下空格键。

需要 macOS 13 Ventura 或更高版本。

> [!NOTE]
> 如果预览没有出现，请在 **系统设置 → 通用 → 登录项与扩展 → 快速查看** 中启用 LitematicaQL。

## 开发

运行渲染器的全部检查并重新构建产物：

```sh
pnpm --prefix Renderer run check
```

运行 Swift 测试：

```sh
swift test
```

清理 LitematicaQL 的旧 Quick Look 注册记录，同时保留已安装的 `/Applications` 版本：

```sh
./scripts/clean-quick-look-registrations.sh
```

### 构建

- Xcode 26 或更高
- Node.js 和 pnpm
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

## 支持格式

| 扩展名 | 文件格式 |
| --- | --- |
| `.litematic` | Litematica |
| `.schem` | Sponge 原理图，v2 与 v3 |
| `.schematic` | MCEdit，1.13 之前的格式 |
| `.nbt` | Java 结构方块 |
| `.snbt` | 结构 SNBT，花括号或方括号方块状态 |
| `.mcstructure` | 基岩版结构 |
| `.nusn` | Nucleation 快照 |

应用还为每种文件格式内置了一份示例原理图，可从欢迎界面进入。

## 致谢

特别感谢 @Nano112 的项目 [Nucleation](https://github.com/Schem-at/Nucleation) 为原理图解析与渲染提供支持。
