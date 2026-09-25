//! Local models: Hugging Face Hub discovery, verified resumable downloads,
//! GGUF inspection and classification, hardware detection and fit estimates,
//! the installed-model library, and Hugging Face credentials.
//!
//! Privacy: this crate performs network requests only when one of its async
//! API calls is made (search, model info, header read, download start/resume,
//! token validation/refresh). Nothing runs in the background on its own, and
//! bearer tokens are never logged or sent to CDN hosts.

pub mod auth;
pub mod classify;
pub mod download;
pub mod fit;
pub mod gguf;
pub mod hardware;
pub mod hub;
pub mod library;

mod http;
mod util;

#[cfg(test)]
mod test_util;

pub use auth::{HfAccount, HfCredentials, MemorySecretStore, SecretStore};
pub use classify::{classify, ModelKind};
pub use download::{
    CompletedDownload, DownloadEvents, DownloadJobView, DownloadManager, DownloadState,
    NoopDownloadEvents,
};
pub use fit::{estimate, max_context_that_fits, FitEstimate, FitVerdict, KvCacheType};
pub use gguf::{GgufError, GgufHeader, GgufSummary, GgufValue};
pub use hardware::HardwareInfo;
pub use hub::{
    HubClient, HubError, HubFile, HubModelInfo, HubModelSummary, HubPage, HubSearch, HubUser,
};
pub use library::{default_storage_dir, EntrySource, Library, LibraryEntry, LibraryError};
