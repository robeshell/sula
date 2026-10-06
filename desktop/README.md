# Sula 桌面端开发

桌面端由 React 界面与 Tauri 原生应用组成。业务状态通过 Zustand 管理，文件、数据库与系统能力通过 Tauri commands/events 连接 Rust。模块边界见[架构说明](../docs/architecture.md)。

以下命令均从仓库根目录执行。

## 环境

- Rust 1.89 或更新版本，包含 Cargo。
- Node.js 22.12+，或 20.19+；版本要求来自当前 Vite 依赖。
- pnpm；CI 使用 pnpm 9，依赖以 `desktop/pnpm-lock.yaml` 为准。
- macOS：Xcode Command Line Tools。
- Windows：MSVC C++ 开发工具和 WebView2。
- Linux：WebKitGTK 4.1、AppIndicator、librsvg、DBus 开发库等系统依赖；仓库 CI 的 Ubuntu 安装清单见[构建工作流](../.github/workflows/desktop-build.yml)。

Linux 运行时需要可用的 Secret Service，例如 GNOME Keyring 或提供相应服务的 KWallet。凭据读取失败会影响配置加载，不应通过删除配置来绕过。

## 启动

```bash
pnpm --dir desktop install --frozen-lockfile
pnpm --dir desktop dev:app
```

Tauri 会启动 Vite 并运行原生应用。开发页面地址为 `http://localhost:1420`，但在普通浏览器直接打开不具备原生 IPC。浏览器组件检查应使用明确隔离的模拟数据入口。

## 定向检查

根据修改选择需要的命令，不默认全部执行：

```bash
# 前端类型
pnpm --dir desktop exec tsc --noEmit

# 前端状态、图片缓存、异步确认
pnpm --dir desktop test:store
pnpm --dir desktop test:poster-cache
pnpm --dir desktop test:confirmation

# Rust 应用边界
cargo check -p sula --lib --locked

# 配置和凭据存储逻辑；测试使用内存凭据替身
cargo test -p sula --lib config:: --locked
```

扫描、整理及刮削修改应运行对应 crate 和测试过滤器。真实文件、原生窗口、外部 API 和安装包验证需单独记录，参见[验证指南](../docs/perf.md)。

## 界面维护

| 位置 | 职责 |
| --- | --- |
| `src/components/ui/` | shadcn/ui 本地组件；来源和修改约定见目录内 README |
| `src/components/` | 资料库工具栏、设置、重命名、任务面板和弹窗等业务界面 |
| `src/store/appStore.ts` | 资料库、条目、选择、任务及操作状态 |
| `src/index.css` | 页面布局、领域组件样式和文字层级 |
| `src/styles/appearance.css` | 软件自己的主题、强调色与窗口布局参数 |
| `src/styles/components.css` | shadcn 组件与应用样式的适配 |
| `src/i18n/locales/` | 中、英、日文案 |

保持既定文字层级：页面标题 24/32、分区标题 18/24、正文和操作控件 14/20、辅助文字 12/16 或 12/18（字号/行高，单位 px）。更换组件不应引入另一套默认字号。

动效由 Motion 提供，根部遵循系统减少动态效果偏好。不要为长媒体列表的每个条目添加入场动画。虚拟列表的间距和行高计算必须与实际卡片几何一致。

## 原生入口与存储

`src-tauri/src/lib.rs` 注册命令与事件；`commands.rs` 负责 UI 调用边界，`state.rs` 初始化数据库与共享状态，`task_queue.rs` 管理后台任务。系统凭据由 `credentials.rs` 访问。

运行标识为 `app.wenworks.sula`，开发版（`dev:app`）为 `app.wenworks.sula.dev`，Rust 包为 `sula`，库为 `sula_lib`。具体数据位置见[数据与迁移](../docs/data.md)。

## 构建

仅在需要验证构建或产出安装包时执行。命令、产物路径与平台限制见[打包说明](../docs/packaging.md)。

开发缓存由 Cargo 和 Vite 管理。移动仓库后，原生构建脚本可能保留旧绝对路径；应依据错误清理受影响包的缓存，不能按目录名清空整个工作区。
