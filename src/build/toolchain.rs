//! Build toolchain detection and environment inspection.

/// Information about the detected build toolchain.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolchainInfo {
    pub cmake_version: String,
    pub c_compiler_version: Option<String>,
    pub cxx_compiler_version: Option<String>,
    pub generator: String,
}
