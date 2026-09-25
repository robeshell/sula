import { confirmAction } from "../lib/confirmation";
import { NativeSelect } from "./ui/native-select";
import { Switch } from "./ui/switch";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { invalidatePosterCache } from "../lib/posterLoadQueue";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Database, FolderTree, Library as LibraryIcon, SlidersHorizontal, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { migrateSkinPreference, watchAppearance } from "../lib/appearance";
import { useAppStore, type Library, type MediaType } from "../store/appStore";

/** In-app settings page. */
type PrefTab = "general" | "library" | "sources" | "rename" | "advanced";

const PREF_TABS: { id: PrefTab; icon: typeof SlidersHorizontal; label: string }[] = [
  { id: "general", icon: SlidersHorizontal, label: "settings.tab.general" },
  { id: "library", icon: LibraryIcon, label: "settings.section.library" },
  { id: "sources", icon: Database, label: "settings.tab.sources" },
  { id: "rename", icon: FolderTree, label: "settings.tab.rename" },
  { id: "advanced", icon: Wrench, label: "settings.tab.advanced" },
];

/** Preferences, laid out like a native settings window: icon tabs over grouped rows. */
export function SettingsPage({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<PrefTab>("general");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const current = PREF_TABS.find((p) => p.id === tab)!;
  return (
    <div className="sl-prefs">
      <header className="sl-prefs-top">
        <div data-tauri-drag-region />
        <div className="sl-prefs-title">{t(current.label)}</div>
        <div className="sl-prefs-tabs" role="tablist" aria-label={t("settings.title")}>
          {PREF_TABS.map(({ id, icon: Icon, label }) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className="sl-prefs-tab" onClick={() => setTab(id)}>
              <Icon aria-hidden />
              <span>{t(label)}</span>
            </button>
          ))}
        </div>
      </header>
      <div className="sl-prefs-body" role="tabpanel">
        {tab === "general" ? (
          <>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.section.appearance")}</p>
              <div className="kg-settings-group">
                <AppearanceBlock />
                <LanguageRow />
              </div>
            </section>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.section.windowBackground")}</p>
              <div className="kg-settings-group">
                <TrayRow />
                <KeepRunningOnCloseRow />
              </div>
            </section>
          </>
        ) : tab === "library" ? (
          <>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.section.library")}</p>
              <LibrarySection />
            </section>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.block.exclusions")}</p>
              <ScrapeExclusionsSection />
            </section>
          </>
        ) : tab === "sources" ? (
          <>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.section.api")}</p>
              <ApiKeysSection />
            </section>
            <section className="kg-settings-section">
              <p className="kg-section-label">{t("settings.block.nfo")}</p>
              <NfoSection />
            </section>
          </>
        ) : tab === "rename" ? (
          <section className="kg-settings-section">
            <p className="kg-section-label">{t("settings.section.rename")}</p>
            <RenameSection />
          </section>
        ) : (
          <section className="kg-settings-section">
            <p className="kg-section-label">{t("settings.section.cache")}</p>
            <div className="kg-settings-group">
              <CacheRow />
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

/** @deprecated alias — prefer SettingsPage */
export const SettingsModal = SettingsPage;

function KgSwitch({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  return <Switch aria-label={label} checked={checked} disabled={disabled} onCheckedChange={onChange} />;
}

function SettingsRow({
  title,
  subtitle,
  trailing,
}: {
  title: string;
  subtitle?: string;
  trailing: ReactNode;
}) {
  return (
    <div className="kg-settings-row">
      <div className="kg-settings-row-text">
        <p className="kg-settings-row-title">{title}</p>
        {subtitle ? <p className="kg-settings-row-sub">{subtitle}</p> : null}
      </div>
      {trailing}
    </div>
  );
}

type ThemeChoice = "system" | "default" | "deep-night";

function AppearanceBlock() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [theme, setTheme] = useState<ThemeChoice>("system");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ appearance?: string }>("get_config");
      const pref = migrateSkinPreference(config.appearance);
      setTheme(pref === "pure" ? "default" : pref);
    })();
  }, []);

  const persist = async (next: ThemeChoice) => {
    const previous = theme;
    setTheme(next);
    setSaving(true);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", { config: { ...config, appearance: next } });
      watchAppearance(next);
    } catch (err) {
      setTheme(previous);
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  const choices: { id: ThemeChoice; label: string }[] = [
    { id: "system", label: t("settings.appearance.system") },
    { id: "default", label: t("settings.appearance.light") },
    { id: "deep-night", label: t("settings.appearance.dark") },
  ];

  return (
    <>
      <div className="kg-settings-row">
        <div className="sl-themes" role="radiogroup" aria-label={t("settings.appearance.mode")}>
          {choices.map((opt) => (
            <button key={opt.id} type="button" role="radio" aria-checked={theme === opt.id} className="sl-theme"
              disabled={saving} onClick={() => void persist(opt.id)}>
              <span className="sl-theme-tile" data-theme={opt.id} aria-hidden><i /><i /><i /></span>
              {opt.label}
            </button>
          ))}
        </div>
      </div>
      <SettingsRow
        title={t("settings.appearance.accent")}
        subtitle={t("settings.appearance.accentSystem")}
        trailing={<span className="sl-accent-note"><i aria-hidden />{t("settings.appearance.followSystem")}</span>}
      />
    </>
  );
}

function LanguageRow() {
  const { t, i18n } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [locale, setLocale] = useState("zh-Hans");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ uiLocale?: string }>("get_config");
      const next = config.uiLocale ?? i18n.language;
      setLocale(next.startsWith("zh") ? "zh-Hans" : next);
    })();
  }, [i18n.language]);

  const save = async (next: string) => {
    setLocale(next);
    setSaving(true);
    try {
      await i18n.changeLanguage(next);
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: {
          ...config,
          uiLocale: next,
        },
      });
      showToast(t("settings.language.saved"));
    } catch (err) {
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SettingsRow
      title={t("settings.language.label")}
      trailing={
        <NativeSelect
          value={locale}
          disabled={saving}
          onChange={(e) => void save(e.target.value)}
        >
          <option value="zh-Hans">{t("lang.zh")}</option>
          <option value="en">{t("lang.en")}</option>
          <option value="ja">{t("lang.ja")}</option>
        </NativeSelect>
      }
    />
  );
}

function CacheRow() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [clearing, setClearing] = useState(false);

  const clearThumbs = async () => {
    setClearing(true);
    try {
      const n = await invoke<number>("clear_thumbnail_cache");
      invalidatePosterCache();
      showToast(t("settings.cache.cleared", { count: n }));
    } catch (err) {
      showToast(String(err));
    } finally {
      setClearing(false);
    }
  };

  return (
    <SettingsRow
      title={t("settings.cache.thumbs")}
      trailing={
        <Button variant="ghost" size="sm"
          type="button"
          className="shrink-0"
          disabled={clearing}
          onClick={() => void clearThumbs()}
        >
          {t("settings.cache.clear")}
        </Button>
      }
    />
  );
}

function TrayRow() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [trayEnabled, setTrayEnabled] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ trayEnabled?: boolean }>("get_config");
      setTrayEnabled(config.trayEnabled ?? true);
    })();
  }, []);

  const save = async (next: boolean) => {
    setTrayEnabled(next);
    setSaving(true);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: {
          ...config,
          trayEnabled: next,
        },
      });
      showToast(t("settings.tray.saved"));
    } catch (err) {
      setTrayEnabled(!next);
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SettingsRow
      title={t("settings.tray.enabled")}
      trailing={
        <KgSwitch
          label={t("settings.tray.enabled")}
          checked={trayEnabled}
          disabled={saving}
          onChange={(v) => void save(v)}
        />
      }
    />
  );
}

function KeepRunningOnCloseRow() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [keepRunning, setKeepRunning] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ keepRunningOnClose?: boolean }>("get_config");
      setKeepRunning(config.keepRunningOnClose ?? true);
    })();
  }, []);

  const save = async (next: boolean) => {
    setKeepRunning(next);
    setSaving(true);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: {
          ...config,
          keepRunningOnClose: next,
        },
      });
      showToast(t("settings.background.saved"));
    } catch (err) {
      setKeepRunning(!next);
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SettingsRow
      title={t("settings.background.keepRunning")}
      subtitle={t("settings.background.keepRunningHint")}
      trailing={
        <KgSwitch
          label={t("settings.background.keepRunning")}
          checked={keepRunning}
          disabled={saving}
          onChange={(v) => void save(v)}
        />
      }
    />
  );
}

/**
 * Debounced autosave for a settings form. Saves only after the user changed
 * something (the loaded values are the baseline), and a pending edit is written
 * immediately when the section unmounts (tab switch, Esc) instead of dropped.
 */
function useAutosave<T>(
  value: T,
  loaded: boolean,
  save: (value: T) => Promise<void>,
  delayMs = 450,
) {
  const saveRef = useRef(save);
  saveRef.current = save;
  const lastSaved = useRef<string | null>(null);
  const pending = useRef<{ value: T; key: string } | null>(null);
  const timer = useRef<number | null>(null);
  const key = JSON.stringify(value);

  const flush = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const next = pending.current;
    if (!next) return;
    pending.current = null;
    const previous = lastSaved.current;
    lastSaved.current = next.key;
    void saveRef.current(next.value).catch(() => {
      // Still dirty relative to what is on disk: the next edit retries.
      if (lastSaved.current === next.key) lastSaved.current = previous;
    });
  };

  useEffect(() => {
    if (!loaded) return;
    if (lastSaved.current === null) {
      lastSaved.current = key; // baseline: opening the page is not an edit
      return;
    }
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    if (key === lastSaved.current) {
      pending.current = null;
      return;
    }
    pending.current = { value, key };
    timer.current = window.setTimeout(flush, delayMs);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` encodes `value`
  }, [loaded, key, delayMs]);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => () => flush(), []);
}

const DEFAULT_TEMPLATES = {
  renameMovieFolderTemplate: "{title} ({year})",
  renameMovieFileTemplate: "{title} ({year})",
  renameTvShowFolderTemplate: "{title} ({year})",
  renameSeasonFolderTemplate: "Season {season:02}",
  renameEpisodeFileTemplate: "{title} - S{season:02}E{episode:02} - {episodeTitle}",
};

function RenameSection() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [autoRename, setAutoRename] = useState(false);
  const [createSeasons, setCreateSeasons] = useState(false);
  const [movieFolder, setMovieFolder] = useState(DEFAULT_TEMPLATES.renameMovieFolderTemplate);
  const [movieFile, setMovieFile] = useState(DEFAULT_TEMPLATES.renameMovieFileTemplate);
  const [tvFolder, setTvFolder] = useState(DEFAULT_TEMPLATES.renameTvShowFolderTemplate);
  const [seasonFolder, setSeasonFolder] = useState(DEFAULT_TEMPLATES.renameSeasonFolderTemplate);
  const [episodeFile, setEpisodeFile] = useState(DEFAULT_TEMPLATES.renameEpisodeFileTemplate);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{
        renameAutoAfterScrape: boolean;
        renameCreateSeasonFolders?: boolean;
        renameMovieFolderTemplate?: string;
        renameMovieFileTemplate?: string;
        renameTvShowFolderTemplate?: string;
        renameSeasonFolderTemplate?: string;
        renameEpisodeFileTemplate?: string;
      }>("get_config");
      setAutoRename(config.renameAutoAfterScrape ?? false);
      setCreateSeasons(config.renameCreateSeasonFolders ?? false);
      setMovieFolder(config.renameMovieFolderTemplate ?? DEFAULT_TEMPLATES.renameMovieFolderTemplate);
      setMovieFile(config.renameMovieFileTemplate ?? DEFAULT_TEMPLATES.renameMovieFileTemplate);
      setTvFolder(config.renameTvShowFolderTemplate ?? DEFAULT_TEMPLATES.renameTvShowFolderTemplate);
      setSeasonFolder(
        config.renameSeasonFolderTemplate ?? DEFAULT_TEMPLATES.renameSeasonFolderTemplate,
      );
      setEpisodeFile(
        config.renameEpisodeFileTemplate ?? DEFAULT_TEMPLATES.renameEpisodeFileTemplate,
      );
      setLoaded(true);
    })();
  }, []);

  useAutosave(
    {
      renameAutoAfterScrape: autoRename,
      renameCreateSeasonFolders: createSeasons,
      renameMovieFolderTemplate: movieFolder,
      renameMovieFileTemplate: movieFile,
      renameTvShowFolderTemplate: tvFolder,
      renameSeasonFolderTemplate: seasonFolder,
      renameEpisodeFileTemplate: episodeFile,
    },
    loaded,
    async (changes) => {
      try {
        const config = await invoke<Record<string, unknown>>("get_config");
        await invoke("save_config", { config: { ...config, ...changes } });
        showToast(t("settings.rename.saved"));
      } catch (err) {
        showToast(String(err));
        throw err;
      }
    },
  );

  const resetDefaults = () => {
    setMovieFolder(DEFAULT_TEMPLATES.renameMovieFolderTemplate);
    setMovieFile(DEFAULT_TEMPLATES.renameMovieFileTemplate);
    setTvFolder(DEFAULT_TEMPLATES.renameTvShowFolderTemplate);
    setSeasonFolder(DEFAULT_TEMPLATES.renameSeasonFolderTemplate);
    setEpisodeFile(DEFAULT_TEMPLATES.renameEpisodeFileTemplate);
  };

  return (
    <>
      <div className="kg-settings-group">
        <SettingsRow
          title={t("settings.rename.auto")}
          trailing={
            <KgSwitch
              label={t("settings.rename.auto")}
              checked={autoRename}
              onChange={setAutoRename}
            />
          }
        />
        <SettingsRow
          title={t("settings.rename.createSeasons")}
          subtitle={t("settings.rename.createSeasonsHint")}
          trailing={
            <KgSwitch
              label={t("settings.rename.createSeasons")}
              checked={createSeasons}
              onChange={setCreateSeasons}
            />
          }
        />
        <TemplateField
          label={t("settings.rename.movieFolder")}
          value={movieFolder}
          onChange={setMovieFolder}
        />
        <TemplateField
          label={t("settings.rename.movieFile")}
          value={movieFile}
          onChange={setMovieFile}
        />
        <TemplateField
          label={t("settings.rename.tvFolder")}
          value={tvFolder}
          onChange={setTvFolder}
        />
        <TemplateField
          label={t("settings.rename.seasonFolder")}
          value={seasonFolder}
          onChange={setSeasonFolder}
        />
        <TemplateField
          label={t("settings.rename.episodeFile")}
          value={episodeFile}
          onChange={setEpisodeFile}
        />
      </div>
      <div className="kg-settings-actions">
        <Button variant="ghost" size="sm" type="button" onClick={resetDefaults}>
          {t("settings.rename.reset")}
        </Button>
      </div>
    </>
  );
}

function TemplateField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <label className="kg-settings-field">
      <span className="kg-settings-block-label">{label}</span>
      <Input
        className="font-mono"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}

function ApiKeysSection() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [tmdb, setTmdb] = useState("");
  const [bangumi, setBangumi] = useState("");
  const [omdb, setOmdb] = useState("");
  const [tvdb, setTvdb] = useState("");
  const [concurrency, setConcurrency] = useState(4);
  const [language, setLanguage] = useState("zh-CN");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{
        apiKeys: { tmdb: string; bangumi?: string; omdb?: string; tvdb?: string };
        scrapeConcurrency: number;
        metadataLanguage: string;
      }>("get_config");
      setTmdb(config.apiKeys.tmdb ?? "");
      setBangumi(config.apiKeys.bangumi ?? "");
      setOmdb(config.apiKeys.omdb ?? "");
      setTvdb(config.apiKeys.tvdb ?? "");
      setConcurrency(config.scrapeConcurrency ?? 4);
      setLanguage(config.metadataLanguage ?? "zh-CN");
      setLoaded(true);
    })();
  }, []);

  useAutosave(
    { tmdb, bangumi, omdb, tvdb, concurrency, language },
    loaded,
    async (v) => {
      try {
        const config = await invoke<Record<string, unknown>>("get_config");
        const apiKeys = {
          ...((config.apiKeys as Record<string, string>) ?? {}),
          tmdb: v.tmdb,
          bangumi: v.bangumi,
          omdb: v.omdb,
          tvdb: v.tvdb,
        };
        await invoke("save_config", {
          config: {
            ...config,
            apiKeys,
            scrapeConcurrency: Math.min(8, Math.max(1, v.concurrency)),
            metadataLanguage: v.language,
          },
        });
        showToast(t("settings.api.saved"));
      } catch (err) {
        showToast(String(err));
        throw err;
      }
    },
  );

  return (
    <div className="kg-settings-group">
      <ApiField label={t("settings.api.tmdb")} value={tmdb} onChange={setTmdb} placeholder="eyJhbGciOi..." />
      <ApiField
        label={t("settings.api.bangumi")}
        value={bangumi}
        onChange={setBangumi}
        placeholder={t("settings.api.bangumiPlaceholder")}
      />
      <ApiField
        label={t("settings.api.omdb")}
        value={omdb}
        onChange={setOmdb}
        placeholder={t("settings.api.omdbPlaceholder")}
      />
      <ApiField
        label={t("settings.api.tvdb")}
        value={tvdb}
        onChange={setTvdb}
        placeholder={t("settings.api.tvdbPlaceholder")}
      />
      <label className="kg-settings-field">
        <span className="kg-settings-block-label">{t("settings.api.concurrency")}</span>
        <Input
          type="number"
          min={1}
          max={8}
          value={concurrency}
          onChange={(e) => setConcurrency(Number(e.target.value) || 4)}
        />
      </label>
      <label className="kg-settings-field">
        <span className="kg-settings-block-label">{t("settings.api.language")}</span>
        <Input
          value={language}
          onChange={(e) => setLanguage(e.target.value)}
        />
      </label>
    </div>
  );
}

function ApiField({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <label className="kg-settings-field">
      <span className="kg-settings-block-label">{label}</span>
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
      />
    </label>
  );
}

function NfoSection() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [nfoFormat, setNfoFormat] = useState("kodi");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ nfoFormat?: string }>("get_config");
      setNfoFormat(config.nfoFormat === "emby" ? "emby" : "kodi");
    })();
  }, []);

  const save = async (next: string) => {
    setNfoFormat(next);
    setSaving(true);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: {
          ...config,
          nfoFormat: next,
        },
      });
      showToast(t("settings.nfo.saved"));
    } catch (err) {
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="kg-settings-group">
      <SettingsRow
        title={t("settings.nfoFormat")}
        trailing={
          <NativeSelect
            value={nfoFormat}
            disabled={saving}
            onChange={(e) => void save(e.target.value)}
          >
            <option value="kodi">{t("settings.nfo.kodi")}</option>
            <option value="emby">{t("settings.nfo.emby")}</option>
          </NativeSelect>
        }
      />
    </div>
  );
}

function LibrarySection() {
  const { t } = useTranslation();
  const libraries = useAppStore((s) => s.libraries);
  const addLibrary = useAppStore((s) => s.addLibrary);
  const refreshLibraries = useAppStore((s) => s.refreshLibraries);
  const [mediaType, setMediaType] = useState<MediaType>("movie");
  const filtered = libraries.filter((l) => l.mediaType === mediaType);
  const typeLabel = (type: MediaType) =>
    type === "movie" ? t("type.movie") : type === "tvShow" ? t("type.tvShow") : t("type.anime");

  return (
    <>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <div className="kg-chip-strip">
          {(["movie", "tvShow", "anime"] as MediaType[]).map((type) => (
            <Button variant="plain" size="none"
              key={type}
              type="button"
              data-selected={mediaType === type}
              onClick={() => setMediaType(type)}
              className="kg-chip"
            >
              {typeLabel(type)}
            </Button>
          ))}
        </div>
        <Button variant="default" size="sm" type="button" onClick={() => void addLibrary(mediaType)} className="ml-auto">
          {t("action.addLibrary")}
        </Button>
      </div>

      <div className="kg-settings-group">
        {filtered.length === 0 ? (
          <div className="px-3.5 py-8 text-center kg-type-body-secondary text-fg-muted">
            {t("settings.library.empty")}
          </div>
        ) : (
          filtered.map((lib) => (
            <LibraryRow key={lib.id} library={lib} onChanged={() => void refreshLibraries()} />
          ))
        )}
      </div>
    </>
  );
}

function LibraryRow({
  library,
  onChanged,
}: {
  library: Library;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(library.name);
  const [rootExists, setRootExists] = useState(true);
  const showToast = useAppStore((s) => s.showToast);

  useEffect(() => {
    void (async () => {
      try {
        const ok = await invoke<boolean>("path_is_dir", { path: library.rootPath });
        setRootExists(ok);
      } catch {
        setRootExists(true);
      }
    })();
  }, [library.rootPath]);

  const rename = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    await invoke("rename_library", { id: library.id, name: trimmed });
    setRenaming(false);
    onChanged();
  };

  const remove = async () => {
    if (!(await confirmAction({ title: t("action.deleteLibrary"), description: t("settings.library.removeConfirm", { name: library.name }) }))) return;
    await invoke("delete_library", { id: library.id });
    onChanged();
  };

  const rebind = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("settings.library.rebindTitle"),
      });
      if (!selected || Array.isArray(selected)) return;
      await invoke("rebind_library", { id: library.id, rootPath: selected });
      showToast(t("settings.library.rebindDone"));
      onChanged();
    } catch (err) {
      showToast(String(err));
    }
  };

  return (
    <div className="kg-settings-row" style={{ minHeight: 64 }}>
      {renaming ? (
        <div className="flex w-full items-center gap-2 py-1">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="flex-1"
          />
          <Button variant="default" size="sm" type="button" onClick={() => void rename()}>
            {t("common.save")}
          </Button>
          <Button variant="ghost" size="sm" type="button" onClick={() => setRenaming(false)}>
            {t("common.cancel")}
          </Button>
        </div>
      ) : (
        <>
          <div className="kg-settings-row-text">
            <p className="kg-settings-row-title truncate">{library.name}</p>
            <p className="kg-settings-row-sub truncate font-mono">{library.rootPath}</p>
            {!rootExists ? (
              <p className="mt-1 kg-type-caption font-semibold text-error">
                {t("settings.library.pathMissing")}
              </p>
            ) : null}
          </div>
          <div className="flex shrink-0 gap-1">
            {!rootExists ? (
              <Button variant="default" size="sm" type="button" onClick={() => void rebind()}>
                {t("settings.library.rebind")}
              </Button>
            ) : null}
            <Button variant="ghost" size="sm" type="button" onClick={() => setRenaming(true)}>
              {t("common.rename")}
            </Button>
            <Button variant="destructive" size="sm" type="button" onClick={() => void remove()}>
              {t("common.remove")}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

function ScrapeExclusionsSection() {
  const { t } = useTranslation();
  const [folders, setFolders] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  /** Source of truth between renders so quick successive edits build on each other. */
  const foldersRef = useRef<string[]>([]);
  const saveQueue = useRef<Promise<void>>(Promise.resolve());
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ scanExcludedFolders: string[] }>("get_config");
      foldersRef.current = config.scanExcludedFolders ?? [];
      setFolders(foldersRef.current);
      setLoaded(true);
    })();
  }, []);

  const update = (next: string[]) => {
    foldersRef.current = next;
    setFolders(next);
    setMessage(null);
    // Saves run in order; each writes the full list as of its edit.
    saveQueue.current = saveQueue.current.then(async () => {
      try {
        const config = await invoke<Record<string, unknown>>("get_config");
        await invoke("save_config", {
          config: { ...config, scanExcludedFolders: next },
        });
        setMessage(t("settings.exclusions.saved"));
      } catch (err) {
        setMessage(String(err));
      }
    });
  };

  const add = () => {
    const value = draft.trim();
    // Before the list loads, an edit would overwrite the saved folders with [].
    if (!value || !loaded) return;
    setDraft("");
    const current = foldersRef.current;
    if (current.some((f) => f.toLowerCase() === value.toLowerCase())) return;
    update([...current, value]);
  };

  const remove = (name: string) => {
    update(foldersRef.current.filter((f) => f !== name));
  };

  return (
    <>
      <p className="mb-2 px-1 kg-type-caption leading-[1.45] text-fg-secondary">
        {t("settings.exclusions.hint")}
      </p>
      <div className="kg-settings-group">
        {folders.length === 0 ? (
          <div className="px-3.5 py-6 text-center kg-type-body-secondary text-fg-muted">
            {t("settings.exclusions.empty")}
          </div>
        ) : (
          folders.map((name) => (
            <div key={name} className="kg-settings-row" style={{ minHeight: 54 }}>
              <span className="font-mono kg-type-body-secondary text-fg">{name}</span>
              <Button variant="destructive" size="sm" type="button" onClick={() => remove(name)}>
                {t("common.remove")}
              </Button>
            </div>
          ))
        )}
      </div>
      <div className="mt-3 flex gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
          }}
          placeholder={t("settings.exclusions.placeholder")}
          className="flex-1"
        />
        <Button variant="default" size="sm" type="button" disabled={!loaded || !draft.trim()} onClick={add}>
          {t("common.add")}
        </Button>
      </div>
      {message ? <p className="mt-2 kg-type-caption text-fg-secondary">{message}</p> : null}
    </>
  );
}
