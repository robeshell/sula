//! Renamer — template engine (M3) + media rename (M3) + rule pipeline (M5).

mod execute;
mod media_rename;
mod preset;
mod preview;
mod rules;
mod template;

pub use execute::{execute, CompletedRename, ExecuteError, RenameSnapshot, RenameUndoManager};
pub use media_rename::{
    consolidate_library_duplicate_shows, consolidate_show_item, organize_season_folders,
    rename_after_scrape, rename_after_scrape_with_options, RenameError, RenameTemplates,
};
pub use preset::{PresetError, PresetManager};
pub use preview::{preview, FileEntry, PreviewResult};
pub use rules::{
    AnyRenameRule, AutoNumbering, BracketType, CaseConversion, CaseMode, DeleteRange, InsertText,
    NumberPosition, RegexReplace, RulePipeline, StripBrackets, TextReplace,
};
pub use template::TemplateEngine;

pub fn crate_name() -> &'static str {
    "renamer"
}

mod media_journal;
pub use media_journal::recover_media_operations;
