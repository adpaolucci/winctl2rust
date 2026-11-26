#![forbid(unsafe_code)]

pub mod error;
pub mod logging;
pub mod policy;
pub mod privilege;
pub mod registry;

pub use error::{WinctlError, WinctlResult};
pub use logging::init_logging;
pub use policy::{ExecutionOptions, R4_FORBIDDEN_IDENTIFIERS, dry_run_preview, validate_r4_compliance};
pub use privilege::{DefaultPrivilegeProbe, PrivilegeProbe, PrivilegeStatus};
pub use registry::{SuiteToolInfo, ToolBundle, ToolCategory, builtin_suite_catalog};
