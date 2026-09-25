//! UI strings for Rust surfaces (tray, task titles, progress). Driven by `AppConfig.ui_locale`.

pub fn normalize(locale: &str) -> &str {
    match locale {
        "en" | "en-US" | "en-GB" => "en",
        "ja" | "ja-JP" => "ja",
        _ => "zh-Hans",
    }
}

/// Maps a BCP-47-ish tag (`en_US.UTF-8`, `ja-JP`, `zh-Hant`) to a supported UI locale.
pub fn from_tag(tag: &str) -> Option<&'static str> {
    let lower = tag.to_ascii_lowercase();
    if lower.starts_with("en") { Some("en") } else if lower.starts_with("ja") { Some("ja") } else if lower.starts_with("zh") { Some("zh-Hans") } else { None }
}

/// Best-effort OS locale for surfaces shown before the config is available.
pub fn system_locale() -> &'static str {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter()
        .filter_map(|name| std::env::var(name).ok())
        .find_map(|value| from_tag(&value))
        .unwrap_or("zh-Hans")
}

pub fn t(locale: &str, key: &str) -> String {
    let locale = normalize(locale);
    if let Some(text) = lookup(locale, key) {
        return text;
    }
    // Tolerate case variants like `err.apikey` from older builds / wrappers.
    if key.starts_with("err.") {
        let lower = key.to_ascii_lowercase();
        let canon = match lower.as_str() {
            "err.apikey" => "err.apiKey",
            "err.forbidden" => "err.forbidden",
            "err.ratelimit" => "err.rateLimit",
            "err.connect" => "err.connect",
            _ => key,
        };
        if canon != key {
            if let Some(text) = lookup(locale, canon) {
                return text;
            }
        }
    }
    key.to_string()
}

pub fn tf(locale: &str, key: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = t(locale, key);
    for (k, v) in pairs {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn lookup(locale: &str, key: &str) -> Option<String> {
    Some(match (locale, key) {
        // Brand / tray idle
        ("en", "tray.idle") => "Sula · Idle".into(),
        ("ja", "tray.idle") => "Sula · 待機".into(),
        (_, "tray.idle") => "Sula · 空闲".into(),

        ("en", "tray.noTask") => "No active task".into(),
        ("ja", "tray.noTask") => "実行中のタスクなし".into(),
        (_, "tray.noTask") => "当前无任务".into(),

        ("en", "tray.show") => "Show main window".into(),
        ("ja", "tray.show") => "メインウィンドウを表示".into(),
        (_, "tray.show") => "显示主窗口".into(),

        ("en", "tray.cancel") => "Cancel current task".into(),
        ("ja", "tray.cancel") => "現在のタスクをキャンセル".into(),
        (_, "tray.cancel") => "取消当前任务".into(),

        ("en", "tray.quit") => "Quit Sula".into(),
        ("ja", "tray.quit") => "Sulaを終了".into(),
        (_, "tray.quit") => "退出Sula".into(),

        ("en", "tray.done") => "Done".into(),
        ("ja", "tray.done") => "完了".into(),
        (_, "tray.done") => "已完成".into(),

        ("en", "tray.cancelled") => "Cancelled".into(),
        ("ja", "tray.cancelled") => "キャンセル".into(),
        (_, "tray.cancelled") => "已取消".into(),

        ("en", "tray.failed") => "Failed: {{err}}".into(),
        ("ja", "tray.failed") => "失敗：{{err}}".into(),
        (_, "tray.failed") => "失败：{{err}}".into(),

        ("en", "tray.unknownError") => "Unknown error".into(),
        ("ja", "tray.unknownError") => "不明なエラー".into(),
        (_, "tray.unknownError") => "未知错误".into(),

        ("en", "tray.queued") => "Queued".into(),
        ("ja", "tray.queued") => "待機中".into(),
        (_, "tray.queued") => "排队中".into(),

        ("en", "tray.checking") => "Checking folders".into(),
        ("ja", "tray.checking") => "フォルダを確認中".into(),
        (_, "tray.checking") => "检查目录中".into(),

        ("en", "tray.unchanged") => "No folder changes".into(),
        ("ja", "tray.unchanged") => "フォルダに変更なし".into(),
        (_, "tray.unchanged") => "目录无变更".into(),

        ("en", "tray.runningCount") => "{{kind}} · found {{n}}".into(),
        ("ja", "tray.runningCount") => "{{kind}}中 · {{n}} 件検出".into(),
        (_, "tray.runningCount") => "{{kind}}中 · 已发现 {{n}}".into(),

        ("en", "tray.runningProgress") => "{{kind}} {{progress}}".into(),
        ("ja", "tray.runningProgress") => "{{kind}}中 {{progress}}".into(),
        (_, "tray.runningProgress") => "{{kind}}中 {{progress}}".into(),

        ("en", "tray.running") => "{{kind}}…".into(),
        ("ja", "tray.running") => "{{kind}}中".into(),
        (_, "tray.running") => "{{kind}}中".into(),

        ("en", "tray.doneTip") => "Sula · Done\n{{task}}".into(),
        ("ja", "tray.doneTip") => "Sula · 完了\n{{task}}".into(),
        (_, "tray.doneTip") => "Sula · 已完成\n{{task}}".into(),

        ("en", "tray.cancelledTip") => "Sula · Cancelled\n{{task}}".into(),
        ("ja", "tray.cancelledTip") => "Sula · キャンセル\n{{task}}".into(),
        (_, "tray.cancelledTip") => "Sula · 已取消\n{{task}}".into(),

        ("en", "kind.refresh") => "Scan".into(),
        ("ja", "kind.refresh") => "スキャン".into(),
        (_, "kind.refresh") => "扫描".into(),

        ("en", "kind.scrape") => "Scrape".into(),
        ("ja", "kind.scrape") => "スクレイプ".into(),
        (_, "kind.scrape") => "刮削".into(),

        ("en", "kind.rename") => "Rename".into(),
        ("ja", "kind.rename") => "改名".into(),
        (_, "kind.rename") => "改名".into(),

        ("en", "kind.cleanup") => "Cleanup".into(),
        ("ja", "kind.cleanup") => "整理".into(),
        (_, "kind.cleanup") => "清理".into(),

        ("en", "kind.task") => "Task".into(),
        ("ja", "kind.task") => "タスク".into(),
        (_, "kind.task") => "任务".into(),

        // Task titles / progress
        ("en", "task.refreshItems") => "Refresh items from disk".into(),
        ("ja", "task.refreshItems") => "ディスクから項目を更新".into(),
        (_, "task.refreshItems") => "从磁盘刷新条目".into(),

        ("en", "task.refreshItemsN") => "Refresh from disk · {{n}} items".into(),
        ("ja", "task.refreshItemsN") => "ディスクから更新 · {{n}} 件".into(),
        (_, "task.refreshItemsN") => "从磁盘刷新 · {{n}} 项".into(),

        ("en", "task.refreshLib") => "Refresh · {{name}}".into(),
        ("ja", "task.refreshLib") => "更新 · {{name}}".into(),
        (_, "task.refreshLib") => "刷新 · {{name}}".into(),

        ("en", "task.scrapeAll") => "Scrape all · {{name}}".into(),
        ("ja", "task.scrapeAll") => "すべてスクレイプ · {{name}}".into(),
        (_, "task.scrapeAll") => "全部刮削 · {{name}}".into(),

        ("en", "task.scrapeN") => "Scrape · {{n}} items".into(),
        ("ja", "task.scrapeN") => "スクレイプ · {{n}} 件".into(),
        (_, "task.scrapeN") => "刮削 · {{n}} 项".into(),

        ("en", "task.rescrapeN") => "Re-scrape · {{n}} items".into(),
        ("ja", "task.rescrapeN") => "再スクレイプ · {{n}} 件".into(),
        (_, "task.rescrapeN") => "重新刮削 · {{n}} 项".into(),

        ("en", "task.renameN") => "Rename · {{n}} items".into(),
        ("ja", "task.renameN") => "改名 · {{n}} 件".into(),
        (_, "task.renameN") => "改名 · {{n}} 项".into(),

        ("en", "task.organizeN") => "Organize seasons · {{n}} items".into(),
        ("ja", "task.organizeN") => "シーズン整理 · {{n}} 件".into(),
        (_, "task.organizeN") => "整理季文件夹 · {{n}} 项".into(),

        ("en", "task.cleanupN") => "Clean residuals · {{n}} files".into(),
        ("ja", "task.cleanupN") => "残余整理 · {{n}} 件".into(),
        (_, "task.cleanupN") => "清理残余 · {{n}} 个文件".into(),

        ("en", "task.manualMatch") => "Manual match · {{title}}".into(),
        ("ja", "task.manualMatch") => "手動マッチ · {{title}}".into(),
        (_, "task.manualMatch") => "手动匹配 · {{title}}".into(),

        ("en", "prog.refreshing") => "Refreshing…".into(),
        ("ja", "prog.refreshing") => "更新中…".into(),
        (_, "prog.refreshing") => "刷新中…".into(),

        ("en", "prog.checking") => "Checking folders…".into(),
        ("ja", "prog.checking") => "フォルダを確認中…".into(),
        (_, "prog.checking") => "检查目录…".into(),

        ("en", "prog.unchanged") => "No folder changes".into(),
        ("ja", "prog.unchanged") => "フォルダに変更なし".into(),
        (_, "prog.unchanged") => "目录无变更".into(),

        ("en", "prog.added") => "Added {{n}} items".into(),
        ("ja", "prog.added") => "{{n}} 件を追加".into(),
        (_, "prog.added") => "新增 {{n}} 项".into(),

        ("en", "prog.refreshDone") => "Refreshed {{ok}} · removed {{removed}}".into(),
        ("ja", "prog.refreshDone") => "更新 {{ok}} · 削除 {{removed}}".into(),
        (_, "prog.refreshDone") => "已刷新 {{ok}} · 移除 {{removed}}".into(),

        ("en", "prog.autoRename") => "Auto-rename".into(),
        ("ja", "prog.autoRename") => "自動改名".into(),
        (_, "prog.autoRename") => "自动改名".into(),

        ("en", "prog.mergedShows") => "Merged {{n}} duplicate show(s)".into(),
        ("ja", "prog.mergedShows") => "重複シリーズ {{n}} 件を統合".into(),
        (_, "prog.mergedShows") => "已合并 {{n}} 部重复剧集".into(),

        ("en", "prog.autoRenameResult") => "rename ok {{ok}} · failed {{failed}}".into(),
        ("ja", "prog.autoRenameResult") => "改名成功 {{ok}} · 失敗 {{failed}}".into(),
        (_, "prog.autoRenameResult") => "改名成功 {{ok}} · 失败 {{failed}}".into(),

        ("en", "prog.renamed") => "Renamed {{n}} items".into(),
        ("ja", "prog.renamed") => "{{n}} 件を改名".into(),
        (_, "prog.renamed") => "已改名 {{n}} 项".into(),

        ("en", "prog.organized") => "Organized {{n}} items".into(),
        ("ja", "prog.organized") => "{{n}} 件を整理".into(),
        (_, "prog.organized") => "已整理 {{n}} 项".into(),

        ("en", "prog.cleaning") => "Cleaning…".into(),
        ("ja", "prog.cleaning") => "整理中…".into(),
        (_, "prog.cleaning") => "清理中…".into(),

        ("en", "prog.cleaned") => "Cleaned {{n}} files".into(),
        ("ja", "prog.cleaned") => "{{n}} 件を整理".into(),
        (_, "prog.cleaned") => "已清理 {{n}} 个文件".into(),

        ("en", "prog.scrapeSummary") => "OK {{success}} · unmatched {{unmatched}} · failed {{failed}}".into(),
        ("ja", "prog.scrapeSummary") => "成功 {{success}} · 未自動マッチ {{unmatched}} · 失敗 {{failed}}".into(),
        (_, "prog.scrapeSummary") => "成功 {{success}} · 未自动匹配 {{unmatched}} · 失败 {{failed}}".into(),

        ("en", "window.renamer") => "Batch Rename".into(),
        ("ja", "window.renamer") => "一括リネーム".into(),
        (_, "window.renamer") => "批量重命名".into(),

        ("en", "err.apiKey") => "API key missing or invalid — check Settings".into(),
        ("ja", "err.apiKey") => "API キーが無効または未設定です。設定を確認してください".into(),
        (_, "err.apiKey") => "API Key 无效或未填写，请到设置中检查".into(),

        ("en", "err.forbidden") => "Access denied (403) — check API key permissions".into(),
        ("ja", "err.forbidden") => "アクセス拒否（403）。API キー権限を確認してください".into(),
        (_, "err.forbidden") => "接口拒绝访问（403），请检查 API Key 权限".into(),

        ("en", "err.rateLimit") => "Too many requests — try again later".into(),
        ("ja", "err.rateLimit") => "リクエストが多すぎます。しばらくしてから再試行してください".into(),
        (_, "err.rateLimit") => "请求过于频繁，请稍后再试".into(),

        ("en", "err.connect") => "Cannot reach scrape servers. If using a proxy/VPN, enable system proxy or TUN mode.".into(),
        ("ja", "err.connect") => "スクレイプサーバーに接続できません。プロキシ利用時はシステムプロキシまたは TUN を有効にしてください。".into(),
        (_, "err.connect") => "无法连接刮削服务器（常见于代理未开系统代理/TUN）。请检查网络后重试".into(),

        ("en", "err.notTvShow") => "Only TV/anime seasons can be scraped".into(),
        ("ja", "err.notTvShow") => "ドラマ/アニメのシーズンのみスクレイプできます".into(),
        (_, "err.notTvShow") => "仅剧集/动漫可刮削季数据".into(),

        ("en", "err.notScraped") => "Scrape the show first".into(),
        ("ja", "err.notScraped") => "先に番組をスクレイプしてください".into(),
        (_, "err.notScraped") => "请先刮削整部剧集".into(),

        ("en", "err.noTmdbId") => "No TMDB id on this show — scrape or rematch first".into(),
        ("ja", "err.noTmdbId") => "TMDB ID がありません。先にスクレイプまたは再マッチしてください".into(),
        (_, "err.noTmdbId") => "该条目没有 TMDB ID，请先刮削或重新匹配".into(),

        ("en", "startup.failedTitle") => "Sula could not start".into(),
        ("ja", "startup.failedTitle") => "Sula を起動できません".into(),
        (_, "startup.failedTitle") => "Sula 无法启动".into(),

        ("en", "startup.failedBody") => "Sula failed to initialize its data and will now quit.\n\n{{err}}".into(),
        ("ja", "startup.failedBody") => "Sula のデータを初期化できなかったため終了します。\n\n{{err}}".into(),
        (_, "startup.failedBody") => "Sula 初始化数据失败，即将退出。\n\n{{err}}".into(),

        ("en", "startup.alreadyRunning") => "Sula is already running. Use the existing window or the tray icon.".into(),
        ("ja", "startup.alreadyRunning") => "Sula はすでに起動しています。既存のウィンドウまたはトレイアイコンを使用してください。".into(),
        (_, "startup.alreadyRunning") => "Sula 已在运行，请使用已打开的窗口或托盘图标。".into(),

        _ => return None,
    })
}
