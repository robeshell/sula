# Sula 打包与分发

开发启动使用 `pnpm --dir desktop dev:app`。只有需要生成产物或验证构建路径时才打包；日常文案、样式和文档修改不触发完整打包。

以下命令从仓库根目录执行。环境准备见[开发指南](../desktop/README.md)。

## 按当前平台构建

```bash
pnpm --dir desktop install --frozen-lockfile

# macOS
pnpm --dir desktop tauri build --bundles app,dmg

# Windows
pnpm --dir desktop tauri build --bundles nsis

# Linux
pnpm --dir desktop tauri build --bundles deb,appimage
```

选择与你的构建主机对应的一条命令。仓库的 `build:app` 脚本等价于 `tauri build`，具体 bundle 由配置和平台决定；明确传入 `--bundles` 更便于控制产物范围。

若只需调试构建而不生成安装包，必须显式关闭打包：

```bash
pnpm --dir desktop tauri build --debug --no-bundle
```

## 产物位置

本项目是 Cargo workspace。未指定 `CARGO_TARGET_DIR` 或 `--target` 时，发布安装包位于**仓库根目录**的 `target/release/bundle/`，不是 `desktop/src-tauri/target/`。

显式指定 Rust target 时，通常位于 `target/<target-triple>/release/bundle/`。调试构建使用对应的 `debug` 目录；`--no-bundle` 不产生安装包。

## 当前 CI

[desktop-build.yml](../.github/workflows/desktop-build.yml) 分别在 macOS、Windows 和 Ubuntu 上构建并上传 artifact：

| 主机 | bundle | artifact 名称 |
| --- | --- | --- |
| macOS | app、dmg | `sula-macos-latest` |
| Windows | nsis | `sula-windows-latest` |
| Ubuntu 22.04 | deb、appimage | `sula-ubuntu-22.04` |

工作流安装矩阵指定的 Rust target，但构建命令没有传 `--target`，实际仍使用 runner 默认主机目标。不能仅凭矩阵字段宣称产物已覆盖其他架构。

当前工作流不包含发布 Release、签名证书导入或公证步骤。artifact 构建成功也不代表安装、升级、托盘、系统凭据和真实文件操作已经验收。

## 签名与版本

macOS 正式分发需要单独配置签名及公证，Windows 正式分发需考虑代码签名。证书与密码应通过受控环境或 CI secrets 提供，不写入仓库。仓库未完成的发布配置不能在文档中当作已启用能力。

发布前同步根 `Cargo.toml` 的 workspace 版本、`desktop/package.json` 和 `desktop/src-tauri/tauri.conf.json`。升级应用标识或数据格式时，还需核对[数据迁移](data.md)。

## 图标

可编辑源为 `desktop/src-tauri/icons/Sula.icon/`，高分辨率图片源为 `desktop/src-tauri/icons/sula_master-v3.png`。更名时只统一了文件名，未重新设计图形。

在具备相应 Xcode 工具的 macOS 上更新源图后执行：

```bash
./desktop/scripts/generate-app-icons.sh
```

脚本通过 Tauri 生成跨平台图标，再用 `actool` 编译分层母版并覆盖 macOS `icon.icns`。应检查实际产物后再打包，不因文字更名重复生成图标。
