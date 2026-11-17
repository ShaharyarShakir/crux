//! Subcommand implementations for the Crux CLI.

pub mod add;
pub mod build;
pub mod check;
pub mod dependency_edit;
pub mod fetch;
pub mod init;
pub mod install;
pub mod integrate;
pub mod lock;
pub mod registry;
pub mod remove;
pub mod resolve;
pub mod run;
pub mod toolchain;

pub use add::run_add;
pub use build::{run_build, sort_packages_topologically};
pub use check::run_check;
pub use fetch::run_fetch;
pub use init::run_init;
pub use install::run_install;
pub use integrate::run_integrate_cmake;
pub use lock::{run_lock, run_lock_check};
pub use registry::{run_registry_add, run_registry_list, run_registry_update};
pub use remove::run_remove;
pub use resolve::run_resolve;
pub use run::run_run;
pub use toolchain::run_toolchain;
