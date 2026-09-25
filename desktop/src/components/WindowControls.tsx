import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";

import { isImmersiveWindow } from "../lib/windowChrome";

export function WindowControls() {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const immersive = isImmersiveWindow();
    document.documentElement.dataset.windowChrome = immersive ? "immersive" : "native";
    setVisible(immersive);
  }, []);

  if (!visible) return null;

  const win = getCurrentWindow();

  return (
    <div className="kg-window-controls" role="group" aria-label={t("window.controls")}>
      <button
        type="button"
        className="kg-window-control"
        aria-label={t("window.minimize")}
        onClick={() => void win.minimize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M1 5h8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      </button>
      <button
        type="button"
        className="kg-window-control"
        aria-label={t("window.maximize")}
        onClick={() => void win.toggleMaximize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <rect
            x="1.25"
            y="1.25"
            width="7.5"
            height="7.5"
            rx="0.6"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
          />
        </svg>
      </button>
      <button
        type="button"
        className="kg-window-control kg-window-control-close"
        aria-label={t("window.close")}
        onClick={() => void win.close()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path
            d="M2 2l6 6M8 2L2 8"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}
