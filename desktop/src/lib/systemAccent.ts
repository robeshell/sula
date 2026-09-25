import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const HEX = /^#[0-9a-f]{6}$/i;

/** The interface uses the operating system's accent color; CSS falls back to blue. */
export function applySystemAccent(hex: string | null | undefined): void {
  const root = document.documentElement;
  if (hex && HEX.test(hex)) root.style.setProperty("--kg-accent-system", hex);
  else root.style.removeProperty("--kg-accent-system");
}

/** Read the accent once and follow changes pushed by the native layer. */
export function watchSystemAccent(): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void invoke<string | null>("get_system_accent")
    .then((hex) => {
      if (!disposed) applySystemAccent(hex);
    })
    .catch(() => {
      // Older native builds lack the command; the CSS default applies.
    });
  void listen<string | null>("system-accent-changed", (event) => {
    if (!disposed) applySystemAccent(event.payload);
  }).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
