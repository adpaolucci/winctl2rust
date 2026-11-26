use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivilegeStatus {
    pub is_elevated: bool,
    pub platform: String,
    pub windows_supported: bool,
    pub integrity_level: String,
    pub notes: Vec<String>,
}

pub trait PrivilegeProbe: Send + Sync {
    fn probe(&self) -> PrivilegeStatus;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultPrivilegeProbe;

impl PrivilegeProbe for DefaultPrivilegeProbe {
    fn probe(&self) -> PrivilegeStatus {
        let windows_supported = cfg!(windows);
        let mut notes = Vec::new();
        if windows_supported {
            notes.push(
                "Safe default probe does not inspect process token directly; OS backends verify elevation per call"
                    .to_string(),
            );
        } else {
            notes.push("Non-Windows host: live Win32/COM execution will return UnsupportedOnPlatform".to_string());
        }
        PrivilegeStatus {
            is_elevated: false,
            platform: std::env::consts::OS.to_string(),
            windows_supported,
            integrity_level: if windows_supported {
                "Unknown (unprobed)".to_string()
            } else {
                "NonWindows".to_string()
            },
            notes,
        }
    }
}
