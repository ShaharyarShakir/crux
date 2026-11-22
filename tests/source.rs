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
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

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
fn test_tampered_extracted_cache_is_rebuilt_from_verified_archive() {
    let cache_dir = unique_temp_dir("cache_tamper");
    let cache = SourceCache::new(&cache_dir).unwrap();

    let zip_bytes = create_test_zip(&[
        ("CMakeLists.txt", b"cmake_minimum_required(VERSION 3.15)"),
        ("src/main.cpp", b"int main() { return 0; }"),
    ]);
    let checksum = compute_sha256(&zip_bytes);
    let url = "https://example.com/cache-tamper.zip";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, zip_bytes);

    let acquirer = SourceAcquirer::with_transport(cache.clone(), transport.clone());
    let pkg_id = PackageId::new(
        PackageName::new("cache-tamper").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let first = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .expect("initial acquisition should succeed");

    fs::write(first.source_dir.join("src/main.cpp"), b"// tampered").unwrap();

    let second = acquirer
        .acquire_archive(&pkg_id, url, &checksum, false)
        .expect("tampered cache should be rebuilt from the verified archive");

    assert_eq!(
        fs::read_to_string(second.source_dir.join("src/main.cpp")).unwrap(),
        "int main() { return 0; }"
    );
    assert_eq!(
        transport.request_count(),
        1,
        "cache repair must not re-download the archive"
    );

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
// 6. HTTPS-Only Archive Enforcement
// -------------------------------------------------------------------------------------------------

#[test]
fn test_http_archive_urls_are_rejected_before_network_access() {
    let cache_dir = unique_temp_dir("http_rejected");
    let cache = SourceCache::new(&cache_dir).unwrap();
    let checksum = compute_sha256(b"unused");
    let acquirer = SourceAcquirer::new(cache);
    let pkg_id = PackageId::new(
        PackageName::new("insecure").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let err = acquirer
        .acquire_archive(&pkg_id, "http://127.0.0.1:1/archive.zip", &checksum, false)
        .expect_err("plaintext HTTP archive URLs must be rejected");

    assert!(matches!(err, AcquisitionError::UnsupportedScheme(_)));
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

// ---------------------------------------------------------------------------
// Remediation (2026-11-21 audit): extraction boundary regression coverage.
// ---------------------------------------------------------------------------

/// Archive entry description for hand-built tar fixtures.
enum TarItem<'a> {
    File(&'a str, &'a [u8]),
    Dir(&'a str),
    Symlink(&'a str, &'a str),
    HardLink(&'a str, &'a str),
}

fn build_tar(items: &[TarItem<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for item in items {
        let mut header = tar::Header::new_gnu();
        match item {
            TarItem::File(path, data) => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder.append_data(&mut header, path, *data).unwrap();
            }
            TarItem::Dir(path) => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o755);
                builder.append_data(&mut header, path, &[][..]).unwrap();
            }
            TarItem::Symlink(path, target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                builder.append_link(&mut header, path, target).unwrap();
            }
            TarItem::HardLink(path, target) => {
                header.set_entry_type(tar::EntryType::Link);
                header.set_size(0);
                header.set_mode(0o644);
                builder.append_link(&mut header, path, target).unwrap();
            }
        }
    }
    builder.into_inner().unwrap()
}

/// Creates `<sandbox>/a/b/staging` so escapes of several levels stay inside
/// the per-test sandbox and can be detected.
fn extraction_sandbox(prefix: &str) -> (PathBuf, PathBuf) {
    let sandbox = unique_temp_dir(prefix);
    let dest = sandbox.join("a/b/staging");
    fs::create_dir_all(&dest).unwrap();
    (sandbox, dest)
}

fn extract(bytes: &[u8], dest: &std::path::Path) -> Result<(), AcquisitionError> {
    crux::source::extraction::extract_tar_reader(bytes, dest, &ExtractionLimits::default())
}

fn files_outside(sandbox: &std::path::Path, dest: &std::path::Path) -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, dest: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.starts_with(dest) {
                continue;
            }
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, dest, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(sandbox, dest, &mut out);
    out
}

/// Exact reproduction of the audit proof of concept: each link is lexically
/// inside the destination, but the chain physically climbs three levels.
#[cfg(unix)]
#[test]
fn test_reported_symlink_chain_escape_is_rejected() {
    let (sandbox, dest) = extraction_sandbox("chain_poc");
    let tar = build_tar(&[
        TarItem::Symlink("x/s", ".."),
        TarItem::Symlink("x/s2", "s/.."),
        TarItem::Symlink("x/s3", "s2/../.."),
        TarItem::File("x/s3/PWNED.txt", b"escaped!\n"),
    ]);

    let result = extract(&tar, &dest);
    assert!(
        matches!(result, Err(AcquisitionError::SecurityViolation(_))),
        "chain escape must be a security violation, got {result:?}"
    );
    assert!(!sandbox.join("PWNED.txt").exists());
    assert!(!sandbox.join("a/PWNED.txt").exists());
    assert!(files_outside(&sandbox, &dest).is_empty());
    let _ = fs::remove_dir_all(&sandbox);
}

/// The chain must be caught regardless of entry order: here the second link
/// is created before the link that makes it escape.
#[cfg(unix)]
#[test]
fn test_symlink_chain_escape_rejected_in_reverse_order() {
    let (sandbox, dest) = extraction_sandbox("chain_reverse");
    let tar = build_tar(&[
        TarItem::Symlink("x/s2", "s/.."),
        TarItem::Symlink("x/s", ".."),
    ]);
    assert!(matches!(
        extract(&tar, &dest),
        Err(AcquisitionError::SecurityViolation(_))
    ));
    assert!(files_outside(&sandbox, &dest).is_empty());
    let _ = fs::remove_dir_all(&sandbox);
}

#[cfg(unix)]
#[test]
fn test_entries_cannot_be_written_through_symlinks() {
    let (sandbox, dest) = extraction_sandbox("through_link");
    // Even an in-tree directory symlink may not be used as a write path.
    let tar = build_tar(&[
        TarItem::Dir("real/"),
        TarItem::Symlink("alias", "real"),
        TarItem::File("alias/file.txt", b"data"),
    ]);
    assert!(matches!(
        extract(&tar, &dest),
        Err(AcquisitionError::SecurityViolation(_))
    ));
    assert!(!dest.join("real/file.txt").exists());

    // A pre-existing symlink planted in the destination is not followed either.
    let (sandbox2, dest2) = extraction_sandbox("planted_link");
    let outside = sandbox2.join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dest2.join("planted")).unwrap();
    let tar = build_tar(&[TarItem::File("planted/evil.txt", b"x")]);
    assert!(matches!(
        extract(&tar, &dest2),
        Err(AcquisitionError::SecurityViolation(_))
    ));
    assert!(!outside.join("evil.txt").exists());

    let _ = fs::remove_dir_all(&sandbox);
    let _ = fs::remove_dir_all(&sandbox2);
}

#[cfg(unix)]
#[test]
fn test_nested_paths_and_valid_internal_links_are_extracted() {
    let (sandbox, dest) = extraction_sandbox("valid_links");
    let tar = build_tar(&[
        TarItem::File("pkg/include/deep/nested/dir/api.h", b"int api();"),
        TarItem::File("pkg/src/lib.c", b"int api() { return 1; }"),
        TarItem::Symlink("pkg/include/api.h", "deep/nested/dir/api.h"),
        TarItem::Symlink("pkg/src/api.h", "../include/deep/nested/dir/api.h"),
        TarItem::Symlink("pkg/include/current", "deep/nested/dir"),
        TarItem::Symlink("pkg/self", "."),
        // Dangling but in-root links are legal.
        TarItem::Symlink("pkg/generated.h", "build-output/generated.h"),
        TarItem::HardLink("pkg/src/lib_copy.c", "pkg/src/lib.c"),
    ]);
    extract(&tar, &dest).expect("valid internal links must extract");

    assert_eq!(
        fs::read(dest.join("pkg/include/api.h")).unwrap(),
        b"int api();"
    );
    assert_eq!(fs::read(dest.join("pkg/src/api.h")).unwrap(), b"int api();");
    assert!(dest.join("pkg/include/current/api.h").is_file());
    assert_eq!(
        fs::read(dest.join("pkg/src/lib_copy.c")).unwrap(),
        b"int api() { return 1; }"
    );
    assert!(
        fs::symlink_metadata(dest.join("pkg/generated.h"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(files_outside(&sandbox, &dest).is_empty());
    let _ = fs::remove_dir_all(&sandbox);
}

#[cfg(unix)]
#[test]
fn test_overwrite_attempts_never_follow_links() {
    // A regular file replacing a symlink must not write to the link target.
    let (sandbox, dest) = extraction_sandbox("overwrite_symlink");
    let tar = build_tar(&[
        TarItem::File("victim.txt", b"original"),
        TarItem::Symlink("f", "victim.txt"),
        TarItem::File("f", b"overwritten"),
    ]);
    extract(&tar, &dest).unwrap();
    assert_eq!(fs::read(dest.join("victim.txt")).unwrap(), b"original");
    assert_eq!(fs::read(dest.join("f")).unwrap(), b"overwritten");
    assert!(
        !fs::symlink_metadata(dest.join("f"))
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // A regular file replacing a hard link must not modify the shared inode.
    let tar = build_tar(&[
        TarItem::File("shared.txt", b"original"),
        TarItem::HardLink("h", "shared.txt"),
        TarItem::File("h", b"overwritten"),
    ]);
    let (sandbox2, dest2) = extraction_sandbox("overwrite_hardlink");
    extract(&tar, &dest2).unwrap();
    assert_eq!(fs::read(dest2.join("shared.txt")).unwrap(), b"original");
    assert_eq!(fs::read(dest2.join("h")).unwrap(), b"overwritten");

    // A symlink or file may not replace an extracted directory.
    let (sandbox3, dest3) = extraction_sandbox("overwrite_dir");
    let tar = build_tar(&[TarItem::Dir("include/"), TarItem::Symlink("include", ".")]);
    assert!(extract(&tar, &dest3).is_err());
    assert!(
        fs::symlink_metadata(dest3.join("include"))
            .unwrap()
            .is_dir()
    );

    // A planted symlink at a file's own path is replaced, not followed.
    let (sandbox4, dest4) = extraction_sandbox("overwrite_planted");
    let outside = sandbox4.join("outside.txt");
    fs::write(&outside, b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, dest4.join("config.h")).unwrap();
    extract(&build_tar(&[TarItem::File("config.h", b"new")]), &dest4).unwrap();
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    assert_eq!(fs::read(dest4.join("config.h")).unwrap(), b"new");

    for dir in [sandbox, sandbox2, sandbox3, sandbox4] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(unix)]
#[test]
fn test_hard_links_cannot_reach_outside_or_through_symlinks() {
    let (sandbox, dest) = extraction_sandbox("hardlink_escape");
    fs::write(sandbox.join("a/secret.txt"), b"secret").unwrap();

    // Traversal in the target is rejected outright.
    let tar = build_tar(&[TarItem::HardLink("stolen", "../../secret.txt")]);
    assert!(matches!(
        extract(&tar, &dest),
        Err(AcquisitionError::SecurityViolation(_))
    ));

    // A target reached through an (in-root) symlink is rejected.
    let (sandbox2, dest2) = extraction_sandbox("hardlink_via_link");
    let tar = build_tar(&[
        TarItem::File("real/file.txt", b"data"),
        TarItem::Symlink("alias", "real"),
        TarItem::HardLink("copy.txt", "alias/file.txt"),
    ]);
    assert!(matches!(
        extract(&tar, &dest2),
        Err(AcquisitionError::SecurityViolation(_))
    ));

    // A target that does not exist in the destination is rejected.
    let (sandbox3, dest3) = extraction_sandbox("hardlink_missing");
    let tar = build_tar(&[TarItem::HardLink("copy.txt", "missing.txt")]);
    assert!(matches!(
        extract(&tar, &dest3),
        Err(AcquisitionError::SecurityViolation(_))
    ));

    assert_eq!(fs::read(sandbox.join("a/secret.txt")).unwrap(), b"secret");
    assert!(!dest.join("stolen").exists());
    for dir in [sandbox, sandbox2, sandbox3] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// A link that is valid inside the archive's top-level directory can escape
/// once that directory is unwrapped; acquisition must reject it.
#[cfg(unix)]
#[test]
fn test_symlink_escaping_after_top_level_unwrap_is_rejected() {
    let cache_dir = unique_temp_dir("unwrap_escape");
    let tar = build_tar(&[
        TarItem::File("pkg/CMakeLists.txt", b"project(x)"),
        TarItem::Symlink("pkg/link", "../pkg/CMakeLists.txt"),
    ]);
    let checksum = compute_sha256(&tar);
    let url = "https://example.com/unwrap.tar";
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, tar);
    let acquirer = SourceAcquirer::with_transport(SourceCache::new(&cache_dir).unwrap(), transport);
    let id = PackageId::new(
        PackageName::new("unwrap").unwrap(),
        semver::Version::new(1, 0, 0),
    );

    let result = acquirer.acquire_archive(&id, url, &checksum, false);
    assert!(
        matches!(result, Err(AcquisitionError::SecurityViolation(_))),
        "{result:?}"
    );
    let _ = fs::remove_dir_all(&cache_dir);
}

/// Regression: a lockfile checksum that differs from the registry's archive
/// checksum must be rejected before any download is attempted.
#[test]
fn test_cli_fetch_rejects_lockfile_checksum_drift() {
    let _guard = ENV_LOCK.lock().unwrap();

    let seed_registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
    if !seed_registry_path.join("registry.toml").is_file() {
        return;
    }

    let project_dir = unique_temp_dir("cli_fetch_drift_proj");
    let cache_dir = unique_temp_dir("cli_fetch_drift_cache");
    fs::write(
        project_dir.join("crux.toml"),
        "[package]\nname = \"my_app\"\nversion = \"0.1.0\"\n\n[dependencies]\nfmt = \"^11.1\"\n",
    )
    .unwrap();
    let crux_dir = project_dir.join(".crux");
    fs::create_dir_all(&crux_dir).unwrap();
    fs::write(
        crux_dir.join("config.toml"),
        format!(
            "cache_dir = \"{}\"\n\n[registries.default]\npath = \"{}\"\n",
            cache_dir.display(),
            seed_registry_path.display()
        ),
    )
    .unwrap();

    // The registry's fmt 11.1.1 archive digest is a25124e4...; the lockfile
    // records a different digest, as if the registry entry had been swapped.
    let tampered_digest = "b".repeat(64);
    fs::write(
        project_dir.join("crux.lock"),
        format!(
            "lockfile_version = 1\n\n[[package]]\nname = \"fmt\"\nversion = \"11.1.1\"\n\
             source = \"registry+default\"\nchecksum = \"sha256:{tampered_digest}\"\ndependencies = []\n"
        ),
    )
    .unwrap();

    let orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_dir).unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = crux::cli::run(["crux", "fetch"], &mut stdout, &mut stderr);
    std::env::set_current_dir(orig_dir).unwrap();

    let err_str = String::from_utf8(stderr).unwrap();
    assert!(res.is_err(), "fetch must fail on lockfile checksum drift");
    assert!(
        err_str.contains("does not match crux.lock"),
        "unexpected error output: {err_str}"
    );
    // Nothing was downloaded or cached for the tampered digest.
    assert!(!cache_dir.join("archives").join(&tampered_digest).exists());

    let _ = fs::remove_dir_all(&project_dir);
    let _ = fs::remove_dir_all(&cache_dir);
}

// ---------------------------------------------------------------------------
// 2026-11-21 hardening pass: additional adversarial extraction coverage.
// ---------------------------------------------------------------------------

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = flate2::write::GzEncoder::new(&mut out, flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap();
    }
    out
}

fn special_entry_tar(kind: tar::EntryType, path: &str) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(0);
    header.set_mode(0o644);
    builder.append_data(&mut header, path, &[][..]).unwrap();
    builder.into_inner().unwrap()
}

fn acquirer_for(
    cache_dir: &std::path::Path,
    url: &str,
    bytes: Vec<u8>,
) -> (SourceAcquirer, Checksum) {
    let checksum = compute_sha256(&bytes);
    let transport = Arc::new(MockHttpTransport::new());
    transport.add_response(url, bytes);
    (
        SourceAcquirer::with_transport(SourceCache::new(cache_dir).unwrap(), transport),
        checksum,
    )
}

fn pkg_id(name: &str) -> PackageId {
    PackageId::new(
        PackageName::new(name).unwrap(),
        semver::Version::new(1, 0, 0),
    )
}

fn cache_is_clean(cache_dir: &std::path::Path) -> bool {
    let sources = cache_dir.join("sources/sha256");
    let tmp = cache_dir.join("tmp");
    let empty = |dir: &std::path::Path| {
        fs::read_dir(dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    };
    empty(&sources) && empty(&tmp)
}

/// Regression: an archive whose wrapper directory contains a same-named
/// directory (`pkg/pkg/...`) failed to unwrap with `DirectoryNotEmpty`.
#[test]
fn test_nested_same_named_root_unwraps_correctly() {
    let cache_dir = unique_temp_dir("nested_root");
    let tar = gzip(&build_tar(&[
        TarItem::File("pkg/CMakeLists.txt", b"project(pkg)"),
        TarItem::File("pkg/pkg/core.c", b"int core;"),
        TarItem::File("pkg/include/pkg.h", b"int pkg;"),
    ]));
    let url = "https://example.com/pkg-1.0.0.tar.gz";
    let (acquirer, checksum) = acquirer_for(&cache_dir, url, tar);
    let acquired = acquirer
        .acquire_archive(&pkg_id("pkg"), url, &checksum, false)
        .expect("nested same-named root must unwrap");
    assert!(acquired.source_dir.join("CMakeLists.txt").is_file());
    assert!(acquired.source_dir.join("pkg/core.c").is_file());
    assert!(acquired.source_dir.join("include/pkg.h").is_file());
    let _ = fs::remove_dir_all(&cache_dir);
}

/// Regression: a hidden top-level entry was silently overwritten by the
/// same-named entry inside the wrapper directory during unwrapping.
#[test]
fn test_unwrap_collision_with_hidden_root_entry_is_refused() {
    let cache_dir = unique_temp_dir("unwrap_collision");
    let tar = gzip(&build_tar(&[
        TarItem::File(".config", b"root copy"),
        TarItem::File("pkg/.config", b"wrapper copy"),
        TarItem::File("pkg/CMakeLists.txt", b"project(pkg)"),
    ]));
    let url = "https://example.com/collide.tar.gz";
    let (acquirer, checksum) = acquirer_for(&cache_dir, url, tar);
    let err = acquirer
        .acquire_archive(&pkg_id("collide"), url, &checksum, false)
        .unwrap_err();
    assert!(err.to_string().contains("refusing to overwrite"), "{err}");
    assert!(
        cache_is_clean(&cache_dir),
        "failed acquisition must leave no source or temp dir"
    );
    let _ = fs::remove_dir_all(&cache_dir);
}

/// Regression: `tar -C dir .` archives start with a `./` directory entry and
/// were rejected as an "empty path" security violation.
#[test]
fn test_dot_rooted_tarball_is_accepted_but_empty_file_paths_are_not() {
    let (sandbox, dest) = extraction_sandbox("dot_root");
    let tar = build_tar(&[
        TarItem::Dir("./"),
        TarItem::Dir("./sub/"),
        TarItem::File("./sub/a.c", b"int a;"),
    ]);
    extract(&tar, &dest).expect("'./' root entry is valid");
    assert_eq!(fs::read(dest.join("sub/a.c")).unwrap(), b"int a;");

    // A non-directory entry that names the root is still rejected.
    let (sandbox2, dest2) = extraction_sandbox("dot_root_file");
    let tar = special_entry_tar(tar::EntryType::Regular, "./");
    assert!(matches!(
        extract(&tar, &dest2),
        Err(AcquisitionError::SecurityViolation(_))
    ));
    let _ = fs::remove_dir_all(sandbox);
    let _ = fs::remove_dir_all(sandbox2);
}

#[test]
fn test_truncated_and_malformed_archives_fail_without_leaving_state() {
    let full = gzip(&build_tar(&[
        TarItem::File("pkg/a.c", &[b'a'; 40_000]),
        TarItem::File("pkg/b.c", &[b'b'; 40_000]),
    ]));
    let zip = create_test_zip(&[("pkg/a.c", &[b'a'; 40_000])]);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "https://example.com/trunc.tar.gz",
            full[..full.len() / 2].to_vec(),
        ),
        (
            "https://example.com/garbage.tar.gz",
            b"\x1f\x8bnot really gzip".to_vec(),
        ),
        (
            "https://example.com/trunc.zip",
            zip[..zip.len() / 2].to_vec(),
        ),
        ("https://example.com/empty.tar", Vec::new()),
    ];
    for (url, bytes) in cases {
        let cache_dir = unique_temp_dir("malformed");
        let (acquirer, checksum) = acquirer_for(&cache_dir, url, bytes);
        let result = acquirer.acquire_archive(&pkg_id("broken"), url, &checksum, false);
        if let Ok(acquired) = &result {
            // An empty tar is a valid (empty) archive; it must not escape.
            assert!(acquired.source_dir.starts_with(&cache_dir), "{url}");
        } else {
            assert!(
                cache_is_clean(&cache_dir),
                "{url}: leftover state after failure"
            );
        }
        let _ = fs::remove_dir_all(&cache_dir);
    }
}

#[test]
fn test_special_file_entries_are_rejected() {
    for kind in [
        tar::EntryType::Fifo,
        tar::EntryType::Char,
        tar::EntryType::Block,
    ] {
        let (sandbox, dest) = extraction_sandbox("special");
        let err = extract(&special_entry_tar(kind, "pkg/device"), &dest).unwrap_err();
        assert!(
            matches!(err, AcquisitionError::SecurityViolation(_)),
            "{kind:?}: {err}"
        );
        assert!(fs::symlink_metadata(dest.join("pkg/device")).is_err());
        let _ = fs::remove_dir_all(sandbox);
    }
}

#[test]
fn test_conflicting_and_duplicate_entries_are_handled_deterministically() {
    // file then directory with the same name
    let (s1, d1) = extraction_sandbox("file_then_dir");
    let tar = build_tar(&[TarItem::File("x", b"file"), TarItem::Dir("x/")]);
    assert!(extract(&tar, &d1).is_err());
    assert_eq!(fs::read(d1.join("x")).unwrap(), b"file");

    // directory then file with the same name
    let (s2, d2) = extraction_sandbox("dir_then_file");
    let tar = build_tar(&[TarItem::Dir("x/"), TarItem::File("x", b"file")]);
    assert!(extract(&tar, &d2).is_err());
    assert!(fs::symlink_metadata(d2.join("x")).unwrap().is_dir());

    // a file used as a parent directory
    let (s3, d3) = extraction_sandbox("file_as_parent");
    let tar = build_tar(&[TarItem::File("x", b"file"), TarItem::File("x/y", b"child")]);
    assert!(extract(&tar, &d3).is_err());
    assert_eq!(fs::read(d3.join("x")).unwrap(), b"file");

    // duplicate regular files: last entry wins, no other file is touched
    let (s4, d4) = extraction_sandbox("duplicates");
    let tar = build_tar(&[
        TarItem::File("a.txt", b"first"),
        TarItem::File("b.txt", b"other"),
        TarItem::File("a.txt", b"second"),
    ]);
    extract(&tar, &d4).unwrap();
    assert_eq!(fs::read(d4.join("a.txt")).unwrap(), b"second");
    assert_eq!(fs::read(d4.join("b.txt")).unwrap(), b"other");

    for dir in [s1, s2, s3, s4] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn test_traversal_and_absolute_variants_are_rejected_in_tar_and_zip() {
    let evil = [
        "../escape.txt",
        "./../escape.txt",
        "a/./../../escape.txt",
        "a/b/../../../escape.txt",
        "/etc/passwd",
        "//double/slash",
        "C:\\windows\\x",
        "a\\..\\..\\x",
    ];
    for name in evil {
        let (sandbox, dest) = extraction_sandbox("traversal");
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let bytes = name.as_bytes();
        header.as_mut_bytes()[..bytes.len()].copy_from_slice(bytes);
        header.set_size(1);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
        let tar = builder.into_inner().unwrap();
        assert!(
            extract(&tar, &dest).is_err(),
            "tar entry {name:?} must be rejected"
        );

        let zip = create_test_zip(&[(name, b"x")]);
        let zip_path = sandbox.join("evil.zip");
        fs::write(&zip_path, zip).unwrap();
        assert!(
            crux::source::extraction::extract_zip(&zip_path, &dest, &ExtractionLimits::default())
                .is_err(),
            "zip entry {name:?} must be rejected"
        );
        assert!(
            files_outside(&sandbox, &dest)
                .iter()
                .all(|p| p.ends_with("evil.zip")),
            "{name:?} wrote outside the extraction root"
        );
        let _ = fs::remove_dir_all(sandbox);
    }
}

#[test]
fn test_unusual_but_valid_filenames_stay_inside_the_root() {
    let (sandbox, dest) = extraction_sandbox("unusual_names");
    let long = format!("pkg/{}.h", "n".repeat(200));
    let names = [
        "pkg/name with spaces.c",
        "pkg/-leading-dash.c",
        "pkg/...",
        "pkg/ünïcødé.h",
        "pkg/semi;colon$and`tick`.txt",
        long.as_str(),
    ];
    let items: Vec<TarItem<'_>> = names.iter().map(|n| TarItem::File(n, b"ok")).collect();
    extract(&build_tar(&items), &dest).expect("unusual names are valid");
    for name in names {
        assert_eq!(fs::read(dest.join(name)).unwrap(), b"ok", "{name}");
    }
    assert!(files_outside(&sandbox, &dest).is_empty());
    let _ = fs::remove_dir_all(sandbox);
}

/// Hardening: temporary cache paths are unique even under concurrent use
/// within one process, and are created exclusively.
#[test]
fn test_cache_temp_paths_are_unique_under_concurrency() {
    let cache_dir = unique_temp_dir("temp_unique");
    let cache = SourceCache::new(&cache_dir).unwrap();
    let paths = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..25 {
                    let dir = cache.create_temp_dir("staging").unwrap().disarm();
                    let file = cache.create_temp_file("archive").unwrap().disarm();
                    paths.lock().unwrap().extend([dir, file]);
                }
            });
        }
    });
    let mut paths = paths.into_inner().unwrap();
    let total = paths.len();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), total, "temporary paths collided");
    assert!(
        paths
            .iter()
            .all(|p| p.exists() && p.starts_with(cache.tmp_dir()))
    );
    let _ = fs::remove_dir_all(&cache_dir);
}

/// Regression: rewriting `crux.lock` reset restrictive permissions (0600 →
/// 0644 under a 022 umask).
#[cfg(unix)]
#[test]
fn test_lockfile_rewrite_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = unique_temp_dir("lock_perms");
    let mut lockfile = crux::lock::Lockfile::new();
    lockfile
        .add_package(crux::lock::LockedPackage::new(
            PackageName::new("fmt").unwrap(),
            semver::Version::new(11, 1, 1),
            crux::lock::PackageSource::default_registry(),
        ))
        .unwrap();
    let path = lockfile.write_atomic(&dir).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    lockfile.write_atomic(&dir).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(&dir);
}
