//! Core foundational utilities for Crux.

pub mod filesystem;
pub mod paths;

pub use filesystem::{generate_nonce, write_atomically};
pub use paths::{is_safe_relative_path, normalize_path};
