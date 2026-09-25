import { MotionConfig } from "motion/react";
import { TooltipProvider } from "./components/ui/tooltip";
import { ConfirmationHost } from "./components/ConfirmationHost";
import React, { useEffect } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import App from "./App";
import { useAppStore } from "./store/appStore";
import { RenamerPage } from "./components/RenamerPage";
import { watchAppearance } from "./lib/appearance";
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

function ThemeBootstrap({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    const stopWindowClass = watchWindowClass();
    let stop = () => {};
    let disposed = false;
    void invoke<{ appearance?: string; accent?: string; uiLocale?: string; configNotice?: string }>("get_config")
      .then((config) => {
        if (disposed) return;
        stop = watchAppearance(config.appearance ?? "system", config.accent ?? "indigo");
        if (config.uiLocale) { void i18n.changeLanguage(config.uiLocale); }
        if (config.configNotice) useAppStore.getState().showToast(i18n.t(config.configNotice));
      })
      .catch(() => {
        if (disposed) return;
        stop = watchAppearance("system", "indigo");
      });
    return () => {
      disposed = true;
      stopWindowClass();
      stop();
    };
  }, []);
  return children;
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
      ) : (
        <App />
      )}
      <ConfirmationHost />
    </ThemeBootstrap>
    </TooltipProvider>
    </MotionConfig>
  </React.StrictMode>,
);
