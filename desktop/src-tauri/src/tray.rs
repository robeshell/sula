use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;
use crate::task_queue::{TaskKind, TaskSnapshot, TaskStatus};
use crate::ui_i18n;

const TRAY_ID: &str = "sula-tray";
const FLASH_SECS: u64 = 3;

pub struct TrayHandle {
    tray: TrayIcon<Wry>,
    status: MenuItem<Wry>,
    task: MenuItem<Wry>,
    show: MenuItem<Wry>,
    cancel: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    sync: Mutex<SyncState>,
    locale: Mutex<String>,
}

struct SyncState {
    last_active_id: Option<String>,
    flash: Option<FlashState>,
    last_fingerprint: String,
}

struct FlashState {
    status: String,
    task: String,
    tooltip: String,
    title: String,
    until: Instant,
}

struct TrayDisplay {
    status: String,
    task: String,
    tooltip: String,
    title: String,
    cancel_enabled: bool,
}

/// 系统托盘菜单宽度由最长项决定，无法设字号；用最少字符撑开可读宽度。
const MENU_MIN_CHARS: usize = 36;

impl TrayHandle {
    fn apply(&self, display: &TrayDisplay) {
        let fingerprint = format!(
            "{}|{}|{}|{}|{}",
            display.status, display.task, display.tooltip, display.title, display.cancel_enabled
        );
        {
            let mut sync = self.sync.lock().expect("tray sync lock");
            if sync.last_fingerprint == fingerprint {
                return;
            }
            sync.last_fingerprint = fingerprint;
        }
        let _ = self.status.set_text(pad_menu(&display.status));
        let _ = self.task.set_text(pad_menu(&display.task));
        let _ = self.cancel.set_enabled(display.cancel_enabled);
        let _ = self.tray.set_tooltip(Some(&display.tooltip));
        #[cfg(target_os = "macos")]
        {
            let title = if display.title.is_empty() {
                None
            } else {
                Some(display.title.as_str())
            };
            let _ = self.tray.set_title(title);
        }
    }

    fn apply_static_labels(&self, locale: &str) {
        let _ = self.show.set_text(pad_menu(&ui_i18n::t(locale, "tray.show")));
        let _ = self.cancel.set_text(pad_menu(&ui_i18n::t(locale, "tray.cancel")));
        let _ = self.quit.set_text(pad_menu(&ui_i18n::t(locale, "tray.quit")));
        let idle = ui_i18n::t(locale, "tray.idle");
        let _ = self.status.set_text(pad_menu(&idle));
        let _ = self.task.set_text(pad_menu(&ui_i18n::t(locale, "tray.noTask")));
        let _ = self.tray.set_tooltip(Some(&idle));
        if let Ok(mut sync) = self.sync.lock() {
            sync.last_fingerprint.clear();
        }
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let locale = tray_locale_from_state(app);
    let status = MenuItem::with_id(
        app,
        "tray-status",
        pad_menu(&ui_i18n::t(&locale, "tray.idle")),
        false,
        None::<&str>,
    )?;
    let task = MenuItem::with_id(
        app,
        "tray-task",
        pad_menu(&ui_i18n::t(&locale, "tray.noTask")),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(
        app,
        "tray-show",
        pad_menu(&ui_i18n::t(&locale, "tray.show")),
        true,
        None::<&str>,
    )?;
    let cancel = MenuItem::with_id(
        app,
        "tray-cancel",
        pad_menu(&ui_i18n::t(&locale, "tray.cancel")),
        false,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "tray-quit",
        pad_menu(&ui_i18n::t(&locale, "tray.quit")),
        true,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&status, &task, &sep, &show, &cancel, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(ui_i18n::t(&locale, "tray.idle"))
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show_main_window(app),
            "tray-cancel" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>() {
                        let _ = state.tasks.cancel_active().await;
                    }
                });
            }
            "tray-quit" => {
                app.exit(0);
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    #[cfg(target_os = "windows")]
    {
        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
        builder = builder
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            });
    }

    let tray = builder.build(app)?;

    let enabled = tray_enabled_from_state(app);
    let _ = tray.set_visible(enabled);

    let handle = TrayHandle {
        tray,
        status,
        task,
        show,
        cancel,
        quit,
        sync: Mutex::new(SyncState {
            last_active_id: None,
            flash: None,
            last_fingerprint: String::new(),
        }),
        locale: Mutex::new(locale),
    };
    app.manage(handle);
    spawn_sync_loop(app.clone());
    Ok(())
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    if let Some(handle) = app.try_state::<TrayHandle>() {
        let _ = handle.tray.set_visible(enabled);
    }
}

pub fn set_locale(app: &AppHandle, locale: &str) {
    let Some(handle) = app.try_state::<TrayHandle>() else {
        return;
    };
    {
        let mut cur = handle.locale.lock().expect("tray locale lock");
        if cur.as_str() == locale {
            return;
        }
        *cur = locale.to_string();
    }
    handle.apply_static_labels(locale);
}

fn tray_enabled_from_state(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return true;
    };
    let enabled = match state.config.try_lock() {
        Ok(store) => store.config.tray_enabled,
        Err(_) => true,
    };
    enabled
}

fn tray_locale_from_state(app: &AppHandle) -> String {
    let Some(state) = app.try_state::<AppState>() else {
        return "zh-Hans".into();
    };
    let locale = match state.config.try_lock() {
        Ok(store) => store.config.ui_locale.clone(),
        Err(_) => "zh-Hans".into(),
    };
    locale
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn spawn_sync_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let Some(tray) = app.try_state::<TrayHandle>() else {
                continue;
            };
            let enabled = {
                let store = state.config.lock().await;
                store.config.tray_enabled
            };
            if !enabled {
                continue;
            }
            let locale = tray.locale.lock().expect("tray locale lock").clone();
            let tasks = state.tasks.list().await;
            let display = {
                let mut sync = tray.sync.lock().expect("tray sync lock");
                compute_display(&tasks, &mut sync, &locale)
            };
            tray.apply(&display);
        }
    });
}

fn compute_display(tasks: &[TaskSnapshot], sync: &mut SyncState, locale: &str) -> TrayDisplay {
    let active = tasks
        .iter()
        .find(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::Running));

    if let Some(task) = active {
        sync.last_active_id = Some(task.id.clone());
        sync.flash = None;
        return display_for_active(task, locale);
    }

    if let Some(id) = sync.last_active_id.take() {
        if let Some(task) = tasks.iter().find(|t| t.id == id) {
            if let Some(flash) = flash_for_terminal(task, locale) {
                sync.flash = Some(flash);
            }
        }
    }

    if let Some(flash) = &sync.flash {
        if Instant::now() < flash.until {
            return TrayDisplay {
                status: flash.status.clone(),
                task: flash.task.clone(),
                tooltip: flash.tooltip.clone(),
                title: flash.title.clone(),
                cancel_enabled: false,
            };
        }
        sync.flash = None;
    }

    TrayDisplay {
        status: ui_i18n::t(locale, "tray.idle"),
        task: ui_i18n::t(locale, "tray.noTask"),
        tooltip: ui_i18n::t(locale, "tray.idle"),
        title: String::new(),
        cancel_enabled: false,
    }
}

fn display_for_active(task: &TaskSnapshot, locale: &str) -> TrayDisplay {
    let kind = kind_label(task.kind, locale);
    let task_name = truncate(&task.title, 40);
    let unchanged = ui_i18n::t(locale, "prog.unchanged");
    let (status, title) = match &task.progress {
        Some(progress)
            if progress.stage_key.as_deref() == Some("checkDirectories")
                || progress.current == "scan.checking"
                || progress.current == "scan.unchanged"
                || progress.current.starts_with("检查")
                || progress.current.starts_with("目录") =>
        {
            let status = if progress.current == unchanged
                || progress.current == "scan.unchanged"
                || progress.current.contains("无变更")
                || progress.current.contains("変更なし")
                || progress.current.contains("No folder changes")
            {
                ui_i18n::t(locale, "tray.unchanged")
            } else {
                ui_i18n::t(locale, "tray.checking")
            };
            (status, String::new())
        }
        Some(progress) if progress.total == 0 => {
            let status = ui_i18n::tf(
                locale,
                "tray.runningCount",
                &[("kind", &kind), ("n", &progress.completed.to_string())],
            );
            let title = progress.completed.to_string();
            (status, title)
        }
        Some(progress) => {
            let short = format!("{}/{}", progress.completed, progress.total);
            let status = ui_i18n::tf(
                locale,
                "tray.runningProgress",
                &[("kind", &kind), ("progress", &short)],
            );
            (status, short)
        }
        None if task.status == TaskStatus::Pending => (ui_i18n::t(locale, "tray.queued"), String::new()),
        None => (
            ui_i18n::tf(locale, "tray.running", &[("kind", &kind)]),
            String::new(),
        ),
    };

    let detail = task
        .progress
        .as_ref()
        .map(|p| p.current.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let detail = truncate(detail, 48);
    let tooltip = if detail.is_empty() {
        format!("{status}\n{task_name}")
    } else {
        format!("{status}\n{task_name}\n{detail}")
    };

    TrayDisplay {
        status,
        task: task_name,
        tooltip,
        title,
        cancel_enabled: true,
    }
}

fn flash_for_terminal(task: &TaskSnapshot, locale: &str) -> Option<FlashState> {
    let until = Instant::now() + Duration::from_secs(FLASH_SECS);
    let task_name = truncate(&task.title, 40);
    match task.status {
        TaskStatus::Completed => Some(FlashState {
            status: ui_i18n::t(locale, "tray.done"),
            task: task_name.clone(),
            tooltip: ui_i18n::tf(locale, "tray.doneTip", &[("task", &task_name)]),
            title: String::new(),
            until,
        }),
        TaskStatus::Failed => {
            let err = task
                .error_message
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if s.starts_with("err.") {
                        ui_i18n::t(locale, s)
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| ui_i18n::t(locale, "tray.unknownError"));
            let status = ui_i18n::tf(locale, "tray.failed", &[("err", &truncate(&err, 28))]);
            Some(FlashState {
                tooltip: format!("{status}\n{task_name}"),
                status,
                task: task_name,
                title: String::new(),
                until,
            })
        }
        TaskStatus::Cancelled => Some(FlashState {
            status: ui_i18n::t(locale, "tray.cancelled"),
            task: task_name.clone(),
            tooltip: ui_i18n::tf(locale, "tray.cancelledTip", &[("task", &task_name)]),
            title: String::new(),
            until,
        }),
        _ => None,
    }
}

fn kind_label(kind: TaskKind, locale: &str) -> String {
    match kind {
        TaskKind::Refresh => ui_i18n::t(locale, "kind.refresh"),
        TaskKind::BatchScrape | TaskKind::Scrape | TaskKind::Rescrape | TaskKind::ManualMatch => {
            ui_i18n::t(locale, "kind.scrape")
        }
        TaskKind::Rename | TaskKind::Organize => ui_i18n::t(locale, "kind.rename"),
        TaskKind::Delete | TaskKind::Cleanup => ui_i18n::t(locale, "kind.cleanup"),
        TaskKind::Smoke => ui_i18n::t(locale, "kind.task"),
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn pad_menu(s: &str) -> String {
    let count = s.chars().count();
    if count >= MENU_MIN_CHARS {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(MENU_MIN_CHARS - count))
}
