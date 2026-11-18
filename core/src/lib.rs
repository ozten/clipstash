//! clipstash core: storage, search, capture filters, retention and placeholder
//! expansion. This crate has no UI or OS dependencies.

pub mod filter;
pub mod history;
pub mod model;
pub mod paths;
pub mod placeholder;
pub mod search;
pub mod settings;
pub mod snippets;

pub use filter::{evaluate, precheck, CaptureMeta, DropReason};
pub use history::{CaptureOutcome, ClearScope, HistoryStore};
pub use model::*;
pub use search::{search, SearchFilter, SearchHit};
pub use settings::{MergeSeparator, Retention, Settings};
pub use snippets::{ImportMode, ImportReport, SnippetStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Current time as unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Open both stores in the given data directory.
pub fn open_stores(dir: &std::path::Path) -> Result<(HistoryStore, SnippetStore)> {
    Ok((HistoryStore::open(dir)?, SnippetStore::open(dir)?))
}
