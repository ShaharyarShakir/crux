//! HTTP network transport abstractions and download implementations.

use super::AcquisitionError;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::time::Duration;

/// Default timeout for HTTP downloads (30 seconds).
pub const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum archive download size (500 MB).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;

/// Abstract HTTP transport for streaming downloads.
pub trait HttpTransport: Send + Sync {
    /// Streams the body from `url` into `writer`, computing the SHA-256 digest on the fly.
    /// Enforces `max_bytes` download limits and `timeout`.
    /// Returns `(total_bytes, sha256_hex_digest)`.
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError>;
}

/// Standard production HTTP transport using `ureq`.
#[derive(Debug, Default, Clone)]
pub struct UreqHttpTransport;

impl HttpTransport for UreqHttpTransport {
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }

        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .redirects(5)
            .build();

        let response = match agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(code, resp)) => {
                let status_text = resp.status_text().to_string();
                return Err(AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: Some(code),
                    message: status_text,
                });
            }
            Err(ureq::Error::Transport(err)) => {
                return Err(AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: None,
                    message: err.to_string(),
                });
            }
        };

        let mut reader = response.into_reader();
        let mut hasher = Sha256::new();
        let mut total_bytes: u64 = 0;
        let mut buf = [0u8; 16384];

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: None,
                    message: format!("read error: {e}"),
                })?;
            if n == 0 {
                break;
            }
            total_bytes += n as u64;
            if total_bytes > max_bytes {
                return Err(AcquisitionError::DownloadSizeExceeded { max_bytes });
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n]).map_err(AcquisitionError::Io)?;
        }

        writer.flush().map_err(AcquisitionError::Io)?;
        let digest = format!("{:x}", hasher.finalize());
        Ok((total_bytes, digest))
    }
}

/// In-memory mock HTTP transport for offline unit and integration testing.
#[derive(Default)]
pub struct MockHttpTransport {
    responses: std::sync::Mutex<std::collections::BTreeMap<String, MockResponse>>,
    request_count: std::sync::atomic::AtomicUsize,
}

/// Encapsulates mocked responses for `MockHttpTransport`.
pub enum MockResponse {
    Success(Vec<u8>),
    Status(u16, String),
    NetworkError(String),
}

impl MockHttpTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a canned successful response for `url`.
    pub fn add_response(&self, url: impl Into<String>, bytes: Vec<u8>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::Success(bytes));
    }

    /// Registers an HTTP error response for `url`.
    pub fn add_status_error(
        &self,
        url: impl Into<String>,
        status: u16,
        message: impl Into<String>,
    ) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::Status(status, message.into()));
    }

    /// Registers a simulated network/transport error for `url`.
    pub fn add_network_error(&self, url: impl Into<String>, message: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::NetworkError(message.into()));
    }

    /// Returns the total number of download requests made through this transport.
    pub fn request_count(&self) -> usize {
        self.request_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl HttpTransport for MockHttpTransport {
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        _timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError> {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }

        let resp_guard = self.responses.lock().unwrap();
        let resp = resp_guard
            .get(url)
            .ok_or_else(|| AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: Some(404),
                message: "Not Found in MockHttpTransport".to_string(),
            })?;

        match resp {
            MockResponse::Success(bytes) => {
                if (bytes.len() as u64) > max_bytes {
                    return Err(AcquisitionError::DownloadSizeExceeded { max_bytes });
                }
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                let digest = format!("{:x}", hasher.finalize());

                writer.write_all(bytes).map_err(AcquisitionError::Io)?;
                writer.flush().map_err(AcquisitionError::Io)?;
                Ok((bytes.len() as u64, digest))
            }
            MockResponse::Status(code, msg) => Err(AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: Some(*code),
                message: msg.clone(),
            }),
            MockResponse::NetworkError(msg) => Err(AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: None,
                message: msg.clone(),
            }),
        }
    }
}
