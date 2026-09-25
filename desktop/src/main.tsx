import { MotionConfig } from "motion/react";
import { TooltipProvider } from "./components/ui/tooltip";
import { ConfirmationHost } from "./components/ConfirmationHost";
import React, { useEffect } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import App from "./App";
import { useAppStore, type Library } from "./store/appStore";
import { RenamerPage } from "./components/RenamerPage";
import { SettingsPage } from "./components/SettingsModal";
import { ToastHost } from "./components/ToastHost";
import { WindowControls } from "./components/WindowControls";
import { watchAppearance } from "./lib/appearance";
import { watchSystemAccent } from "./lib/systemAccent";
import {
  applyWindowClass,
  isImmersiveWindow,
  watchWindowClass,
} from "./lib/windowChrome";
import i18n from "./i18n";
import "./index.css";

document.documentElement.dataset.windowChrome = isImmersiveWindow()
  ? "immersive"
  : "native";
applyWindowClass();
// macOS overlay title bars put the traffic lights inside our toolbars.
if (navigator.userAgent.includes("Mac")) document.documentElement.dataset.platform = "mac";

type BootConfig = { appearance?: string; accent?: string; uiLocale?: string; configNotice?: string };

function ThemeBootstrap({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    const stopWindowClass = watchWindowClass();
    const stopAccent = watchSystemAccent();
    let stop = () => {};
    let disposed = false;
    let unlistenConfig: (() => void) | undefined;
    const apply = (config: BootConfig) => {
      stop();
      stop = watchAppearance(config.appearance ?? "system", config.accent ?? "indigo");
      if (config.uiLocale && config.uiLocale !== i18n.language) void i18n.changeLanguage(config.uiLocale);
    };
    void invoke<BootConfig>("get_config")
      .then((config) => {
        if (disposed) return;
        apply(config);
        if (config.configNotice) useAppStore.getState().showToast(i18n.t(config.configNotice));
      })
      .catch(() => {
        if (disposed) return;
        stop = watchAppearance("system", "indigo");
      });
    // Settings live in their own window; every window follows the saved config.
    void listen<BootConfig>("config-changed", (event) => {
      if (disposed) return;
      apply(event.payload);
      // e.g. saved API keys could not be read from the keychain after startup.
      if (event.payload.configNotice) useAppStore.getState().showToast(i18n.t(event.payload.configNotice));
    }).then((fn) => {
      if (disposed) fn();
      else unlistenConfig = fn;
    });
    return () => {
      disposed = true;
      stopWindowClass();
      stopAccent();
      unlistenConfig?.();
      stop();
    };
  }, []);
  return children;
}

/** Standalone settings window: keeps its own library list in step with the main window. */
function SettingsWindow() {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // Only the list is needed here; selectLibrary would also load media pages.
    const load = () =>
      invoke<Library[]>("list_libraries")
        .then((libraries) => { if (!disposed) useAppStore.setState({ libraries }); })
        .catch((err) => useAppStore.getState().showToast(String(err)));
    void load();
    void listen("library-updated", () => void load()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return (
    <div className="relative flex h-screen flex-col overflow-hidden">
      <div className="absolute right-0 top-0 z-30 h-[var(--kg-titlebar-height)]"><WindowControls /></div>
      <SettingsPage onClose={() => void getCurrentWindow().close()} />
      <ToastHost />
    </div>
  );
}

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);
const label = getCurrentWindow().label;

root.render(
  <React.StrictMode>
    <MotionConfig reducedMotion="user" transition={{ duration: 0.16, ease: "easeOut" }}>
    <TooltipProvider delayDuration={400}>
    <ThemeBootstrap>
      {label === "renamer" ? (
        <div className="flex h-screen flex-col overflow-hidden">
          <RenamerPage />
        </div>
      ) : label === "settings" ? (
        <SettingsWindow />
      ) : (
        <App />
      )}
      <ConfirmationHost />
    </ThemeBootstrap>
    </TooltipProvider>
    </MotionConfig>
  </React.StrictMode>,
);
