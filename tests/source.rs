//! Integration test suite for Milestone 10: Source Acquisition, Verification, Caching, and Extraction.

use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use crux::registry::{GitRegistry, PackageSourceDefinition};
use crux::source::{
    AcquisitionError, ArchiveFormat, ExtractionLimits, MockHttpTransport, SourceAcquirer,
    SourceCache,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "crux_m10_test_{prefix}_{}_{}",
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn compute_sha256(bytes: &[u8]) -> Checksum {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Checksum::new_sha256(format!("{:x}", hasher.finalize())).unwrap()
}

fn create_test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
    buf.into_inner()
}

fn create_test_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
        builder.finish().unwrap();
    }
    tar_buf
}

fn create_test_tar_raw(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(100);
            header.as_mut_bytes()[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
        builder.finish().unwrap();
    }
    tar_buf
}

fn create_test_targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let tar_buf = create_test_tar(files);
    let mut gz_buf = Vec::new();
    {
        let mut encoder =
            flate2::write::GzEncoder::new(&mut gz_buf, flate2::Compression::default());
        encoder.write_all(&tar_buf).unwrap();
        encoder.finish().unwrap();
    }
    gz_buf
}

fn create_test_tar_with_symlink(link_name: &str, target: &str) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        let mut header = tar::Header::new_gnu();
        header.set_path(link_name).unwrap();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name(target).unwrap();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
        builder.finish().unwrap();
    }
    tar_buf
}

// -------------------------------------------------------------------------------------------------
// 1. Metadata and Format Validation Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_metadata_validation_supported_formats() {
    assert_eq!(
        ArchiveFormat::from_url_or_filename("https://example.com/fmt.zip"),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        ArchiveFormat::from_url_or_filename("https://example.com/zlib.tar.gz"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_url_or_filename("https://example.com/catch2.tgz"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_url_or_filename("https://example.com/archive.tar"),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(
        ArchiveFormat::from_url_or_filename("https://example.com/bad.exe"),
        None
    );
}

#[test]
fn test_unsupported_url_schemes_rejected() {
    let cache_dir = unique_temp_dir("unsupported_scheme");
    let cache = SourceCache::new(&cache_dir).unwrap();
    let acquirer = SourceAcquirer::new(cache);
    let pkg_id = PackageId::new(
        PackageName::new("test").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let checksum =
        Checksum::new_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();

    let invalid_urls = [
        "file:///etc/passwd",
        "ftp://example.com/archive.zip",
        "ssh://git@github.com/repo.git",
        "javascript:alert(1)",
        "data:text/plain;base64,SGVsbG8=",
    ];

    for url in &invalid_urls {
        let res = acquirer.acquire_archive(&pkg_id, url, &checksum, false);
        assert!(
            matches!(res, Err(AcquisitionError::UnsupportedScheme(_))),
            "expected UnsupportedScheme for '{url}', got {:?}",
            res
        );
    }

    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 2. Download and Cryptographic Verification Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_download_and_verify_matching_checksum_zip() {
    let cache_dir = unique_temp_dir("zip_success");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let zip_bytes = create_test_zip(&[
        ("CMakeLists.txt", b"cmake_minimum_required(VERSION 3.15)"),
        ("src/main.cpp", b"int main() { return 0; }"),
    ]);
    let checksum = compute_sha256(&zip_bytes);

    let url = "https://example.com/test-1.0.0.zip";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport.clone());
    let pkg_id = PackageId::new(
        PackageName::new("test").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let acquired = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .expect("should successfully acquire and verify zip");

    assert_eq!(acquired.package_id, pkg_id);
    assert_eq!(acquired.checksum, checksum);
    assert!(acquired.archive().is_some_and(|path| path.is_file()));
    assert!(acquired.source_dir.is_dir());
    assert!(acquired.source_dir.join("CMakeLists.txt").is_file());
    assert!(acquired.source_dir.join("src").join("main.cpp").is_file());
    assert!(cache.has_valid_source(checksum.digest()));
    assert!(cache.has_valid_archive(checksum.digest()));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_download_and_verify_matching_checksum_targz() {
    let cache_dir = unique_temp_dir("targz_success");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let targz_bytes = create_test_targz(&[
        ("include/zlib.h", b"#define ZLIB_VERSION 1.3.1"),
        ("adler32.c", b"/* adler32 algorithm */"),
    ]);
    let checksum = compute_sha256(&targz_bytes);

    let url = "https://example.com/zlib-1.3.1.tar.gz";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, targz_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("zlib").unwrap(),
        semver::Version::new(1, 3, 1),
    );

    let acquired = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .expect("should successfully acquire and verify tar.gz");

    assert!(acquired.source_dir.join("include").join("zlib.h").is_file());
    assert!(acquired.source_dir.join("adler32.c").is_file());
    assert!(cache.has_valid_source(checksum.digest()));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_checksum_mismatch_rejects_and_cleans_up() {
    let cache_dir = unique_temp_dir("checksum_mismatch");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let actual_bytes = b"tampered malicious content";
    let actual_checksum = compute_sha256(actual_bytes);
    let expected_checksum =
        Checksum::new_sha256("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .unwrap();

    let url = "https://example.com/tampered.tar.gz";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, actual_bytes.to_vec());

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("bad").unwrap(),
        semver::Version::new(0, 1, 0),
    );

    let res = acquirer.acquire_archive(&pkg_id, url, &expected_checksum, false);
    match res {
        Err(AcquisitionError::ChecksumMismatch { expected, actual }) => {
            assert!(
                expected
                    .contains("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            );
            assert!(actual.contains(actual_checksum.digest()));
        }
        other => panic!("expected ChecksumMismatch, got {:?}", other),
    }

    assert!(!cache.has_valid_archive(expected_checksum.digest()));
    assert!(!cache.has_valid_source(expected_checksum.digest()));
    assert!(!cache.archive_path(expected_checksum.digest()).exists());
    assert!(!cache.source_dir(expected_checksum.digest()).exists());

    let tmp_entries: Vec<_> = fs::read_dir(cache.tmp_dir()).unwrap().collect();
    assert!(
        tmp_entries.is_empty(),
        "expected temp directory to be clean"
    );

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_download_http_error_handling() {
    let cache_dir = unique_temp_dir("http_error");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let url_404 = "https://example.com/missing.zip";
    let url_500 = "https://example.com/server_error.tar.gz";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_status_error(url_404, 404, "Not Found");
    transport.add_status_error(url_500, 500, "Internal Server Error");

    let acquirer = SourceAcquirer::with_transport(cache, transport);
    let pkg_id = PackageId::new(
        PackageName::new("foo").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let checksum =
        Checksum::new_sha256("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let err404 = acquirer
        .acquire_archive(&pkg_id, url_404, &checksum, false)
        .unwrap_err();
    assert!(matches!(
        err404,
        AcquisitionError::DownloadFailed {
            status: Some(404),
            ..
        }
    ));

    let err500 = acquirer
        .acquire_archive(&pkg_id, url_500, &checksum, false)
        .unwrap_err();
    assert!(matches!(
        err500,
        AcquisitionError::DownloadFailed {
            status: Some(500),
            ..
        }
    ));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_download_size_limit_exceeded() {
    let cache_dir = unique_temp_dir("size_limit");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let large_bytes = vec![0u8; 100 * 1024]; // 100 KB
    let checksum = compute_sha256(&large_bytes);
    let url = "https://example.com/large.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, large_bytes);

    let acquirer =
        SourceAcquirer::with_transport(cache.clone(), transport).with_max_download_bytes(10 * 1024); // max 10 KB
    let pkg_id = PackageId::new(
        PackageName::new("large").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(
        err,
        AcquisitionError::DownloadSizeExceeded { max_bytes: 10240 }
    ));

    let tmp_entries: Vec<_> = fs::read_dir(cache.tmp_dir()).unwrap().collect();
    assert!(tmp_entries.is_empty());

    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 3. Cache Hit and Corruption Recovery Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_cache_hit_skips_download_and_extraction() {
    let cache_dir = unique_temp_dir("cache_hit");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let zip_bytes = create_test_zip(&[("foo.txt", b"foo")]);
    let checksum = compute_sha256(&zip_bytes);
    let url = "https://example.com/pkg-1.0.0.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport.clone());
    let pkg_id = PackageId::new(
        PackageName::new("pkg").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let acq1 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(transport.request_count(), 1);

    let acq2 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(
        transport.request_count(),
        1,
        "second acquire must NOT make network requests"
    );
    assert_eq!(acq1.source_dir, acq2.source_dir);

    let acq3 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, true)
        .unwrap();
    assert_eq!(
        transport.request_count(),
        2,
        "force acquire must re-download"
    );
    assert_eq!(acq1.source_dir, acq3.source_dir);

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_cache_source_missing_reuses_cached_archive() {
    let cache_dir = unique_temp_dir("archive_reuse");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let zip_bytes = create_test_zip(&[("code.c", b"int x = 1;")]);
    let checksum = compute_sha256(&zip_bytes);
    let url = "https://example.com/reuse-1.0.0.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport.clone());
    let pkg_id = PackageId::new(
        PackageName::new("reuse").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let acq1 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(transport.request_count(), 1);

    fs::remove_dir_all(&acq1.source_dir).unwrap();
    assert!(!cache.has_valid_source(checksum.digest()));
    assert!(cache.has_valid_archive(checksum.digest()));

    let acq2 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(
        transport.request_count(),
        1,
        "re-extract must NOT re-download archive"
    );
    assert!(acq2.source_dir.join("code.c").is_file());

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_corrupted_cached_archive_is_purged_and_redownloaded() {
    let cache_dir = unique_temp_dir("corrupted_cache");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let good_bytes = create_test_zip(&[("valid.txt", b"valid")]);
    let checksum = compute_sha256(&good_bytes);
    let url = "https://example.com/recover-1.0.0.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, good_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport.clone());
    let pkg_id = PackageId::new(
        PackageName::new("recover").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let acq1 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(transport.request_count(), 1);

    fs::remove_dir_all(&acq1.source_dir).unwrap();
    let archive_path = acq1.archive().expect("archive-backed source").to_path_buf();
    fs::write(&archive_path, b"corrupted bytes!").unwrap();

    assert!(!cache.has_valid_archive(checksum.digest()));
    assert!(!archive_path.exists());

    let acq2 = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();
    assert_eq!(
        transport.request_count(),
        2,
        "must re-download after corrupted archive"
    );
    assert!(acq2.source_dir.join("valid.txt").is_file());

    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 4. Archive Extraction Security & Decompression Safety Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_path_traversal_zip_slip_rejected() {
    let cache_dir = unique_temp_dir("zip_slip");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let malicious_zip = create_test_zip(&[("../../escaped.txt", b"evil"), ("normal.txt", b"ok")]);
    let checksum = compute_sha256(&malicious_zip);
    let url = "https://example.com/slip.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, malicious_zip);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("slip").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(err, AcquisitionError::SecurityViolation(_)));

    let escaped_path = cache_dir.join("escaped.txt");
    assert!(!escaped_path.exists());
    assert!(!cache.has_valid_source(checksum.digest()));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_path_traversal_tar_slip_rejected() {
    let cache_dir = unique_temp_dir("tar_slip");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let malicious_tar = create_test_tar_raw(&[("../../evil_tar.txt", b"evil tar")]);
    let checksum = compute_sha256(&malicious_tar);
    let url = "https://example.com/slip.tar";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, malicious_tar);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("tarslip").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(err, AcquisitionError::SecurityViolation(_)));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_symlink_pointing_outside_extraction_root_rejected() {
    let cache_dir = unique_temp_dir("symlink_escape");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let tar_symlink_escape = create_test_tar_with_symlink("escape_link", "../../etc/shadow");
    let checksum = compute_sha256(&tar_symlink_escape);
    let url = "https://example.com/symlink.tar";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, tar_symlink_escape);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("symlink").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(err, AcquisitionError::SecurityViolation(_)));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_decompression_bomb_uncompressed_bytes_limit() {
    let cache_dir = unique_temp_dir("decompression_bomb");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let bomb_data = vec![b'A'; 200 * 1024];
    let bomb_zip = create_test_zip(&[("bomb.txt", &bomb_data)]);
    let checksum = compute_sha256(&bomb_zip);
    let url = "https://example.com/bomb.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, bomb_zip);

    let limits = ExtractionLimits {
        max_uncompressed_bytes: 50 * 1024,
        max_files: 1000,
    };

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport).with_limits(limits);
    let pkg_id = PackageId::new(
        PackageName::new("bomb").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(err, AcquisitionError::DecompressionBomb(_)));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_decompression_bomb_file_count_limit() {
    let cache_dir = unique_temp_dir("file_count_bomb");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let files: Vec<(String, &[u8])> = (0..50)
        .map(|i| (format!("file_{i}.txt"), b"x".as_slice()))
        .collect();
    let file_refs: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (n.as_str(), *c)).collect();

    let zip_bytes = create_test_zip(&file_refs);
    let checksum = compute_sha256(&zip_bytes);
    let url = "https://example.com/many_files.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let limits = ExtractionLimits {
        max_uncompressed_bytes: 1024 * 1024,
        max_files: 10,
    };

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport).with_limits(limits);
    let pkg_id = PackageId::new(
        PackageName::new("many").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap_err();
    assert!(matches!(err, AcquisitionError::DecompressionBomb(_)));

    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_single_top_level_directory_unwrapping() {
    let cache_dir = unique_temp_dir("unwrap");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let zip_bytes = create_test_zip(&[
        ("fmt-11.1.1/CMakeLists.txt", b"cmake 3.15"),
        ("fmt-11.1.1/include/fmt/format.h", b"// format.h"),
        ("fmt-11.1.1/src/format.cc", b"// format.cc"),
    ]);
    let checksum = compute_sha256(&zip_bytes);
    let url = "https://example.com/fmt-11.1.1.zip";

    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);
    let pkg_id = PackageId::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::new(11, 1, 1),
    );

    let acquired = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .unwrap();

    assert!(acquired.source_dir.join("CMakeLists.txt").is_file());
    assert!(
        acquired
            .source_dir
            .join("include")
            .join("fmt")
            .join("format.h")
            .is_file()
    );
    assert!(acquired.source_dir.join("src").join("format.cc").is_file());
    assert!(!acquired.source_dir.join("fmt-11.1.1").exists());

    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 5. Real Seed Registry Packages Testing (fmt 11.1.1, zlib 1.3.1, catch2 3.8.0)
// -------------------------------------------------------------------------------------------------

#[test]
fn test_acquire_seed_registry_package_definitions() {
    let seed_registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
    if !seed_registry_path.join("registry.toml").is_file() {
        return;
    }

    let registry = GitRegistry::open(&seed_registry_path).expect("failed to open seed registry");
    let cache_dir = unique_temp_dir("seed_pkgs");
    let cache = SourceCache::new(&cache_dir).unwrap();
    let transport = Arc::new(MockHttpTransport::new());

    // 1. fmt 11.1.1 (.zip)
    let fmt_id = PackageId::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::new(11, 1, 1),
    );
    let fmt_def = registry
        .get_package_definition(&fmt_id)
        .expect("fmt 11.1.1 must exist in seed registry");
    let (fmt_url, _) = match &fmt_def.source {
        PackageSourceDefinition::Archive { url, checksum } => (url.clone(), checksum.clone()),
        _ => panic!("expected archive source for fmt"),
    };
    let dummy_zip = create_test_zip(&[
        ("fmt-11.1.1/CMakeLists.txt", b"project(fmt)"),
        ("fmt-11.1.1/include/fmt/format.h", b"// fmt"),
    ]);
    let dummy_checksum = compute_sha256(&dummy_zip);
    let mut test_fmt_def = fmt_def.clone();
    test_fmt_def.source = PackageSourceDefinition::Archive {
        url: fmt_url.clone(),
        checksum: dummy_checksum.clone(),
    };
    transport.add_response(&fmt_url, dummy_zip);

    // 2. zlib 1.3.1 (.tar.gz)
    let zlib_id = PackageId::new(
        PackageName::new("zlib").unwrap(),
        semver::Version::new(1, 3, 1),
    );
    let zlib_def = registry
        .get_package_definition(&zlib_id)
        .expect("zlib 1.3.1 must exist in seed registry");
    let (zlib_url, _) = match &zlib_def.source {
        PackageSourceDefinition::Archive { url, checksum } => (url.clone(), checksum.clone()),
        _ => panic!("expected archive source for zlib"),
    };
    let dummy_targz = create_test_targz(&[
        ("zlib-1.3.1/CMakeLists.txt", b"project(zlib)"),
        ("zlib-1.3.1/zlib.h", b"#define ZLIB 1"),
    ]);
    let dummy_zlib_checksum = compute_sha256(&dummy_targz);
    let mut test_zlib_def = zlib_def.clone();
    test_zlib_def.source = PackageSourceDefinition::Archive {
        url: zlib_url.clone(),
        checksum: dummy_zlib_checksum.clone(),
    };
    transport.add_response(&zlib_url, dummy_targz);

    // 3. catch2 3.8.0 (.tar.gz)
    let catch2_id = PackageId::new(
        PackageName::new("catch2").unwrap(),
        semver::Version::new(3, 8, 0),
    );
    let catch2_def = registry
        .get_package_definition(&catch2_id)
        .expect("catch2 3.8.0 must exist in seed registry");
    let (catch2_url, _) = match &catch2_def.source {
        PackageSourceDefinition::Archive { url, checksum } => (url.clone(), checksum.clone()),
        _ => panic!("expected archive source for catch2"),
    };
    let dummy_catch2 = create_test_targz(&[("Catch2-3.8.0/CMakeLists.txt", b"project(Catch2)")]);
    let dummy_catch2_checksum = compute_sha256(&dummy_catch2);
    let mut test_catch2_def = catch2_def.clone();
    test_catch2_def.source = PackageSourceDefinition::Archive {
        url: catch2_url.clone(),
        checksum: dummy_catch2_checksum.clone(),
    };
    transport.add_response(&catch2_url, dummy_catch2);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport);

    let acq_fmt = acquirer
        .acquire_package_definition(&fmt_id, &test_fmt_def, false)
        .unwrap();
    assert!(acq_fmt.source_dir.join("CMakeLists.txt").is_file());

    let acq_zlib = acquirer
        .acquire_package_definition(&zlib_id, &test_zlib_def, false)
        .unwrap();
    assert!(acq_zlib.source_dir.join("CMakeLists.txt").is_file());

    let acq_catch2 = acquirer
        .acquire_package_definition(&catch2_id, &test_catch2_def, false)
        .unwrap();
    assert!(acq_catch2.source_dir.join("CMakeLists.txt").is_file());

    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 6. Real Loopback TCP HTTP Transport Test (UreqHttpTransport)
// -------------------------------------------------------------------------------------------------

#[test]
fn test_loopback_http_streaming_download_with_ureq() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local tcp");
    let local_addr = listener.local_addr().unwrap();

    let archive_bytes = create_test_zip(&[("loopback.txt", b"loopback download works!")]);
    let checksum = compute_sha256(&archive_bytes);
    let bytes_to_serve = archive_bytes.clone();

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf);

            let response_headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes_to_serve.len()
            );
            let _ = stream.write_all(response_headers.as_bytes());
            let _ = stream.write_all(&bytes_to_serve);
            let _ = stream.flush();
        }
    });

    let url = format!("http://{local_addr}/archive.zip");
    let cache_dir = unique_temp_dir("loopback_ureq");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let acquirer = SourceAcquirer::new(cache);
    let pkg_id = PackageId::new(
        PackageName::new("loopback").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let acquired = acquirer
        .acquire_archive(&pkg_id, &url, &checksum, false)
        .expect("production UreqHttpTransport should successfully download from loopback");

    assert!(acquired.source_dir.join("loopback.txt").is_file());
    let content = fs::read_to_string(acquired.source_dir.join("loopback.txt")).unwrap();
    assert_eq!(content, "loopback download works!");

    server_handle.join().unwrap();
    let _ = fs::remove_dir_all(&cache_dir);
}

// -------------------------------------------------------------------------------------------------
// 7. CLI Command Tests (crux fetch)
// -------------------------------------------------------------------------------------------------

#[test]
fn test_cli_fetch_help() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = crux::cli::run(["crux", "fetch", "--help"], &mut stdout, &mut stderr);
    assert!(res.is_ok());

    let out_str = String::from_utf8(stdout).unwrap();
    assert!(out_str.contains("crux-fetch"));
    assert!(out_str.contains("Usage:"));
    assert!(out_str.contains("--force"));
}

#[test]
fn test_cli_fetch_unexpected_flag() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = crux::cli::run(["crux", "fetch", "--bogus"], &mut stdout, &mut stderr);
    assert!(res.is_err());

    let err_str = String::from_utf8(stderr).unwrap();
    assert!(err_str.contains("unexpected argument '--bogus'"));
}

#[test]
fn test_cli_fetch_package_from_configured_registry() {
    let _guard = ENV_LOCK.lock().unwrap();

    let seed_registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
    if !seed_registry_path.join("registry.toml").is_file() {
        return;
    }

    let project_dir = unique_temp_dir("cli_fetch_pkg_proj");
    let cache_dir = unique_temp_dir("cli_fetch_pkg_cache");

    // Create crux.toml so project_dir is recognized as project
    let manifest_content = "\
[package]
name = \"app\"
version = \"0.1.0\"
";
    fs::write(project_dir.join("crux.toml"), manifest_content).unwrap();

    // Configure registry and cache in .crux/config.toml
    let crux_dir = project_dir.join(".crux");
    fs::create_dir_all(&crux_dir).unwrap();
    let config_content = format!(
        "cache_dir = \"{}\"\n\n[registries.default]\npath = \"{}\"\n",
        cache_dir.display(),
        seed_registry_path.display()
    );
    fs::write(crux_dir.join("config.toml"), config_content).unwrap();

    // Pre-populate cache with verified fmt 11.1.1 archive so fetch doesn't need external network
    let cache = SourceCache::new(&cache_dir).unwrap();
    let fmt_zip = create_test_zip(&[("fmt-11.1.1/CMakeLists.txt", b"project(fmt)")]);
    let fmt_digest = "a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45";
    let archive_path = cache.archive_path(fmt_digest);
    fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    fs::write(&archive_path, fmt_zip).unwrap();

    // Run crux fetch fmt@11.1.1 inside project_dir
    let orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_dir).unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = crux::cli::run(["crux", "fetch", "fmt@11.1.1"], &mut stdout, &mut stderr);

    std::env::set_current_dir(orig_dir).unwrap();

    let out_str = String::from_utf8(stdout).unwrap();
    let err_str = String::from_utf8(stderr).unwrap();
    assert!(res.is_ok(), "crux fetch failed: {err_str}");
    assert!(out_str.contains("Successfully acquired fmt 11.1.1"));

    assert!(cache.has_valid_source(fmt_digest));

    let _ = fs::remove_dir_all(&project_dir);
    let _ = fs::remove_dir_all(&cache_dir);
}

#[test]
fn test_cli_fetch_all_dependencies_from_lockfile() {
    let _guard = ENV_LOCK.lock().unwrap();

    let seed_registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
    if !seed_registry_path.join("registry.toml").is_file() {
        return;
    }

    let project_dir = unique_temp_dir("cli_fetch_all_proj");
    let cache_dir = unique_temp_dir("cli_fetch_all_cache");

    // Create crux.toml
    let manifest_content = "\
[package]
name = \"my_app\"
version = \"0.1.0\"

[dependencies]
fmt = \"^11.1\"
";
    fs::write(project_dir.join("crux.toml"), manifest_content).unwrap();

    // Configure registry and cache in .crux/config.toml
    let crux_dir = project_dir.join(".crux");
    fs::create_dir_all(&crux_dir).unwrap();
    let config_content = format!(
        "cache_dir = \"{}\"\n\n[registries.default]\npath = \"{}\"\n",
        cache_dir.display(),
        seed_registry_path.display()
    );
    fs::write(crux_dir.join("config.toml"), config_content).unwrap();

    // Create crux.lock with fmt 11.1.1
    let fmt_digest = "a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45";
    let lock_content = format!(
        "\
lockfile_version = 1

[[package]]
name = \"fmt\"
version = \"11.1.1\"
source = \"registry+default\"
checksum = \"sha256:{fmt_digest}\"
dependencies = []
"
    );
    fs::write(project_dir.join("crux.lock"), lock_content).unwrap();

    // Pre-populate cache archive
    let cache = SourceCache::new(&cache_dir).unwrap();
    let fmt_zip = create_test_zip(&[("fmt-11.1.1/CMakeLists.txt", b"project(fmt)")]);
    let archive_path = cache.archive_path(fmt_digest);
    fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    fs::write(&archive_path, fmt_zip).unwrap();

    let orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_dir).unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = crux::cli::run(["crux", "fetch"], &mut stdout, &mut stderr);

    std::env::set_current_dir(orig_dir).unwrap();

    let out_str = String::from_utf8(stdout).unwrap();
    let err_str = String::from_utf8(stderr).unwrap();
    assert!(res.is_ok(), "crux fetch failed: {err_str}");
    assert!(out_str.contains("Successfully acquired 1 dependencies:"));
    assert!(out_str.contains("fmt 11.1.1"));

    let _ = fs::remove_dir_all(&project_dir);
    let _ = fs::remove_dir_all(&cache_dir);
}
