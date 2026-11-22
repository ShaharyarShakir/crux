//! HTTP network transport abstractions and download implementations.

use super::AcquisitionError;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::time::Duration;

/// Default HTTP inactivity timeout (30 seconds).
///
/// This bounds connection establishment and each individual socket read or
/// write. It is deliberately *not* a deadline for the whole transfer: a large
/// archive on a slow but live connection must be allowed to finish, while a
/// stalled connection still fails after this long without progress.
pub const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum archive download size (500 MB).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;

/// Abstract HTTP transport for streaming downloads.
pub trait HttpTransport: Send + Sync {
    /// Streams the body from `url` into `writer`, computing the SHA-256 digest on the fly.
    /// Enforces `max_bytes` download limits. `timeout` is an inactivity limit
    /// applied to connecting and to each read/write, not to the whole transfer.
    /// Returns `(total_bytes, sha256_hex_digest)`.
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError>;

    /// Like [`HttpTransport::download_to`], reporting `(downloaded, total)`
    /// as bytes arrive. `total` is the server-declared `Content-Length` when
    /// known and is never guessed. The default implementation reports no
    /// progress, so existing transports keep working unchanged.
    fn download_to_with_progress(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(u64, String), AcquisitionError> {
        let _ = progress;
        self.download_to(url, writer, max_bytes, timeout)
    }
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
        if !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }
        stream_download(
            build_agent(timeout, true),
            url,
            writer,
            max_bytes,
            &mut |_, _| {},
        )
    }

    fn download_to_with_progress(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(u64, String), AcquisitionError> {
        if !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }
        stream_download(build_agent(timeout, true), url, writer, max_bytes, progress)
    }
}

/// Builds the HTTP agent used for archive downloads.
///
/// `https_only` makes ureq refuse to follow a redirect to plain HTTP *before*
/// any request is sent there. It is only disabled by loopback unit tests.
fn build_agent(timeout: Duration, https_only: bool) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .https_only(https_only)
        .redirects(5)
        .build()
}

fn stream_download(
    agent: ureq::Agent,
    url: &str,
    writer: &mut dyn Write,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(u64, String), AcquisitionError> {
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

    if url.starts_with("https://") && !response.get_url().starts_with("https://") {
        return Err(AcquisitionError::UnsupportedScheme(
            response.get_url().to_string(),
        ));
    }

    // Content-Length is only a declared size; it is used for display, never
    // trusted for limits (the byte counter below enforces `max_bytes`).
    let total = response
        .header("Content-Length")
        .and_then(|value| value.trim().parse::<u64>().ok());
    progress(0, total);
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
        progress(total_bytes, total);
    }

    writer.flush().map_err(AcquisitionError::Io)?;
    let digest = format!("{:x}", hasher.finalize());
    Ok((total_bytes, digest))
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
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError> {
        self.download_to_with_progress(url, writer, max_bytes, timeout, &mut |_, _| {})
    }

    fn download_to_with_progress(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        _timeout: Duration,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(u64, String), AcquisitionError> {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if !url.starts_with("https://") {
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

                // Deliver in chunks with a declared total so progress
                // reporting can be exercised offline.
                let total = Some(bytes.len() as u64);
                progress(0, total);
                let mut written = 0u64;
                for chunk in bytes.chunks(16 * 1024) {
                    writer.write_all(chunk).map_err(AcquisitionError::Io)?;
                    written += chunk.len() as u64;
                    progress(written, total);
                }
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

#[cfg(test)]
mod tests {
    //! Loopback-only timeout tests. A local `127.0.0.1` listener stands in for
    //! a remote server; no external network is contacted.
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    /// Serves one HTTP/1.1 response whose body is sent in `chunks`, sleeping
    /// `gap` before each chunk.
    fn serve_slowly(chunks: Vec<Vec<u8>>, gap: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/archive.tar.gz", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if line == "\r\n" {
                    break;
                }
                line.clear();
            }
            let total: usize = chunks.iter().map(Vec::len).sum();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.flush();
            for chunk in chunks {
                thread::sleep(gap);
                if stream.write_all(&chunk).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        (url, handle)
    }

    /// Regression: the agent used an overall request deadline, so any
    /// transfer lasting longer than the timeout failed even while data flowed.
    #[test]
    fn slow_but_live_transfer_outlasting_timeout_succeeds() {
        let chunks: Vec<Vec<u8>> = (0..6u8).map(|i| vec![i; 1024]).collect();
        let expected: Vec<u8> = chunks.concat();
        let (url, server) = serve_slowly(chunks, Duration::from_millis(250));

        let timeout = Duration::from_millis(800);
        let started = std::time::Instant::now();
        let mut body = Vec::new();
        let (bytes, digest) = stream_download(
            build_agent(timeout, false),
            &url,
            &mut body,
            1 << 20,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(
            started.elapsed() > timeout,
            "transfer must outlast the inactivity timeout to be meaningful"
        );
        assert_eq!(bytes, expected.len() as u64);
        assert_eq!(body, expected);
        assert_eq!(digest, format!("{:x}", Sha256::digest(&expected)));
        server.join().unwrap();
    }

    #[test]
    fn stalled_transfer_fails_with_read_timeout() {
        let (url, server) = serve_slowly(vec![vec![1u8; 16]], Duration::from_secs(3));
        let started = std::time::Instant::now();
        let err = stream_download(
            build_agent(Duration::from_millis(300), false),
            &url,
            &mut Vec::new(),
            1 << 20,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(
            matches!(err, AcquisitionError::DownloadFailed { .. }),
            "{err}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        server.join().unwrap();
    }

    #[test]
    fn production_agent_refuses_plain_http() {
        let err = UreqHttpTransport
            .download_to(
                "http://127.0.0.1:9/archive.tar.gz",
                &mut Vec::new(),
                1024,
                Duration::from_millis(100),
            )
            .unwrap_err();
        assert!(matches!(err, AcquisitionError::UnsupportedScheme(_)));
    }

    #[test]
    fn progress_reports_declared_content_length_and_byte_counts() {
        let chunks: Vec<Vec<u8>> = (0..4u8).map(|i| vec![i; 1000]).collect();
        let (url, server) = serve_slowly(chunks, Duration::from_millis(1));
        let mut events = Vec::new();
        let (bytes, _) = stream_download(
            build_agent(Duration::from_secs(5), false),
            &url,
            &mut Vec::new(),
            1 << 20,
            &mut |done, total| events.push((done, total)),
        )
        .unwrap();
        assert_eq!(bytes, 4000);
        assert_eq!(events.first(), Some(&(0, Some(4000))));
        assert_eq!(events.last(), Some(&(4000, Some(4000))));
        assert!(events.windows(2).all(|w| w[0].0 <= w[1].0), "monotonic");
        server.join().unwrap();
    }
}
