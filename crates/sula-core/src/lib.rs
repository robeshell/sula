//! Sula's application core. Everything a UI shell needs — libraries, the task
//! queue, scraping, organizing, config, credentials and logs — without any
//! dependency on a particular shell (Tauri today, native shells later).

pub mod app;
pub mod config;
pub mod credentials;
pub mod files;
pub mod images;
pub mod items;
pub mod jobs;
pub mod libraries;
pub mod log_store;
pub mod media;
pub mod settings;
pub mod state;
pub mod task_queue;
pub mod ui_i18n;
