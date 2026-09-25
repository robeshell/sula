use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub config_notice: Option<String>,
    pub scrape_concurrency: u8,
    pub metadata_language: String,
    pub nfo_format: String,
    pub scan_excluded_folders: Vec<String>,
    pub rename_auto_after_scrape: bool,
    pub rename_create_season_folders: bool,
    #[serde(default = "default_movie_folder_template")]
    pub rename_movie_folder_template: String,
    #[serde(default = "default_movie_file_template")]
    pub rename_movie_file_template: String,
    #[serde(default = "default_tv_show_folder_template")]
    pub rename_tv_show_folder_template: String,
    #[serde(default = "default_season_folder_template")]
    pub rename_season_folder_template: String,
    #[serde(default = "default_episode_file_template")]
    pub rename_episode_file_template: String,
    pub appearance: String,
    /// Accent axis id: indigo | teal | sky | slate (sula presets).
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_tray_enabled")]
    pub tray_enabled: bool,
    /// Keep the process alive and hide the main window when it is closed.
    #[serde(default = "default_keep_running_on_close")]
    pub keep_running_on_close: bool,
    /// UI language: zh-Hans | en | ja (I18N surface language).
    #[serde(default = "default_ui_locale")]
    pub ui_locale: String,
    pub api_keys: ApiKeysConfig,
}

fn default_tray_enabled() -> bool {
    true
}

fn default_keep_running_on_close() -> bool {
    true
}

fn default_accent() -> String {
    "indigo".into()
}

fn default_ui_locale() -> String {
    "zh-Hans".into()
}

fn default_movie_folder_template() -> String {
    renamer::TemplateEngine::MOVIE_FOLDER.into()
}
fn default_movie_file_template() -> String {
    renamer::TemplateEngine::MOVIE_FILE.into()
}
fn default_tv_show_folder_template() -> String {
    renamer::TemplateEngine::TV_SHOW_FOLDER.into()
}
fn default_season_folder_template() -> String {
    renamer::TemplateEngine::SEASON_FOLDER.into()
}
fn default_episode_file_template() -> String {
    renamer::TemplateEngine::EPISODE_FILE.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeysConfig {
    pub tmdb: String,
    pub tvdb: String,
    pub omdb: String,
    #[serde(default)]
    pub bangumi: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            config_notice: None,
            scrape_concurrency: 4,
            metadata_language: "zh-CN".into(),
            nfo_format: "kodi".into(),
            scan_excluded_folders: vec![
                "NCOP&NCED".into(),
                "PV".into(),
                "menu".into(),
                "SP".into(),
                "Extras".into(),
                "Specials".into(),
                ".actors".into(),
            ],
            rename_auto_after_scrape: false,
            rename_create_season_folders: false,
            rename_movie_folder_template: default_movie_folder_template(),
            rename_movie_file_template: default_movie_file_template(),
            rename_tv_show_folder_template: default_tv_show_folder_template(),
            rename_season_folder_template: default_season_folder_template(),
            rename_episode_file_template: default_episode_file_template(),
            appearance: "system".into(),
            accent: default_accent(),
            tray_enabled: true,
            keep_running_on_close: true,
            ui_locale: default_ui_locale(),
            api_keys: ApiKeysConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn keep_running_on_close_defaults_to_enabled() {
        assert!(AppConfig::default().keep_running_on_close);
    }

    #[test]
    fn legacy_config_without_close_preference_defaults_to_enabled() {
        let config: AppConfig = toml::from_str("").expect("empty config should use defaults");
        assert!(config.keep_running_on_close);
    }
}

impl AppConfig {
    pub fn rename_templates(&self) -> renamer::RenameTemplates {
        renamer::RenameTemplates {
            movie_folder: self.rename_movie_folder_template.clone(),
            movie_file: self.rename_movie_file_template.clone(),
            tv_show_folder: self.rename_tv_show_folder_template.clone(),
            season_folder: self.rename_season_folder_template.clone(),
            episode_file: self.rename_episode_file_template.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DiskConfig {
    #[serde(flatten)]
    config: AppConfig,
    #[serde(default)]
    credential_ref: Option<String>,
}

pub struct ConfigStore {
    path: PathBuf,
    credential_ref: Option<String>,
    /// Keychain read failed at load; `credential_ref` is kept until the user enters new keys.
    keys_unloaded: bool,
    pub config: AppConfig,
}

/// Reads only `uiLocale` so startup errors can be localized even when full loading fails.
pub fn peek_ui_locale(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    raw.parse::<toml::Table>().ok()?.get("uiLocale")?.as_str().map(str::to_owned)
}

fn load_api_keys(id: &str) -> anyhow::Result<ApiKeysConfig> {
    Ok(serde_json::from_str(&crate::credentials::get(id)?)?)
}

impl ConfigStore {
    pub fn load_or_default(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut credential_ref = None;
        let mut needs_save = false;
        let mut keys_unloaded = false;
        let config = match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<DiskConfig>(&raw) {
                Ok(mut disk) => {
                    credential_ref = disk.credential_ref;
                    if let Some(id) = &credential_ref {
                        match load_api_keys(id) {
                            Ok(keys) => disk.config.api_keys = keys,
                            Err(error) => {
                                tracing::warn!(%error, "stored API keys unavailable; continuing without them");
                                keys_unloaded = true;
                                disk.config.api_keys = ApiKeysConfig::default();
                                disk.config.config_notice = Some("err.credentialsUnavailable".into());
                            }
                        }
                    } else if disk.config.api_keys != ApiKeysConfig::default() { needs_save = true; }
                    disk.config
                }
                Err(error) => {
                    let backup = path.with_extension(format!("invalid-{}.toml", uuid::Uuid::new_v4()));
                    fs::rename(&path, &backup)?;
                    tracing::error!(%error, backup = %backup.display(), "invalid configuration preserved; using defaults");
                    needs_save = true;
                    AppConfig { config_notice: Some("err.configRecovered".into()), ..AppConfig::default() }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => { needs_save = true; AppConfig::default() },
            Err(e) => return Err(e.into()),
        };
        let mut store = Self { path, config, credential_ref, keys_unloaded };
        if needs_save { store.save()?; }
        Ok(store)
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() { fs::create_dir_all(parent)?; }
        // Keys that never loaded are still empty placeholders; keep the stored record untouched.
        let keep_ref = self.keys_unloaded && self.config.api_keys == ApiKeysConfig::default();
        let next_ref = if keep_ref { self.credential_ref.clone() } else if self.config.api_keys != ApiKeysConfig::default() {
            let id = uuid::Uuid::new_v4().to_string();
            crate::credentials::set(&id, &serde_json::to_string(&self.config.api_keys)?)?;
            Some(id)
        } else { None };
        let mut disk_config = self.config.clone();
        disk_config.api_keys = ApiKeysConfig::default();
        disk_config.config_notice = None;
        let result = (|| -> anyhow::Result<()> {
            let raw = toml::to_string_pretty(&DiskConfig { config: disk_config, credential_ref: next_ref.clone() })?;
            let tmp = self.path.with_extension("toml.tmp");
            use std::io::Write;
            let mut options = fs::OpenOptions::new();
            options.create(true).truncate(true).write(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            let mut file = options.open(&tmp)?;
            file.write_all(raw.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &self.path)?;
            Ok(())
        })();
        if let Err(error) = result {
            if !keep_ref { if let Some(id) = next_ref { crate::credentials::remove(&id); } }
            return Err(error);
        }
        if keep_ref { return Ok(()); }
        // The old record remains valid until the new file reference is committed.
        if let Some(old) = self.credential_ref.take() {
            crate::credentials::remove(&old);
        }
        self.credential_ref = next_ref;
        self.keys_unloaded = false;
        Ok(())
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!("sula-config-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap(); path
    }
    #[test]
    fn legacy_keys_migrate_and_roundtrip_without_plaintext_on_disk() {
        let dir = folder(); let path = dir.join("config.toml");
        let mut config = AppConfig::default(); config.api_keys.tmdb = "test-only-key".into();
        fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
        let mut store = ConfigStore::load_or_default(&path).unwrap();
        assert_eq!(store.config.api_keys.tmdb, "test-only-key");
        assert!(!fs::read_to_string(&path).unwrap().contains("test-only-key"));
        assert_eq!(ConfigStore::load_or_default(&path).unwrap().config.api_keys.tmdb, "test-only-key");
        let old = store.credential_ref.clone().unwrap();
        store.config.api_keys.tmdb = "replacement-test-key".into(); store.save().unwrap();
        assert!(crate::credentials::get(&old).is_err());
        assert_eq!(ConfigStore::load_or_default(&path).unwrap().config.api_keys.tmdb, "replacement-test-key");
        crate::credentials::remove(store.credential_ref.as_ref().unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_config_is_preserved_before_defaults_are_written() {
        let dir = folder(); let path = dir.join("config.toml");
        fs::write(&path, "invalid = [").unwrap();
        let store = ConfigStore::load_or_default(&path).unwrap();
        assert!(store.config.config_notice.is_some());
        let backups: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains("invalid-")).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "invalid = [");
        assert!(ConfigStore::load_or_default(&path).unwrap().config.config_notice.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_config_save_preserves_previous_disk_and_credential() {
        let dir = folder(); let path = dir.join("config.toml");
        let mut store = ConfigStore::load_or_default(&path).unwrap();
        store.config.api_keys.tmdb = "old-test-key".into(); store.save().unwrap();
        let original = fs::read(&path).unwrap(); let old = store.credential_ref.clone().unwrap();
        fs::create_dir(path.with_extension("toml.tmp")).unwrap();
        store.config.api_keys.tmdb = "new-test-key".into();
        assert!(store.save().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(store.credential_ref.as_ref(), Some(&old));
        assert_eq!(ConfigStore::load_or_default(&path).unwrap().config.api_keys.tmdb, "old-test-key");
        crate::credentials::remove(&old); fs::remove_dir_all(dir).unwrap();
    }
    fn dangling_ref_config(dir: &Path) -> (PathBuf, String) {
        let path = dir.join("config.toml"); let id = format!("missing-{}", uuid::Uuid::new_v4());
        let config = AppConfig { ui_locale: "en".into(), ..AppConfig::default() };
        fs::write(&path, toml::to_string(&DiskConfig { config, credential_ref: Some(id.clone()) }).unwrap()).unwrap();
        (path, id)
    }
    #[test]
    fn unavailable_credentials_degrade_without_touching_disk() {
        let dir = folder(); let (path, id) = dangling_ref_config(&dir);
        let original = fs::read(&path).unwrap();
        let store = ConfigStore::load_or_default(&path).unwrap();
        assert_eq!(store.config.api_keys, ApiKeysConfig::default());
        assert!(store.keys_unloaded);
        assert_eq!(store.credential_ref.as_deref(), Some(id.as_str()));
        assert_eq!(store.config.config_notice.as_deref(), Some("err.credentialsUnavailable"));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn saving_other_settings_keeps_unloaded_credential_ref() {
        let dir = folder(); let (path, id) = dangling_ref_config(&dir);
        let mut store = ConfigStore::load_or_default(&path).unwrap();
        store.config.scrape_concurrency = 2; store.save().unwrap();
        assert!(crate::credentials::get(&id).is_err());
        assert_eq!(store.credential_ref.as_deref(), Some(id.as_str()));
        let disk: DiskConfig = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(disk.credential_ref.as_deref(), Some(id.as_str()));
        assert_eq!(disk.config.scrape_concurrency, 2);
        assert!(disk.config.config_notice.is_none());
        // Keychain comes back on a later launch: the original keys are still reachable.
        crate::credentials::set(&id, &serde_json::to_string(&ApiKeysConfig { tmdb: "restored-test-key".into(), ..Default::default() }).unwrap()).unwrap();
        let reloaded = ConfigStore::load_or_default(&path).unwrap();
        assert_eq!(reloaded.config.api_keys.tmdb, "restored-test-key");
        assert!(!reloaded.keys_unloaded);
        crate::credentials::remove(&id); fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn entering_keys_after_unavailable_load_replaces_ref() {
        let dir = folder(); let (path, id) = dangling_ref_config(&dir);
        let mut store = ConfigStore::load_or_default(&path).unwrap();
        store.config.api_keys.tmdb = "fresh-test-key".into(); store.save().unwrap();
        let next = store.credential_ref.clone().unwrap();
        assert_ne!(next, id);
        assert!(!store.keys_unloaded);
        assert_eq!(ConfigStore::load_or_default(&path).unwrap().config.api_keys.tmdb, "fresh-test-key");
        crate::credentials::remove(&next); fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn peek_ui_locale_reads_camel_case_key() {
        let dir = folder(); let path = dir.join("config.toml");
        assert_eq!(peek_ui_locale(&path), None);
        fs::write(&path, "uiLocale = \"ja\"\nscrapeConcurrency = \"broken\"\n").unwrap();
        assert_eq!(peek_ui_locale(&path).as_deref(), Some("ja"));
        fs::remove_dir_all(dir).unwrap();
    }
}
