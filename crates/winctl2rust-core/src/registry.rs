use std::fmt;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpServerRegistry};
use guap_table::TableProvider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    Cmd,
    Audio,
    Display,
    Desktop,
}

impl ToolCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cmd => "cmd",
            Self::Audio => "audio",
            Self::Display => "display",
            Self::Desktop => "desktop",
        }
    }
}

impl fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuiteToolInfo {
    pub tool_id: &'static str,
    pub nirsoft_name: &'static str,
    pub category: ToolCategory,
    pub summary: &'static str,
    pub risk_level: ActionRiskLevel,
    pub requires_elevation: bool,
    pub supports_dry_run: bool,
    pub table_ids: &'static [&'static str],
    pub action_ids: &'static [&'static str],
}

static SUITE_CATALOG: &[SuiteToolInfo] = &[
    // -cmd (4 tools, M3.1)
    SuiteToolInfo {
        tool_id: "cmd_nircmd",
        nirsoft_name: "NirCmd",
        category: ToolCategory::Cmd,
        summary: "Parse, dry-run preview, or run non-hooking NirCmd DSL commands",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["cmd_nircmd_preview"],
        action_ids: &["cmd_nircmd_parse", "cmd_nircmd"],
    },
    SuiteToolInfo {
        tool_id: "cmd_run",
        nirsoft_name: "AdvancedRun",
        category: ToolCategory::Cmd,
        summary: "Validate or launch executables with working dir, priority, affinity, window state, and elevation profiles",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["cmd_run_profiles"],
        action_ids: &["cmd_run_validate", "cmd_run"],
    },
    SuiteToolInfo {
        tool_id: "cmd_from_proc",
        nirsoft_name: "RunFromProcess",
        category: ToolCategory::Cmd,
        summary: "Launch a child process with parent attribution via PROC_THREAD_ATTRIBUTE_PARENT_PROCESS",
        risk_level: ActionRiskLevel::PrivilegedMutating,
        requires_elevation: true,
        supports_dry_run: true,
        table_ids: &["cmd_from_proc_candidates"],
        action_ids: &["cmd_from_proc"],
    },
    SuiteToolInfo {
        tool_id: "cmd_atnow",
        nirsoft_name: "AtNow",
        category: ToolCategory::Cmd,
        summary: "Schedule one-shot delayed tasks via Windows Task Scheduler v2 ITaskService",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["cmd_atnow_tasks"],
        action_ids: &["cmd_atnow"],
    },
    // -audio (4 tools, M3.2)
    SuiteToolInfo {
        tool_id: "audio_view",
        nirsoft_name: "SoundVolumeView",
        category: ToolCategory::Audio,
        summary: "Inspect and control CoreAudio endpoints, sessions, and subunits",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["audio_view"],
        action_ids: &["audio_view_control"],
    },
    SuiteToolInfo {
        tool_id: "audio_svcl",
        nirsoft_name: "SoundVolumeCommandLine",
        category: ToolCategory::Audio,
        summary: "Command-line CoreAudio volume, mute, switch, and default-endpoint control",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["audio_svcl_status"],
        action_ids: &["audio_svcl"],
    },
    SuiteToolInfo {
        tool_id: "audio_app_config",
        nirsoft_name: "AppAudioConfig",
        category: ToolCategory::Audio,
        summary: "Inspect and update per-application PropertyStore audio routing configuration",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["audio_app_config"],
        action_ids: &["audio_app_config_update"],
    },
    SuiteToolInfo {
        tool_id: "audio_volumouse",
        nirsoft_name: "Volumouse",
        category: ToolCategory::Audio,
        summary: "Rule-based mouse wheel / modifier volume control via WM_INPUT/WH_MOUSE_LL",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["audio_volumouse_rules"],
        action_ids: &["audio_volumouse"],
    },
    // -display (4 tools, M3.2)
    SuiteToolInfo {
        tool_id: "display_monitors",
        nirsoft_name: "MultiMonitorTool",
        category: ToolCategory::Display,
        summary: "Inspect and configure multi-monitor CCD topology, primary display, and enable/disable state",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["display_monitors"],
        action_ids: &["display_monitors_apply"],
    },
    SuiteToolInfo {
        tool_id: "display_ddc",
        nirsoft_name: "ControlMyMonitor",
        category: ToolCategory::Display,
        summary: "Read and write DDC/CI VCP monitor features with MCCS 2.2a dictionary",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["display_ddc_vcp"],
        action_ids: &["display_ddc_set"],
    },
    SuiteToolInfo {
        tool_id: "display_info",
        nirsoft_name: "MonitorInfoView",
        category: ToolCategory::Display,
        summary: "Enumerate live display targets, EDID serials, and timing capabilities",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["display_info"],
        action_ids: &[],
    },
    SuiteToolInfo {
        tool_id: "display_edid",
        nirsoft_name: "DumpEDID",
        category: ToolCategory::Display,
        summary: "Pure-Rust VESA EDID 1.3/1.4 and CEA-861 binary block parser",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["display_edid"],
        action_ids: &[],
    },
    // -desktop (10 tools, M3.3)
    SuiteToolInfo {
        tool_id: "desktop_props",
        nirsoft_name: "GUIPropView",
        category: ToolCategory::Desktop,
        summary: "Inspect and manipulate top-level and child window properties",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_props"],
        action_ids: &["desktop_props_control"],
    },
    SuiteToolInfo {
        tool_id: "desktop_windows",
        nirsoft_name: "WinLister",
        category: ToolCategory::Desktop,
        summary: "List open windows with placement bounds and minimize/maximize/close actions",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_windows"],
        action_ids: &["desktop_windows_control"],
    },
    SuiteToolInfo {
        tool_id: "desktop_tree",
        nirsoft_name: "WinExplorer",
        category: ToolCategory::Desktop,
        summary: "Walk recursive HWND child window trees and inspect/update control text or bounds",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_tree"],
        action_ids: &["desktop_tree_control"],
    },
    SuiteToolInfo {
        tool_id: "desktop_sysexport",
        nirsoft_name: "SysExporter",
        category: ToolCategory::Desktop,
        summary: "Read SysListView32/SysTreeView32 item text via non-injecting LVM_GETITEMW/TVM_GETITEMW",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["desktop_sysexport"],
        action_ids: &[],
    },
    SuiteToolInfo {
        tool_id: "desktop_clipboard",
        nirsoft_name: "InsideClipboard",
        category: ToolCategory::Desktop,
        summary: "Inspect active clipboard formats and payloads, or clear/set text",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_clipboard"],
        action_ids: &["desktop_clipboard_control"],
    },
    SuiteToolInfo {
        tool_id: "desktop_clip_hist",
        nirsoft_name: "Clipboardic",
        category: ToolCategory::Desktop,
        summary: "Browse, restore, or purge local clipboard history archive",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_clip_hist"],
        action_ids: &["desktop_clip_hist_control"],
    },
    SuiteToolInfo {
        tool_id: "desktop_hotkeys",
        nirsoft_name: "HotKeysList",
        category: ToolCategory::Desktop,
        summary: "Probe registered global hotkeys and decode [MS-SHLLINK] .lnk wHotKey bindings",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["desktop_hotkeys"],
        action_ids: &[],
    },
    SuiteToolInfo {
        tool_id: "desktop_keys",
        nirsoft_name: "KeyboardStateView",
        category: ToolCategory::Desktop,
        summary: "Inspect virtual key states, scan codes, and localized key names",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["desktop_keys"],
        action_ids: &[],
    },
    SuiteToolInfo {
        tool_id: "desktop_fonts",
        nirsoft_name: "WinFontsView",
        category: ToolCategory::Desktop,
        summary: "Enumerate installed fonts and parse OpenType name/OS2 tables",
        risk_level: ActionRiskLevel::ReadOnly,
        requires_elevation: false,
        supports_dry_run: false,
        table_ids: &["desktop_fonts"],
        action_ids: &[],
    },
    SuiteToolInfo {
        tool_id: "desktop_webcam",
        nirsoft_name: "WebCamImageSave",
        category: ToolCategory::Desktop,
        summary: "Enumerate Media Foundation video capture sources and capture still frames",
        risk_level: ActionRiskLevel::Mutating,
        requires_elevation: false,
        supports_dry_run: true,
        table_ids: &["desktop_webcam"],
        action_ids: &["desktop_webcam_capture"],
    },
];

pub fn builtin_suite_catalog() -> &'static [SuiteToolInfo] {
    SUITE_CATALOG
}

#[derive(Default)]
pub struct ToolBundle {
    pub providers: Vec<Arc<dyn TableProvider>>,
    pub actions: Vec<Arc<dyn ActionCommand>>,
    pub tools: Vec<SuiteToolInfo>,
}

impl ToolBundle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn merge(&mut self, other: ToolBundle) {
        self.providers.extend(other.providers);
        self.actions.extend(other.actions);
        self.tools.extend(other.tools);
    }

    pub fn register_into(&self, registry: &mut McpServerRegistry) {
        for provider in &self.providers {
            registry.register_table(Arc::clone(provider));
        }
        for action in &self.actions {
            registry.register_action(Arc::clone(action));
        }
    }

    pub fn find_provider(&self, tool_id: &str) -> Option<Arc<dyn TableProvider>> {
        self.providers.iter().find(|p| p.tool_id() == tool_id).cloned()
    }

    pub fn find_action(&self, command_id: &str) -> Option<Arc<dyn ActionCommand>> {
        self.actions.iter().find(|a| a.command_id() == command_id).cloned()
    }
}
