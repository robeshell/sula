# Sula 全面更名记录

> 阶段记录：以下内容描述记录当时的工作和验证范围。现行使用与开发说明见[文档目录](README.md)。

2026-09-25。

- GitHub 仓库：`robeshell/sula`；本地仓库目录：`/Users/wangwenyu/Documents/Code/sula`。
- 前端与 Rust 包：`sula`；Rust 库：`sula_lib`；应用标识：`com.sula.app`。
- 窗口、托盘、文案、User-Agent、环境变量、临时文件前缀、图标源文件名与文档统一更名。
- 数据目录：系统应用数据目录下的 `sula`；数据库：`sula.sqlite3`；系统凭据服务：`sula.scraper-keys`。
- 本机数据库已在停止应用后备份、检查并迁移；配置、重命名预设与恢复记录随目录保留。数据库备份位于数据目录的 `backups/before-sula-rename/sula.sqlite3`。
- 本机系统凭据已复制到新服务并在内存中比较验证，未输出密钥；原凭据保留用于回滚。缓存及 WebKit 数据目录已迁移。
- 这是本机的一次性迁移，不包含已分发版本的自动升级迁移器。旧安装包、Git 历史和可再生构建产物未重写。
- 图标源文件名称已统一；图形本身未重新设计。

验证：新路径 TypeScript 检查通过；`cargo test -p sula --lib config:: --locked` 的 5 项配置/凭据测试通过；迁移后 9 张表的行数与备份一致，数据库完整性检查通过。目录变更引起的 Tauri 绝对路径缓存已按依赖包清理并重新生成。

新目录下 `pnpm --dir desktop tauri dev` 启动成功，运行二进制为 `target/debug/sula`，日志确认打开新路径的 `sula.sqlite3`。
