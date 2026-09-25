import { confirmAction } from "../lib/confirmation";
import { motion } from "motion/react";
import { NativeSelect } from "./ui/native-select";
import { Switch } from "./ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./ui/tabs";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { invalidatePosterCache } from "../lib/posterLoadQueue";
import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import {
  ACCENT_PRESETS,
  migrateAccent,
  migrateSkinPreference,
  watchAppearance,
  type AccentId,
  type SkinPreference,
} from "../lib/appearance";
import { useAppStore, type Library, type MediaType } from "../store/appStore";

/** In-app settings page. */
export function SettingsPage({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <motion.div initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="kg-settings-page min-h-0 flex-1 overflow-auto">
      <Tabs defaultValue="appearance" className="kg-page-shell w-full">
        <header className="kg-page-header">
          <div className="min-w-0 flex-1">
            <h2 className="kg-page-header-title">{t("settings.title")}</h2>
          </div>
        </header>

        <TabsList className="mb-6 h-auto flex-wrap justify-start">
          {[["appearance", "appearanceLang"], ["library", "library"], ["scrape", "scrape"], ["rename", "rename"], ["system", "system"]].map(([value, label]) => <TabsTrigger key={value} value={value}>{t(`settings.section.${label}`)}</TabsTrigger>)}
        </TabsList>

        <TabsContent value="appearance" className="kg-settings-section mt-0">
          <p className="kg-section-label">{t("settings.section.appearanceLang")}</p>
          <div className="kg-settings-group">
            <AppearanceBlock />
            <LanguageRow />
          </div>
        </TabsContent>

        <TabsContent value="library" className="kg-settings-section mt-0">
          <p className="kg-section-label">{t("settings.section.library")}</p>
          <LibrarySection />
          <p className="kg-section-label mt-3">{t("settings.block.exclusions")}</p>
          <ScrapeExclusionsSection />
        </TabsContent>

        <TabsContent value="scrape" className="kg-settings-section mt-0">
          <p className="kg-section-label">{t("settings.section.scrape")}</p>
          <ApiKeysSection />
          <p className="kg-section-label mt-3">{t("settings.block.nfo")}</p>
          <NfoSection />
        </TabsContent>

        <TabsContent value="rename" className="kg-settings-section mt-0">
          <p className="kg-section-label">{t("settings.section.rename")}</p>
          <RenameSection />
        </TabsContent>

        <TabsContent value="system" className="kg-settings-section mt-0">
          <p className="kg-section-label">{t("settings.section.system")}</p>
          <div className="kg-settings-group">
            <KeepRunningOnCloseRow />
            <TrayRow />
            <CacheRow />
          </div>
        </TabsContent>
      </Tabs>
    </motion.div>
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

function AppearanceBlock() {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const [skin, setSkin] = useState<SkinPreference>("system");
  const [accent, setAccent] = useState<AccentId>("indigo");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ appearance?: string; accent?: string }>("get_config");
      setSkin(migrateSkinPreference(config.appearance));
      setAccent(migrateAccent(config.accent));
    })();
  }, []);

  const persist = async (nextSkin: SkinPreference, nextAccent: AccentId) => {
    setSkin(nextSkin);
    setAccent(nextAccent);
    setSaving(true);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: {
          ...config,
          appearance: nextSkin,
          accent: nextAccent,
        },
      });
      watchAppearance(nextSkin, nextAccent);
      showToast(t("settings.appearance.saved"));
    } catch (err) {
      showToast(String(err));
    } finally {
      setSaving(false);
    }
  };

  const skins: { id: SkinPreference; label: string }[] = [
    { id: "system", label: t("settings.appearance.system") },
    { id: "default", label: t("settings.appearance.skinDefault") },
    { id: "pure", label: t("settings.appearance.skinPure") },
    { id: "deep-night", label: t("settings.appearance.skinNight") },
  ];

  return (
    <>
      <div>
        <p className="kg-settings-block-label">{t("settings.appearance.skin")}</p>
        <div className="kg-skin-strip">
          {skins.map((opt) => (
            <SkinCard
              key={opt.id}
              id={opt.id}
              label={opt.label}
              selected={skin === opt.id}
              disabled={saving}
              onSelect={() => void persist(opt.id, accent)}
            />
          ))}
        </div>
      </div>
      <div>
        <p className="kg-settings-block-label">{t("settings.appearance.accent")}</p>
        <div className="kg-accent-strip" role="group" aria-label={t("settings.appearance.accent")}>
          {ACCENT_PRESETS.map((opt) => (
            <Button variant="plain" size="none"
              key={opt.id}
              type="button"
              className="kg-accent-swatch"
              style={{ ["--swatch" as string]: opt.color }}
              data-selected={accent === opt.id}
              aria-pressed={accent === opt.id}
              disabled={saving}
              aria-label={t(`settings.appearance.accent.${opt.id}`)}
              title={t(`settings.appearance.accent.${opt.id}`)}
              onClick={() => void persist(skin, opt.id)}
            />
          ))}
        </div>
      </div>
    </>
  );
}

function SkinCard({
  id,
  label,
  selected,
  disabled,
  onSelect,
}: {
  id: SkinPreference;
  label: string;
  selected: boolean;
  disabled?: boolean;
  onSelect: () => void;
}) {
  if (id === "system") {
    return (
      <Button variant="plain" size="none"
        type="button"
        className="kg-skin-card"
        data-variant="system"
        data-selected={selected}
      aria-pressed={selected}
        disabled={disabled}
        onClick={onSelect}
      >
        <div className="kg-skin-card-preview">
          <div
            className="kg-skin-card-half"
            style={
              {
                "--kg-skin-preview-canvas": "#f7f9fc",
                "--kg-skin-preview-elevated": "#ffffff",
                "--kg-skin-preview-border": "rgb(0 0 0 / 0.08)",
                "--kg-skin-preview-glass-border": "rgb(0 0 0 / 0.07)",
                "--kg-skin-preview-line": "rgb(28 28 34 / 0.22)",
                "--kg-skin-preview-line-muted": "rgb(90 90 98 / 0.32)",
                background: "#f7f9fc",
              } as CSSProperties
            }
          >
            <MiniChrome />
          </div>
          <div
            className="kg-skin-card-half"
            style={
              {
                "--kg-skin-preview-canvas": "#0d0d0f",
                "--kg-skin-preview-elevated": "#202024",
                "--kg-skin-preview-border": "rgb(255 255 255 / 0.1)",
                "--kg-skin-preview-glass-border": "rgb(255 255 255 / 0.11)",
                "--kg-skin-preview-line": "rgb(247 243 244 / 0.22)",
                "--kg-skin-preview-line-muted": "rgb(255 255 255 / 0.32)",
                background: "#0d0d0f",
              } as CSSProperties
            }
          >
            <MiniChrome />
          </div>
        </div>
        <span className="kg-skin-card-label">{label}</span>
      </Button>
    );
  }

  const preview =
    id === "pure"
      ? {
          canvas: "#f1f4f8",
          elevated: "#ffffff",
          border: "rgb(82 97 116 / 0.12)",
          glassBorder: "rgb(82 97 116 / 0.12)",
          line: "rgb(24 32 42 / 0.22)",
          lineMuted: "rgb(83 97 113 / 0.32)",
        }
      : id === "deep-night"
        ? {
            canvas: "#0d0d0f",
            elevated: "#202024",
            border: "rgb(255 255 255 / 0.1)",
            glassBorder: "rgb(255 255 255 / 0.11)",
            line: "rgb(247 243 244 / 0.22)",
            lineMuted: "rgb(255 255 255 / 0.32)",
          }
        : {
            // Application light theme preview.
            canvas: "#f7f9fc",
            elevated: "#ffffff",
            border: "rgb(0 0 0 / 0.08)",
            glassBorder: "rgb(0 0 0 / 0.07)",
            line: "rgb(28 28 34 / 0.22)",
            lineMuted: "rgb(90 90 98 / 0.32)",
          };

  return (
    <Button variant="plain" size="none"
      type="button"
      className="kg-skin-card"
      data-selected={selected}
      aria-pressed={selected}
      disabled={disabled}
      onClick={onSelect}
    >
      <div
        className="kg-skin-card-preview"
        style={
          {
            "--kg-skin-preview-canvas": preview.canvas,
            "--kg-skin-preview-elevated": preview.elevated,
            "--kg-skin-preview-border": preview.border,
            "--kg-skin-preview-glass-border": preview.glassBorder,
            "--kg-skin-preview-line": preview.line,
            "--kg-skin-preview-line-muted": preview.lineMuted,
            background: preview.canvas,
            borderColor: preview.border,
          } as CSSProperties
        }
      >
        <MiniChrome />
      </div>
      <span className="kg-skin-card-label">{label}</span>
    </Button>
  );
}

function MiniChrome() {
  return (
    <div className="kg-skin-card-mini">
      <span className="kg-skin-card-bar" />
      <span className="kg-skin-card-line long" />
      <span className="kg-skin-card-line short" />
    </div>
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

  useEffect(() => {
    if (!loaded) return;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const config = await invoke<Record<string, unknown>>("get_config");
          await invoke("save_config", {
            config: {
              ...config,
              renameAutoAfterScrape: autoRename,
              renameCreateSeasonFolders: createSeasons,
              renameMovieFolderTemplate: movieFolder,
              renameMovieFileTemplate: movieFile,
              renameTvShowFolderTemplate: tvFolder,
              renameSeasonFolderTemplate: seasonFolder,
              renameEpisodeFileTemplate: episodeFile,
            },
          });
          showToast(t("settings.rename.saved"));
        } catch (err) {
          showToast(String(err));
        }
      })();
    }, 450);
    return () => window.clearTimeout(timer);
  }, [
    loaded,
    autoRename,
    createSeasons,
    movieFolder,
    movieFile,
    tvFolder,
    seasonFolder,
    episodeFile,
    showToast,
    t,
  ]);

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

  useEffect(() => {
    if (!loaded) return;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const config = await invoke<Record<string, unknown>>("get_config");
          const apiKeys = {
            ...((config.apiKeys as Record<string, string>) ?? {}),
            tmdb,
            bangumi,
            omdb,
            tvdb,
          };
          await invoke("save_config", {
            config: {
              ...config,
              apiKeys,
              scrapeConcurrency: Math.min(8, Math.max(1, concurrency)),
              metadataLanguage: language,
            },
          });
          showToast(t("settings.api.saved"));
        } catch (err) {
          showToast(String(err));
        }
      })();
    }, 450);
    return () => window.clearTimeout(timer);
  }, [loaded, tmdb, bangumi, omdb, tvdb, concurrency, language, showToast, t]);

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
        placeholder="OMDb API Key"
      />
      <ApiField
        label={t("settings.api.tvdb")}
        value={tvdb}
        onChange={setTvdb}
        placeholder="TVDB API Key"
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
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      const config = await invoke<{ scanExcludedFolders: string[] }>("get_config");
      setFolders(config.scanExcludedFolders ?? []);
    })();
  }, []);

  const persist = async (next: string[]) => {
    setSaving(true);
    setMessage(null);
    try {
      const config = await invoke<Record<string, unknown>>("get_config");
      await invoke("save_config", {
        config: { ...config, scanExcludedFolders: next },
      });
      setFolders(next);
      setMessage(t("settings.exclusions.saved"));
    } catch (err) {
      setMessage(String(err));
    } finally {
      setSaving(false);
    }
  };

  const add = () => {
    const value = draft.trim();
    if (!value) return;
    if (folders.some((f) => f.toLowerCase() === value.toLowerCase())) {
      setDraft("");
      return;
    }
    void persist([...folders, value]);
    setDraft("");
  };

  const remove = (name: string) => {
    void persist(folders.filter((f) => f !== name));
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
        <Button variant="default" size="sm" type="button" disabled={saving} onClick={add}>
          {t("common.add")}
        </Button>
      </div>
      {message ? <p className="mt-2 kg-type-caption text-fg-secondary">{message}</p> : null}
    </>
  );
}
