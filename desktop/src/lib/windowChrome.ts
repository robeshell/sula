/** Windows uses undecorated immersive chrome; macOS keeps Overlay + traffic lights. */
export function isImmersiveWindow(): boolean {
  return navigator.userAgent.includes("Windows");
}

/** Desktop window breakpoints: medium <1100, wide ≥1100. */
export type WindowClass = "medium" | "wide";

const WIDE_MIN_PX = 1100;

export function resolveWindowClass(width = window.innerWidth): WindowClass {
  return width < WIDE_MIN_PX ? "medium" : "wide";
}

/** Writes `data-window-class` so CSS layout rules (sidebar / gutter / title) update. */
export function applyWindowClass(width?: number): WindowClass {
  const cls = resolveWindowClass(width);
  document.documentElement.dataset.windowClass = cls;
  return cls;
}

export function watchWindowClass(): () => void {
  const update = () => {
    applyWindowClass();
  };
  update();
  window.addEventListener("resize", update);
  return () => window.removeEventListener("resize", update);
}
