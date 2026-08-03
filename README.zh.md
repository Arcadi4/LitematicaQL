<div align="center">
  <img src="icon.png" alt="LitematicaQL 图标" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>在 macOS 上快速预览 Litematica 原理图</strong></p>
  <!-- README-I18N:START -->

  [English](./README.md) | **中文**

  <!-- README-I18N:END -->
</div>

LitematicaQL 为 `.litematic` 文件提供快速查看预览扩展。在访达中选中原理图，按下空格键，即可环绕、缩放、平移渲染出的方块——无需启动 Minecraft 或 Litematica。

## 功能特性

- 原生 macOS 应用，内嵌快速查看预览扩展
- 基于 [`schematic-renderer`](https://www.npmjs.com/package/schematic-renderer) 的 Three.js/WebGL 带纹理渲染
- 可交互的环绕、缩放与平移控制

## 环境要求

- macOS 13 Ventura 或更高

## 构建

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

## 安装

1. 将 `LitematicaQL.app` 移动到 `/Applications`。
2. 打开一次应用，让 macOS 注册其预览扩展。
3. 在访达中选中 `.litematic` 文件，按下空格键。

可以使用 [`Fixtures/LitematicaQL-Demo.litematic`](Fixtures/LitematicaQL-Demo.litematic) 试用，或在应用中点击 **View Example**。

> [!NOTE]
> 如果预览没有出现，请在 **系统设置 → 通用 → 登录项与扩展 → 快速查看** 中启用 LitematicaQL。

## 开发

运行渲染器的全部检查并重新构建其分发资源：

```sh
pnpm --prefix Renderer run check
```

运行原生的文件校验测试：

```sh
swift test
```

清理 LitematicaQL 的旧 Quick Look 注册记录，同时保留已安装的
`/Applications` 版本：

```sh
./scripts/clean-quick-look-registrations.sh
```

## 项目结构

| 路径 | 用途 |
| --- | --- |
| `App/` | SwiftUI 宿主应用与文档集成 |
| `PreviewExtension/` | 快速查看扩展入口 |
| `Shared/` | 文件校验与共享的 WKWebView 控制器 |
| `Renderer/` | Vite+/TypeScript 渲染器源码与测试 |
| `Resources/Renderer/` | 生成的自包含渲染器 bundle |
| `Fixtures/` | 示例 `.litematic` 测试文件 |
| `Tests/` | Swift 包测试 |
