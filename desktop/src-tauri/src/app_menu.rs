//! macOS menu bar: the standard app / File / Edit / View / Window / Help menus
//! (same layout as Tauri's default) plus "Settings…" (⌘,), localized via `ui_i18n`.
//! Windows and Linux get no menu bar.

use std::sync::{Mutex, PoisonError};

use tauri::menu::{
    AboutMetadata, Menu, MenuBuilder, MenuEvent, MenuItem, SubmenuBuilder, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Manager, Wry};

use crate::ui_i18n::{t, tf};

const SETTINGS_ID: &str = "app-settings";

struct MenuLocale(Mutex<String>);

pub fn setup(app: &AppHandle, locale: &str) -> tauri::Result<()> {
    app.manage(MenuLocale(Mutex::new(locale.to_string())));
    // Replaces Tauri's default macOS menu, which has no Settings item.
    app.set_menu(build(app, locale)?)?;
    app.on_menu_event(|app, event| handle_event(app, &event));
    Ok(())
}

/// Rebuilds the menu bar when the UI locale changed.
pub fn set_locale(app: &AppHandle, locale: &str) {
    let Some(current) = app.try_state::<MenuLocale>() else {
        return;
    };
    {
        let mut current = current.0.lock().unwrap_or_else(PoisonError::into_inner);
        if current.as_str() == locale {
            return;
        }
        *current = locale.to_string();
    }
    match build(app, locale) {
        Ok(menu) => {
            let _ = app.set_menu(menu);
        }
        Err(error) => tracing::warn!(%error, "failed to rebuild the menu bar"),
    }
}

fn handle_event(app: &AppHandle, event: &MenuEvent) {
    if event.id() == SETTINGS_ID {
        crate::commands::spawn_open_settings(app);
    }
}

fn build(app: &AppHandle, locale: &str) -> tauri::Result<Menu<Wry>> {
    let package = app.package_info();
    let config = app.config();
    let name = package.name.clone();
    let app_name = [("app", name.as_str())];
    let about = AboutMetadata {
        name: Some(name.clone()),
        version: Some(package.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|publisher| vec![publisher]),
        ..Default::default()
    };
    let settings = MenuItem::with_id(app, SETTINGS_ID, t(locale, "menu.settings"), true, Some("CmdOrCtrl+,"))?;

    let app_menu = SubmenuBuilder::new(app, &name)
        .about_with_text(tf(locale, "menu.about", &app_name), Some(about))
        .separator()
        .item(&settings)
        .separator()
        .services_with_text(t(locale, "menu.services"))
        .separator()
        .hide_with_text(tf(locale, "menu.hide", &app_name))
        .hide_others_with_text(t(locale, "menu.hideOthers"))
        .show_all_with_text(t(locale, "menu.showAll"))
        .separator()
        .quit_with_text(tf(locale, "menu.quit", &app_name))
        .build()?;
    let file_menu = SubmenuBuilder::new(app, t(locale, "menu.file"))
        .close_window_with_text(t(locale, "menu.closeWindow"))
        .build()?;
    // Text fields rely on these items for ⌘C / ⌘V / ⌘A / ⌘Z on macOS.
    let edit_menu = SubmenuBuilder::new(app, t(locale, "menu.edit"))
        .undo_with_text(t(locale, "menu.undo"))
        .redo_with_text(t(locale, "menu.redo"))
        .separator()
        .cut_with_text(t(locale, "menu.cut"))
        .copy_with_text(t(locale, "menu.copy"))
        .paste_with_text(t(locale, "menu.paste"))
        .select_all_with_text(t(locale, "menu.selectAll"))
        .build()?;
    let view_menu = SubmenuBuilder::new(app, t(locale, "menu.view"))
        .fullscreen_with_text(t(locale, "menu.fullscreen"))
        .build()?;
    let window_menu = SubmenuBuilder::with_id(app, WINDOW_SUBMENU_ID, t(locale, "menu.window"))
        .minimize_with_text(t(locale, "menu.minimize"))
        .maximize_with_text(t(locale, "menu.zoom"))
        .separator()
        .close_window_with_text(t(locale, "menu.closeWindow"))
        .build()?;
    let help_menu = SubmenuBuilder::with_id(app, HELP_SUBMENU_ID, t(locale, "menu.help")).build()?;

    MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu, &help_menu])
        .build()
}
