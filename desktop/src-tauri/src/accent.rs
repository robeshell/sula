//! OS accent color as `#RRGGBB` (sRGB), so the UI can follow the system tint.
//!
//! - macOS: `NSColor.controlAccentColor` converted to sRGB (graphite yields a gray).
//! - Windows: `HKCU\Software\Microsoft\Windows\DWM\AccentColor` (DWORD, 0xAABBGGRR).
//! - Linux: freedesktop portal `org.freedesktop.appearance` / `accent-color`.
//!
//! Changes are picked up when any window gains focus (and on macOS also via
//! `NSSystemColorsDidChangeNotification`) and broadcast as [`CHANGED_EVENT`].

use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Emitter, Manager};

/// Payload: the new `"#RRGGBB"` string, or `null` when the accent is unknown.
pub const CHANGED_EVENT: &str = "system-accent-changed";

/// Last accent seen; the outer `None` means "not read yet".
#[derive(Default)]
pub struct AccentState {
    last: Mutex<Option<Option<String>>>,
}

/// Registers state, starts the macOS observer and seeds the first value.
/// Must be called from the main thread (Tauri `setup`).
pub fn setup(app: &AppHandle) {
    app.manage(AccentState::default());
    #[cfg(target_os = "macos")]
    platform::observe_system_colors(app.clone());
    refresh(app);
}

/// Re-reads the accent and emits [`CHANGED_EVENT`] when it differs from the last
/// known value. On macOS this must run on the main thread (window/menu events do).
pub fn refresh(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    update(app, platform::read());
    #[cfg(not(target_os = "macos"))]
    {
        // The Linux portal call may block up to its timeout; keep it off the UI thread.
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || update(&app, platform::read()));
    }
}

#[tauri::command]
pub async fn get_system_accent(app: AppHandle) -> Option<String> {
    let value = read_async(&app).await;
    update(&app, value.clone());
    value
}

async fn read_async(app: &AppHandle) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // AppKit colors resolve against the current appearance; read them on the main thread.
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(platform::read());
        })
        .ok()?;
        rx.await.ok().flatten()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(platform::read).await.ok().flatten()
    }
}

fn update(app: &AppHandle, value: Option<String>) {
    let Some(state) = app.try_state::<AccentState>() else {
        return;
    };
    let changed = {
        let mut last = state.last.lock().unwrap_or_else(PoisonError::into_inner);
        record(&mut last, value.clone())
    };
    if changed {
        let _ = app.emit(CHANGED_EVENT, value);
    }
}

/// Stores `value`; true when it replaced a different, previously known value.
/// The first read only seeds the slot (the frontend asks via the command).
fn record(slot: &mut Option<Option<String>>, value: Option<String>) -> bool {
    let changed = matches!(slot, Some(previous) if *previous != value);
    *slot = Some(value);
    changed
}

fn rgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Windows DWM colors are stored as 0xAABBGGRR.
#[cfg_attr(not(windows), allow(dead_code))]
fn abgr_dword_to_hex(value: u32) -> String {
    let [r, g, b, _a] = value.to_le_bytes();
    rgb_hex(r, g, b)
}

/// Components in 0..=1 (slightly out-of-gamut values are clamped); None for NaN/inf.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn srgb_components_to_hex(r: f64, g: f64, b: f64) -> Option<String> {
    let channel = |c: f64| c.is_finite().then(|| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(rgb_hex(channel(r)?, channel(g)?, channel(b)?))
}

/// The portal reports out-of-range components when no accent is configured.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn portal_accent_to_hex(r: f64, g: f64, b: f64) -> Option<String> {
    let in_range = |c: f64| (0.0..=1.0).contains(&c);
    (in_range(r) && in_range(g) && in_range(b)).then(|| srgb_components_to_hex(r, g, b)).flatten()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2_app_kit::{NSColor, NSColorSpace, NSSystemColorsDidChangeNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};
    use tauri::AppHandle;

    pub fn read() -> Option<String> {
        let accent = NSColor::controlAccentColor();
        let srgb = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        super::srgb_components_to_hex(srgb.redComponent(), srgb.greenComponent(), srgb.blueComponent())
    }

    /// Live updates while the app stays in the background (System Settings open).
    pub fn observe_system_colors(app: AppHandle) {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| super::refresh(&app));
        let center = NSNotificationCenter::defaultCenter();
        let main_queue = NSOperationQueue::mainQueue();
        // SAFETY: the notification name is an AppKit constant; the block runs on the
        // main queue, which `refresh` requires on macOS.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSSystemColorsDidChangeNotification),
                None,
                Some(&main_queue),
                &block,
            )
        };
        // Observe for the lifetime of the process.
        std::mem::forget(observer);
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn read() -> Option<String> {
        let key = wide(r"Software\Microsoft\Windows\DWM");
        let name = wide("AccentColor");
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: NUL-terminated UTF-16 strings; the output buffer is a u32 of `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut value as *mut u32).cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS).then(|| super::abgr_dword_to_hex(value))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::time::Duration;

    use dbus::arg::{ArgType, RefArg, Variant};
    use dbus::blocking::Connection;

    pub fn read() -> Option<String> {
        let conn = Connection::new_session().ok()?;
        let proxy = conn.with_proxy(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            Duration::from_millis(500),
        );
        // `Read` wraps the value in an extra variant on older portals; the walker unwraps both.
        let (value,): (Variant<Box<dyn RefArg>>,) = proxy
            .method_call("org.freedesktop.portal.Settings", "Read", ("org.freedesktop.appearance", "accent-color"))
            .ok()?;
        let mut components = Vec::with_capacity(3);
        collect_doubles(&value, &mut components);
        match components[..] {
            [r, g, b] => super::portal_accent_to_hex(r, g, b),
            _ => None,
        }
    }

    fn collect_doubles(arg: &dyn RefArg, out: &mut Vec<f64>) {
        match arg.arg_type() {
            ArgType::Double => out.extend(arg.as_f64()),
            ArgType::Variant | ArgType::Struct => {
                if let Some(items) = arg.as_iter() {
                    for item in items {
                        collect_doubles(item, out);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    pub fn read() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgr_dword_is_little_endian_rgb() {
        // Windows default blue #0078D7 stored as 0xFFD77800.
        assert_eq!(abgr_dword_to_hex(0xFFD7_7800), "#0078D7");
        assert_eq!(abgr_dword_to_hex(0x0000_00FF), "#FF0000");
        assert_eq!(abgr_dword_to_hex(0x00FF_0000), "#0000FF");
    }

    #[test]
    fn srgb_components_round_and_clamp() {
        assert_eq!(srgb_components_to_hex(0.0, 0.5, 1.0).as_deref(), Some("#0080FF"));
        // macOS system blue in sRGB ≈ (0.0, 0.478, 1.0).
        assert_eq!(srgb_components_to_hex(0.0, 0.478_431, 1.0).as_deref(), Some("#007AFF"));
        assert_eq!(srgb_components_to_hex(-0.001, 1.002, 0.5).as_deref(), Some("#00FF80"));
        assert_eq!(srgb_components_to_hex(f64::NAN, 0.0, 0.0), None);
        assert_eq!(srgb_components_to_hex(0.0, f64::INFINITY, 0.0), None);
    }

    #[test]
    fn portal_out_of_range_means_unset() {
        assert_eq!(portal_accent_to_hex(0.2, 0.4, 0.6).as_deref(), Some("#336699"));
        assert_eq!(portal_accent_to_hex(-1.0, -1.0, -1.0), None);
        assert_eq!(portal_accent_to_hex(1.1, 0.0, 0.0), None);
    }

    #[test]
    fn record_seeds_silently_then_reports_changes() {
        let mut slot = None;
        assert!(!record(&mut slot, Some("#007AFF".into())));
        assert!(!record(&mut slot, Some("#007AFF".into())));
        assert!(record(&mut slot, Some("#8C8C8C".into())));
        assert!(record(&mut slot, None));
        assert!(!record(&mut slot, None));
        assert!(record(&mut slot, Some("#007AFF".into())));
    }
}
