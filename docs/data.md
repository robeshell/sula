# 数据、凭据与迁移

## 存储位置

应用标识为 `app.wenworks.sula`，开发版为 `app.wenworks.sula.dev`（`src-tauri/tauri.dev.conf.json`）。业务数据目录不随标识变化，两者共用。Rust 使用系统应用数据目录下的 `sula` 保存业务状态，实际位置由 `dirs::data_dir()` 决定。

| 平台 | 典型业务数据目录 |
| --- | --- |
| macOS | `~/Library/Application Support/sula/` |
| Windows | `%APPDATA%\sula\` |
| Linux | `$XDG_DATA_HOME/sula/`，未设置时通常为 `~/.local/share/sula/` |

| 内容 | 名称 |
| --- | --- |
| 媒体索引及操作记录 | `sula.sqlite3` |
| 配置与凭据引用 | `config.toml` |
| 后台任务历史 | `task_history.json`（写入后创建） |
| 重命名恢复数据 | `rename_snapshots/` |
| 重命名预设 | `rename_presets/` |
| 实例互斥锁 | `application.lock` |

缩略图和头像缓存位于系统缓存目录的 `sula/thumbnails`、`sula/avatars`。WebView 的站点存储由平台管理，与业务数据库分开；例如详情面板宽度属于前端本地偏好。

媒体文件、NFO 和海报仍位于用户选择的资料库目录。备份应用数据不等于备份媒体文件。

## API 密钥

系统凭据服务名为 `sula.scraper-keys`。配置文件保存凭据引用，正常保存流程不将 API 密钥明文写入配置。已有明文配置可在加载时迁入凭据库；凭据服务不可用或写入失败时不能假装迁移成功。

macOS 使用钥匙串，Windows 使用系统凭据服务，Linux 使用 Secret Service。仅复制 `config.toml` 到另一台电脑不能同时复制系统凭据；在新环境需要重新配置密钥。

## 备份与恢复

先停止应用及后台任务，再备份整个业务数据目录。SQLite 运行期间可能存在 `-wal` 和 `-shm` 文件，不要在运行中只复制主数据库文件并假定副本完整；在线备份需要使用 SQLite 的备份接口。

恢复时一并考虑数据库、配置、重命名恢复数据及预设。系统凭据单独管理。不要在恢复过程中直接清空操作日志来绕过未完成的文件变更。

## Sula 更名迁移

本机更名已完成一次性迁移：备份并检查数据库，迁移数据与缓存目录，将凭据复制到新服务并比较验证。数据库备份位于业务数据目录的 `backups/before-sula-rename/sula.sqlite3`。

这次操作不是面向所有已分发版本的自动迁移机制。旧版本用户不能仅通过替换应用标识就假定数据与凭据会自动找到；分发升级前需单独实现或执行迁移流程。具体执行记录见[全面更名记录](rename.md)。
