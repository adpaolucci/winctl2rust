use guap_mcp::ActionRiskLevel;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{WinctlError, WinctlResult};

pub const R4_FORBIDDEN_IDENTIFIERS: &[&str] = &[
    "runasdate",
    "cred2rust",
    "creddump",
    "sniffpass",
    "lsass",
    "samdump",
    "mimikatz",
    "createremotethread",
    "ntcreatethreadex",
    "queueuserapc",
    "setwindowshookex",
    "socketsniff",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOptions {
    pub dry_run: bool,
    pub allow_mutating: bool,
    pub confirm_destructive: bool,
}

impl ExecutionOptions {
    pub fn read_only() -> Self {
        Self::default()
    }

    pub fn dry_run_only() -> Self {
        Self {
            dry_run: true,
            allow_mutating: false,
            confirm_destructive: false,
        }
    }

    pub fn allow_all_mutating() -> Self {
        Self {
            dry_run: false,
            allow_mutating: true,
            confirm_destructive: true,
        }
    }

    pub fn can_mutate(self) -> bool {
        !self.dry_run && self.allow_mutating && self.confirm_destructive
    }

    pub fn validate_action(self, action_id: &str, risk: ActionRiskLevel) -> WinctlResult<()> {
        if risk == ActionRiskLevel::ReadOnly || self.dry_run {
            return Ok(());
        }
        if !self.allow_mutating {
            return Err(WinctlError::PolicyDenied(format!(
                "action '{action_id}' is mutating ({risk:?}) and requires allow_mutating=true or dry_run=true"
            )));
        }
        if !self.confirm_destructive {
            return Err(WinctlError::PolicyDenied(format!(
                "action '{action_id}' is mutating ({risk:?}) and requires confirm_destructive=true"
            )));
        }
        Ok(())
    }
}

pub fn validate_r4_compliance(program_or_verb: &str, args: &[String]) -> WinctlResult<()> {
    check_r4_token("target", program_or_verb)?;
    for (idx, arg) in args.iter().enumerate() {
        check_r4_token(&format!("arg[{idx}]"), arg)?;
    }
    Ok(())
}

fn check_r4_token(field: &str, value: &str) -> WinctlResult<()> {
    let lower = value.to_ascii_lowercase();
    for forbidden in R4_FORBIDDEN_IDENTIFIERS {
        if lower.contains(forbidden) {
            return Err(WinctlError::PolicyDenied(format!(
                "R4 compliance violation in {field}: prohibited identifier '{forbidden}' detected"
            )));
        }
    }
    Ok(())
}

pub fn dry_run_preview(command_id: &str, description: &str, planned_steps: Value) -> Value {
    json!({
        "dry_run": true,
        "applied": false,
        "command_id": command_id,
        "description": description,
        "plan": planned_steps,
    })
}
