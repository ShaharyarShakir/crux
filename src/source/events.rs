//! Structured source-acquisition events.
//!
//! The acquirer reports *what happened* (data only); presentation layers
//! decide how to show it. Events never contain credentials: URLs are
//! redacted with [`crate::resolution::redact_url_credentials`].

use crate::package::PackageId;

/// One step of acquiring a package's sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionEvent {
    /// A verified, previously extracted source tree was reused.
    SourceCacheHit,
    /// A verified cached archive was reused; extraction follows.
    ArchiveCacheHit,
    /// An archive download started. `total` is the declared size, if any.
    DownloadStarted { url: String, total: Option<u64> },
    /// Bytes received so far (`total` as declared by the server, if any).
    DownloadProgress { downloaded: u64, total: Option<u64> },
    /// The download completed.
    DownloadFinished { bytes: u64 },
    /// The archive's SHA-256 matched the expected checksum.
    ChecksumVerified { checksum: String },
    /// The archive was extracted into the content-addressed cache.
    Extracted,
    /// A Git repository is being cloned.
    GitCloning { url: String },
    /// A Git ref resolved to (or a locked commit was) this commit.
    GitCommit { commit: String, cached: bool },
    /// A local path source was snapshotted into the cache (or reused).
    PathSnapshot { reused: bool },
}

/// Receives acquisition events. Implementations must be cheap: progress
/// events can arrive for every received chunk.
pub trait AcquisitionObserver: Send + Sync {
    fn on_event(&self, package: &PackageId, event: &AcquisitionEvent);
}

/// An observer that records every event, for tests and tooling.
#[derive(Debug, Default)]
pub struct RecordingObserver {
    events: std::sync::Mutex<Vec<(PackageId, AcquisitionEvent)>>,
}

impl RecordingObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns and clears the recorded events.
    pub fn take(&self) -> Vec<(PackageId, AcquisitionEvent)> {
        self.events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }
}

impl AcquisitionObserver for RecordingObserver {
    fn on_event(&self, package: &PackageId, event: &AcquisitionEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push((package.clone(), event.clone()));
        }
    }
}
