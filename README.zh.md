<div align="center">
  <img src="icon.png" alt="LitematicaQL 图标" width="200"/>
  <h1>LitematicaQL</h1>
  <p><strong>在 macOS 上快速预览 Minecraft 原理图</strong></p>
  <!-- README-I18N:START -->

[English](./README.md) | **中文**

  <!-- README-I18N:END -->
</div>

LitematicaQL 为 Minecraft 原理图与结构提供了 macOS 快速查看预览扩展，支持 `.litematic`、`.schem`、`.schematic`、`.nbt`、`.snbt`、`.mcstructure`、`.nusn` 与 `.mca` 格式。

如果你使用的是 Windows，欢迎体验姐妹项目 [LitematicaPreview](https://github.com/Arcadi4/LitematicaPreview)，享受与 LitematicaQL 一样飞速的渲染体验！

<table>
<tr>
<th>预览投影</th>
<th>预览世界存档</th>
</tr>
<tr>
<td>https://github.com/user-attachments/assets/d2268ef1-2fcd-4232-af3f-1d29fd32ccb6</td>
<td>https://github.com/user-attachments/assets/d8b034f7-44b6-4940-8fc5-be5c5af25cb4</td>
</tr>
</table>

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

需要 macOS 13 Ventura 或更高版本，以及支持 Metal 的 GPU。

预览采用原生 Metal、分块流式网格生成、压缩顶点缓冲区和按需绘制。
共享的 [LitematicaPreview Core](https://github.com/Arcadi4/litematica-preview-core)
负责生成经过校验的顶点与索引缓冲区，并在纹理首次使用时提供 RGBA 像素；
原生宿主将这些数据上传到 Metal 的 GPU 专用缓冲区。几何数据上传时直接借用原生内存中
完整对齐的页，只暂存首尾不足一页的部分。原生批次会保留到 Metal 读取完成，
同时最多有两个上传任务在执行。

> [!NOTE]
> 如果预览没有出现，请在 **系统设置 → 通用 → 登录项与扩展 → 快速查看** 中启用 LitematicaQL。

## 材质包

打开 **LitematicaQL → Settings…**（`⌘ + ,`），选择 **Add Texture Packs…** 即可导入 Minecraft Java 版资源包 ZIP 文件。

在内嵌列表中勾选要启用的资源包，拖动已启用的行或使用箭头按钮调整优先级，最上方的资源包优先。缺失的纹理、模型和方块状态依次从下方资源包及内置原版资源中获取。

修改后，应用中打开的预览会重新加载，后续快速查看也会使用同一资源包顺序。原有的单资源包选择会保留。移除已保存的资源包不会删除原始 ZIP 文件。

## 预览限制

超大原理图默认会被拒绝预览，并询问是否仍要渲染。若要跳过警告并始终渲染，请在 **LitematicaQL → Settings…**（`⌘ + ,`）中启用 **Render oversized schematics without asking**。该设置同时作用于应用和快速查看；超大预览可能耗时很久或耗尽内存。

## 支持格式

| 扩展名         | 文件格式                                    |
| -------------- | ------------------------------------------- |
| `.litematic`   | Litematica                                  |
| `.schem`       | Sponge 原理图                               |
| `.schematic`   | MCEdit                                      |
| `.nbt`         | Java 结构方块                               |
| `.snbt`        | 结构 SNBT，花括号或方括号方块状态           |
| `.mcstructure` | 基岩版结构                                  |
| `.nusn`        | Nucleation 快照                             |
| `.mca`         | Minecraft Anvil 世界存档（每个文件 4 区块） |

## 开发

所有命令都由 [just](https://github.com/casey/just) 统一调度。不带参数运行即可列出全部配方：

```sh
just
```

| 命令                       | 作用                                                                                |
| -------------------------- | ----------------------------------------------------------------------------------- |
| `just doctor`              | 检查工具链依赖是否已安装                                                            |
| `just build`               | 构建临时签名（ad-hoc）的 Apple 芯片与 Intel 版本                                    |
| `just run`                 | 构建并打开应用                                                                      |
| `just test`                | 运行 Swift 文件校验测试与 Rust 桥接测试                                             |
| `just test-metal`          | 检查实际 Metal 上传与原生内存生命周期（需要支持 Metal 的 Mac）                       |
| `just check`               | 对 Rust 桥接层做类型检查                                                            |
| `just clean`               | 删除全部构建产物                                                                    |
| `just ci`                  | 在本地复现 CI 流程                                                                  |
| `just unregister`          | 清理 LitematicaQL 的旧 Quick Look 注册记录，同时保留 `/Applications` 中的已安装版本 |
| `just bump patch`          | 在 `main` 上提升版本号（默认 `patch`，也可选 `minor`、`major`），提交并打标签       |
| `just release 1.2.6 arm64` | 归档、校验并打包单一架构                                                            |
| `just demos`               | 重新生成内置演示原理图                                                              |

`just` 会读取环境变量 `CONFIGURATION`、`ARCHS` 与 `CURRENT_PROJECT_VERSION`，
CI 与发布矩阵正是通过它们驱动同一批配方。

### 构建

前置依赖：

- [just](https://github.com/casey/just)
- Xcode 26 或更高，包含 Metal Toolchain 组件
- Rust（rustup）
- [XcodeGen](https://github.com/yonaskolb/XcodeGen)

```sh
git clone https://github.com/Arcadi4/LitematicaQL.git
cd LitematicaQL

just build
```

## 致谢

特别感谢 [@Nano112](https://github.com/Nano112) 的项目 [Nucleation](https://github.com/Schem-at/Nucleation) 为整个解析与网格生成管线提供支持。

感谢我的慷慨好友 [@johnbean393](https://github.com/johnbean393) 分享其 Apple 开发者签名来为应用公证，在免去我 100 美元「苹果税」的同时带来了更好的开箱即用体验！也欢迎了解他们的项目 [Chiboard](https://www.chiboard.app)——一款基于机器学习的高效拼音输入法，能成倍提升中文打字速度。
