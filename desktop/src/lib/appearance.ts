/** Application appearance preferences. */

export type SkinPreference = "system" | "default" | "pure" | "deep-night";
export type ResolvedSkin = "default" | "pure" | "deep-night";
export type AccentId = "indigo" | "teal" | "sky" | "slate";

/** @deprecated legacy appearance values — migrated on load */
type LegacyAppearance = "light" | "dark";

/** Accent choices maintained alongside styles/appearance.css. */
export const ACCENT_PRESETS: { id: AccentId; color: string }[] = [
  { id: "indigo", color: "#5A66B8" },
  { id: "teal", color: "#3F9E98" },
  { id: "sky", color: "#0177B5" },
  { id: "slate", color: "#475569" },
];

export function migrateSkinPreference(raw: string | undefined | null): SkinPreference {
  if (raw === "light") return "default";
  if (raw === "dark") return "deep-night";
  if (raw === "system" || raw === "default" || raw === "pure" || raw === "deep-night") {
    return raw;
  }
  return "system";
}

export function migrateAccent(raw: string | undefined | null): AccentId {
  if (raw === "indigo" || raw === "teal" || raw === "sky" || raw === "slate") return raw;
  return "indigo";
}

export function resolveSkin(pref: SkinPreference): ResolvedSkin {
  if (pref === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "deep-night"
      : "default";
  }
  return pref;
}

export function applyAccent(accent: string): AccentId {
  const id = migrateAccent(accent);
  document.documentElement.setAttribute("data-accent", id);
  return id;
}

export function applyAppearance(appearance: string): SkinPreference {
  const pref = migrateSkinPreference(appearance);
  const resolved = resolveSkin(pref);
  const root = document.documentElement;
  root.setAttribute("data-skin", resolved);
  root.style.colorScheme = resolved === "deep-night" ? "dark" : "light";
  return pref;
}

export function applyTheme(appearance: string, accent: string): {
  skin: SkinPreference;
  accent: AccentId;
} {
  return {
    skin: applyAppearance(appearance),
    accent: applyAccent(accent),
  };
}

let mediaListener: ((e: MediaQueryListEvent) => void) | null = null;
let mediaQuery: MediaQueryList | null = null;
let lastAppearance = "system";
let lastAccent = "indigo";

/** Keep system mode in sync with OS theme changes. */
export function watchAppearance(appearance: string, accent?: string): () => void {
  lastAppearance = appearance;
  if (accent !== undefined) lastAccent = accent;
  applyTheme(lastAppearance, lastAccent);

  if (mediaListener && mediaQuery) {
    mediaQuery.removeEventListener("change", mediaListener);
    mediaListener = null;
    mediaQuery = null;
  }

  const pref = migrateSkinPreference(lastAppearance);
  if (pref !== "system") {
    return () => {};
  }

  mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaListener = () => {
    applyTheme(lastAppearance, lastAccent);
  };
  mediaQuery.addEventListener("change", mediaListener);
  const subscribedQuery = mediaQuery;
  const subscribedListener = mediaListener;
  return () => {
    subscribedQuery.removeEventListener("change", subscribedListener);
    if (mediaListener === subscribedListener) {
      mediaListener = null;
      mediaQuery = null;
    }
  };
}

export type { LegacyAppearance };
