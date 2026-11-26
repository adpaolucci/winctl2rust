#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use guap_export::{ExportFormat, build_custody_manifest, sha256_hex, sha512_hex};
use guap_mcp::{ActionRiskLevel, PolicyMode};
use guap_table::{GetNirQuery, TableData, TableProvider, apply_query};
use serde_json::{Value, json};
use winctl2rust_audio::backend::{PointerWheelEvent, VolumouseRule};
use winctl2rust_audio::svcl::{parse_svcl_command, parse_svcl_tokens};
use winctl2rust_core::{
    ExecutionOptions, ToolBundle, ToolCategory, WinctlError, builtin_suite_catalog, validate_r4_compliance,
};
use winctl2rust_desktop::fonts::{font_entry_to_row, fonts_table_schema};
use winctl2rust_desktop::hotkeys::{hotkey_to_row, hotkeys_table_schema};
use winctl2rust_desktop::lnk::parse_lnk_file;
use winctl2rust_desktop::opentype::parse_font_bytes;
use winctl2rust_display::edid::{edid_table_from_bytes, edid_table_from_hex, parse_edid_input};
use winctl2rust_mcp::{build_registry_from_bundle, collect_default_bundle};
use winctl2rust_report::{
    WinctlReportOptions, append_audit_jsonl, record_action_audit, render_table, write_report, write_report_bundle,
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum CliOutputFormat {
    #[default]
    Table,
    Csv,
    Tsv,
    Json,
    Jsonl,
    Html,
    HtmlVertical,
    Markdown,
}

impl CliOutputFormat {
    pub fn to_export_format(self) -> Option<ExportFormat> {
        match self {
            Self::Table => None,
            Self::Csv => Some(ExportFormat::Csv),
            Self::Tsv => Some(ExportFormat::Tsv),
            Self::Json => Some(ExportFormat::Json),
            Self::Jsonl => Some(ExportFormat::Jsonl),
            Self::Html => Some(ExportFormat::Html),
            Self::HtmlVertical => Some(ExportFormat::HtmlVertical),
            Self::Markdown => Some(ExportFormat::Markdown),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliCategory {
    Cmd,
    Audio,
    Display,
    Desktop,
}

impl From<CliCategory> for ToolCategory {
    fn from(value: CliCategory) -> Self {
        match value {
            CliCategory::Cmd => ToolCategory::Cmd,
            CliCategory::Audio => ToolCategory::Audio,
            CliCategory::Display => ToolCategory::Display,
            CliCategory::Desktop => ToolCategory::Desktop,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "winctl2rust",
    author = "Winctl2Rust Contributors",
    version,
    about = "100% pure-Rust Windows control, audio, display DDC/CI & EDID, desktop automation, export, dual GUI, and MCP CLI suite (22 non-hooking tools)",
    long_about = "Replace 22 non-hooking NirSoft Windows control, CoreAudio, display/EDID, and desktop utilities in pure Rust.\n\nMutating operations are gated behind --dry-run (preview) or --allow-mutating --confirm-destructive."
)]
pub struct Cli {
    /// Logging verbosity level written strictly to stderr (error, warn, info, debug, trace)
    #[arg(short = 'l', long, global = true, default_value = "warn")]
    pub log_level: String,

    /// Preview mutating operations without modifying OS state
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Permit mutating and privileged-mutating commands (must be paired with --confirm-destructive)
    #[arg(long, global = true)]
    pub allow_mutating: bool,

    /// Explicit destructive-action confirmation required when applying mutations
    #[arg(long, global = true)]
    pub confirm_destructive: bool,

    /// Output format for tabular views and exports
    #[arg(long, global = true, value_enum, default_value_t = CliOutputFormat::Table)]
    pub format: CliOutputFormat,

    /// Destination file or directory path for output / exports
    #[arg(short = 'o', long, global = true)]
    pub output: Option<PathBuf>,

    /// Write SHA-256/SHA-512 custody sidecar or JSONL action audit record to PATH
    #[arg(long, global = true)]
    pub custody: Option<PathBuf>,

    /// GetNir filter expression (e.g. "active == true && width >= 1920")
    #[arg(long, global = true)]
    pub filter: Option<String>,

    /// Comma-separated sort columns (e.g. "name:asc,pid:desc" or "-volume_percent")
    #[arg(long, global = true)]
    pub sort: Option<String>,

    /// Maximum number of table rows to return
    #[arg(long, global = true)]
    pub limit: Option<usize>,

    /// Starting row offset for table pagination
    #[arg(long, global = true)]
    pub offset: Option<usize>,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn execution_options(&self) -> ExecutionOptions {
        ExecutionOptions {
            dry_run: self.dry_run,
            allow_mutating: self.allow_mutating,
            confirm_destructive: self.confirm_destructive,
        }
    }

    pub fn table_query(&self) -> Result<GetNirQuery> {
        let sort = match &self.sort {
            Some(spec) => GetNirQuery::parse_sort(spec).map_err(|e| anyhow!(e))?,
            None => Vec::new(),
        };
        Ok(GetNirQuery {
            columns: None,
            filter_expr: self.filter.clone(),
            sort,
            offset: self.offset.unwrap_or(0),
            limit: self.limit,
        })
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Non-hooking command launcher, AdvancedRun profiles, RunFromProcess, and AtNow scheduler
    Cmd {
        #[command(subcommand)]
        action: CmdSubcommand,
    },
    /// CoreAudio endpoint/session control, svcl, AppAudioConfig, and Volumouse rules
    Audio {
        #[command(subcommand)]
        action: AudioSubcommand,
    },
    /// MultiMonitorTool topology, DDC/CI VCP, MonitorInfoView, and DumpEDID parser
    Display {
        #[command(subcommand)]
        action: DisplaySubcommand,
    },
    /// GUIPropView, WinLister, WinExplorer, SysExporter, Clipboard, HotKeys, Keys, Fonts, and WebCam
    Desktop {
        #[command(subcommand)]
        action: DesktopSubcommand,
    },
    /// Export any single tool or category/suite bundle with SHA-256/SHA-512 custody sidecars
    Export(ExportArgs),
    /// Run policy-gated JSON-RPC 2.0 stdio MCP server or print the 40-tool manifest
    Mcp(McpArgs),
    /// Launch native Win32 (winctl2rust-win32) or cross-platform Iced (winctl2rust-iced) GUI
    Gui(GuiArgs),
    /// List all 22 built-in tools and their capabilities
    Catalog(CatalogArgs),
}

#[derive(Subcommand, Debug)]
pub enum CmdSubcommand {
    /// Parse, preview, or execute non-hooking NirCmd DSL scripts
    Nircmd(NirCmdArgs),
    /// Validate or launch executables with AdvancedRun INI/JSON profiles
    Run(RunArgs),
    /// Enumerate parent candidates or spawn a child via PROC_THREAD_ATTRIBUTE_PARENT_PROCESS
    FromProc(FromProcArgs),
    /// List scheduled tasks or schedule a one-shot delayed task via Task Scheduler v2
    Atnow(AtnowArgs),
}

#[derive(Args, Debug, Default)]
pub struct NirCmdArgs {
    /// Inline NirCmd DSL script (newline-separated commands)
    #[arg(long, conflicts_with = "script_file")]
    pub script: Option<String>,
    /// Path to a NirCmd DSL script file
    #[arg(long, value_name = "PATH")]
    pub script_file: Option<PathBuf>,
    /// Template variable substitution KEY=VAL (repeatable)
    #[arg(long = "var", value_name = "KEY=VAL")]
    pub vars: Vec<String>,
    /// Parse and validate script read-only without executing
    #[arg(long)]
    pub parse_only: bool,
}

#[derive(Args, Debug, Default)]
pub struct RunArgs {
    /// Executable path to launch
    #[arg(long, value_name = "PATH")]
    pub exe: Option<PathBuf>,
    /// Arguments passed to the target executable
    #[arg(long, num_args = 1..)]
    pub args: Vec<String>,
    /// Working directory
    #[arg(long, value_name = "DIR")]
    pub work_dir: Option<PathBuf>,
    /// Elevation mode: normal | administrator | current_user | system
    #[arg(long)]
    pub run_as: Option<String>,
    /// Process priority class: low | below_normal | normal | above_normal | high | realtime
    #[arg(long)]
    pub priority: Option<String>,
    /// CPU affinity bitmask (decimal or 0x hex)
    #[arg(long)]
    pub affinity: Option<String>,
    /// Initial window state: normal | minimized | maximized | hidden
    #[arg(long)]
    pub window_state: Option<String>,
    /// __COMPAT_LAYER flags (repeatable)
    #[arg(long)]
    pub compat: Vec<String>,
    /// Environment variable override KEY=VAL (repeatable)
    #[arg(long = "env", value_name = "KEY=VAL")]
    pub env: Vec<String>,
    /// AdvancedRun INI/CFG profile file
    #[arg(long, value_name = "FILE")]
    pub ini: Option<PathBuf>,
    /// AdvancedRun JSON LaunchProfile file
    #[arg(long, value_name = "JSON_FILE")]
    pub profile: Option<PathBuf>,
    /// Validate profile read-only without launching
    #[arg(long)]
    pub validate: bool,
}

#[derive(Args, Debug, Default)]
pub struct FromProcArgs {
    /// Target parent process PID
    #[arg(long)]
    pub parent_pid: Option<u32>,
    /// Child executable path to spawn
    #[arg(long)]
    pub child_exe: Option<String>,
    /// Arguments passed to the child process
    #[arg(long, num_args = 1..)]
    pub args: Vec<String>,
    /// Working directory for child process
    #[arg(long)]
    pub working_dir: Option<String>,
    /// Inherit parent handles
    #[arg(long)]
    pub inherit_handles: bool,
    /// Allocate a new console window
    #[arg(long)]
    pub new_console: bool,
}

#[derive(Args, Debug, Default)]
pub struct AtnowArgs {
    /// Scheduled task name
    #[arg(long)]
    pub task_name: Option<String>,
    /// Command executable path
    #[arg(long)]
    pub command: Option<String>,
    /// Schedule specification ('now', '+Ns/+Nm/+Nh', 'HH:MM', or ISO-8601)
    #[arg(long)]
    pub when: Option<String>,
    /// Optional command arguments
    #[arg(long)]
    pub arguments: Option<String>,
    /// Optional working directory
    #[arg(long)]
    pub working_dir: Option<String>,
    /// Register task under LocalSystem service account
    #[arg(long)]
    pub run_as_system: bool,
    /// Use non-interactive logon (S4U)
    #[arg(long)]
    pub non_interactive: bool,
    /// Request highest privileges (TASK_RUNLEVEL_HIGHEST)
    #[arg(long)]
    pub highest_privileges: bool,
    /// Task Scheduler folder path
    #[arg(long)]
    pub folder_path: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum AudioSubcommand {
    /// Inspect or control CoreAudio endpoints, sessions, and subunits (SoundVolumeView)
    View(AudioViewArgs),
    /// Parse or execute SoundVolumeCommandLine (svcl) verbs
    Svcl(AudioSvclArgs),
    /// Inspect or update per-application PropertyStore audio blobs (AppAudioConfig)
    AppConfig(AudioAppConfigArgs),
    /// Inspect, validate, evaluate, or run Volumouse rules
    Volumouse(AudioVolumouseArgs),
}

#[derive(Args, Debug, Default)]
pub struct AudioViewArgs {
    /// Control action: set_volume | set_channel_volumes | set_volume_db | set_mute | set_default | enable | disable | set_app_default
    #[arg(long)]
    pub action: Option<String>,
    /// Endpoint ID, friendly name, process name, PID, or DefaultRenderDevice
    #[arg(long)]
    pub target: Option<String>,
    /// Scalar volume percentage (0..=100)
    #[arg(long)]
    pub volume_percent: Option<f32>,
    /// Comma-separated per-channel volumes (0..=100)
    #[arg(long, value_delimiter = ',')]
    pub channel_volumes: Vec<f32>,
    /// Volume in decibels
    #[arg(long, allow_hyphen_values = true)]
    pub volume_db: Option<f32>,
    /// Mute mode: mute | unmute | toggle
    #[arg(long)]
    pub mute_mode: Option<String>,
    /// Comma-separated default roles (console, multimedia, communications, all)
    #[arg(long, value_delimiter = ',')]
    pub roles: Vec<String>,
    /// Application process/PID for set_app_default
    #[arg(long)]
    pub app_target: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct AudioSvclArgs {
    /// Complete svcl command line string (e.g. '/SetVolume "Speakers" 50')
    #[arg(long)]
    pub command_line: Option<String>,
    /// Structured svcl verb (e.g. SetVolume, GetPercent)
    #[arg(long)]
    pub verb: Option<String>,
    /// Structured svcl target selector
    #[arg(long)]
    pub target: Option<String>,
    /// Raw trailing svcl tokens (e.g. /GetPercent Speakers)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub svcl_args: Vec<String>,
}

#[derive(Args, Debug, Default)]
pub struct AudioAppConfigArgs {
    /// PropertyStore subkey name (e.g. 186430e1_0)
    #[arg(long)]
    pub subkey: Option<String>,
    /// Volume percentage (0..=100)
    #[arg(long)]
    pub volume_percent: Option<f32>,
    /// Comma-separated channel balance scalars (0.0..=1.0)
    #[arg(long, value_delimiter = ',')]
    pub channel_balances: Vec<f32>,
    /// Mute state override (true or false)
    #[arg(long)]
    pub muted: Option<bool>,
    /// Delete the PropertyStore subkey
    #[arg(long)]
    pub delete_subkey: bool,
}

#[derive(Args, Debug, Default)]
pub struct AudioVolumouseArgs {
    /// Mode: validate_rules | evaluate_events | apply_rule | start_listener
    #[arg(long)]
    pub mode: Option<String>,
    /// JSON file containing Vec<VolumouseRule>
    #[arg(long, value_name = "JSON")]
    pub rules_file: Option<PathBuf>,
    /// JSON file containing Vec<PointerWheelEvent>
    #[arg(long, value_name = "JSON")]
    pub events_file: Option<PathBuf>,
    /// Initial volume percentage for event stream evaluation
    #[arg(long)]
    pub initial_volume: Option<f32>,
    /// Apply matched rules to live audio backend
    #[arg(long)]
    pub apply_to_backend: bool,
    /// Listener duration in milliseconds
    #[arg(long)]
    pub duration_ms: Option<u64>,
    /// Maximum events before stopping listener
    #[arg(long)]
    pub max_events: Option<usize>,
}

#[derive(Subcommand, Debug)]
pub enum DisplaySubcommand {
    /// Inspect or configure multi-monitor CCD topology (MultiMonitorTool)
    Monitors(DisplayMonitorsArgs),
    /// Read or write DDC/CI VCP monitor features (ControlMyMonitor)
    Ddc(DisplayDdcArgs),
    /// Enumerate live display targets and correlated EDID metadata (MonitorInfoView)
    Info,
    /// Parse VESA EDID 1.3/1.4 + CEA-861 blocks from live registry or offline .bin/.hex/.reg files (DumpEDID)
    Edid(DisplayEdidArgs),
}

#[derive(Args, Debug, Default)]
pub struct DisplayMonitorsArgs {
    /// Operation: set_primary | enable | disable | move | set_resolution | set_orientation | save_config | load_config
    #[arg(long)]
    pub operation: Option<String>,
    /// Monitor device name, Monitor ID, or friendly name
    #[arg(long)]
    pub target: Option<String>,
    /// Horizontal position for move
    #[arg(long, allow_hyphen_values = true)]
    pub x: Option<i32>,
    /// Vertical position for move
    #[arg(long, allow_hyphen_values = true)]
    pub y: Option<i32>,
    /// Resolution width
    #[arg(long)]
    pub width: Option<u32>,
    /// Resolution height
    #[arg(long)]
    pub height: Option<u32>,
    /// Refresh rate in Hz
    #[arg(long)]
    pub refresh_hz: Option<u32>,
    /// Bits per pixel
    #[arg(long)]
    pub bpp: Option<u32>,
    /// Orientation in degrees (0, 90, 180, 270)
    #[arg(long)]
    pub orientation: Option<u32>,
    /// Profile file path for save_config/load_config
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Monitor profile serialization format: json | ini
    #[arg(long)]
    pub config_format: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct DisplayDdcArgs {
    /// Monitor ID, device name, or friendly name
    #[arg(long)]
    pub monitor: Option<String>,
    /// VCP code (e.g. 0x10 or 16)
    #[arg(long)]
    pub vcp_code: Option<String>,
    /// New VCP value (0..=65535)
    #[arg(long)]
    pub value: Option<u32>,
}

#[derive(Args, Debug, Default)]
pub struct DisplayEdidArgs {
    /// Offline EDID binary (.bin), hex dump, DumpEDID text, or Windows .reg export
    #[arg(long, conflicts_with = "hex")]
    pub file: Option<PathBuf>,
    /// Inline EDID hex string (128+ byte multiples)
    #[arg(long)]
    pub hex: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum DesktopSubcommand {
    /// Inspect or control window properties (GUIPropView)
    Props(DesktopPropsArgs),
    /// List open windows or change placement/state (WinLister)
    Windows(DesktopWindowsArgs),
    /// Walk recursive HWND child trees or update control text/bounds (WinExplorer)
    Tree(DesktopTreeArgs),
    /// Read SysListView32/SysTreeView32 item text via non-injecting messages (SysExporter)
    Sysexport,
    /// Inspect, clear, or set active clipboard formats (InsideClipboard)
    Clipboard(DesktopClipboardArgs),
    /// Browse, capture, restore, delete, or export/import clipboard history (Clipboardic)
    ClipHist(DesktopClipHistArgs),
    /// Probe registered global hotkeys or decode an offline .lnk shortcut (HotKeysList)
    Hotkeys(DesktopHotkeysArgs),
    /// Inspect virtual key states, scan codes, and key names (KeyboardStateView)
    Keys,
    /// Enumerate installed fonts or parse an offline .ttf/.otf/.ttc font file (WinFontsView)
    Fonts(DesktopFontsArgs),
    /// List Media Foundation cameras or capture a BMP/PPM frame (WebCamImageSave)
    Webcam(DesktopWebcamArgs),
}

#[derive(Args, Debug, Default)]
pub struct DesktopPropsArgs {
    /// Operation: show | hide | enable | disable | topmost | untopmost | close | set_opacity
    #[arg(long)]
    pub operation: Option<String>,
    /// Target HWND (hex 0x... or decimal)
    #[arg(long)]
    pub hwnd: Option<String>,
    /// Alpha opacity (0..=255) for set_opacity
    #[arg(long)]
    pub opacity: Option<u16>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopWindowsArgs {
    /// Operation: minimize | maximize | restore | close | focus
    #[arg(long)]
    pub operation: Option<String>,
    /// Target top-level HWND (hex 0x... or decimal)
    #[arg(long)]
    pub hwnd: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopTreeArgs {
    /// Operation: set_text | resize | move | show | hide
    #[arg(long)]
    pub operation: Option<String>,
    /// Target child/window HWND (hex 0x... or decimal)
    #[arg(long)]
    pub hwnd: Option<String>,
    /// New text for set_text
    #[arg(long)]
    pub text: Option<String>,
    /// X coordinate for move
    #[arg(long, allow_hyphen_values = true)]
    pub x: Option<i32>,
    /// Y coordinate for move
    #[arg(long, allow_hyphen_values = true)]
    pub y: Option<i32>,
    /// Width for resize
    #[arg(long)]
    pub width: Option<i32>,
    /// Height for resize
    #[arg(long)]
    pub height: Option<i32>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopClipboardArgs {
    /// Operation: clear | set_text
    #[arg(long)]
    pub operation: Option<String>,
    /// UTF-8 text to copy to clipboard for set_text
    #[arg(long)]
    pub text: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopClipHistArgs {
    /// Operation: capture_current | restore | delete | clear | export_json | import_json
    #[arg(long)]
    pub operation: Option<String>,
    /// Archive entry ID for restore/delete
    #[arg(long)]
    pub entry_id: Option<u64>,
    /// JSON file path for export_json/import_json
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Override SQLite clipboard archive path
    #[arg(long)]
    pub db_path: Option<PathBuf>,
    /// Wait for WM_CLIPBOARDUPDATE before capturing
    #[arg(long)]
    pub wait_for_update: bool,
    /// Timeout in milliseconds for wait_for_update
    #[arg(long)]
    pub timeout_ms: Option<u32>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopHotkeysArgs {
    /// Offline [MS-SHLLINK] .lnk shortcut file to decode
    #[arg(long, value_name = "PATH")]
    pub lnk: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopFontsArgs {
    /// Offline TrueType/OpenType (.ttf/.otf/.ttc) font file to parse
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct DesktopWebcamArgs {
    /// Destination image path (.bmp or .ppm) to capture a frame
    #[arg(long)]
    pub output_path: Option<PathBuf>,
    /// Zero-based Media Foundation device index
    #[arg(long)]
    pub device_index: Option<u32>,
    /// Still image format: bmp | ppm
    #[arg(long)]
    pub image_format: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct ExportArgs {
    /// Specific TableProvider ID (e.g. display_edid, audio_view)
    #[arg(long, conflicts_with = "all")]
    pub tool: Option<String>,
    /// Export all 22 TableProviders into a bundle directory
    #[arg(long)]
    pub all: bool,
    /// Filter bundle export by tool category
    #[arg(long, value_enum)]
    pub category: Option<CliCategory>,
    /// Skip writing .custody.json / custody_manifest.json sidecar
    #[arg(long)]
    pub no_custody_sidecar: bool,
}

#[derive(Args, Debug, Default)]
pub struct McpArgs {
    /// Print JSON Schema manifest of all 40 MCP tools and exit
    #[arg(long)]
    pub tools: bool,
    /// Enable mutating and privileged-mutating MCP actions (still requires confirm_destructive=true)
    #[arg(long)]
    pub allow_mutating: bool,
}

#[derive(Args, Debug, Default)]
pub struct GuiArgs {
    /// Launch the cross-platform Iced 0.14 GUI instead of the native Win32 GUI
    #[arg(long)]
    pub iced: bool,
}

#[derive(Args, Debug, Default)]
pub struct CatalogArgs {
    /// Emit the 22-tool catalog as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn run_cli(cli: &Cli) -> Result<()> {
    let bundle = collect_default_bundle();
    run_cli_with_bundle(cli, &bundle)
}

pub fn run_cli_with_bundle(cli: &Cli, bundle: &ToolBundle) -> Result<()> {
    match &cli.command {
        Commands::Cmd { action } => match action {
            CmdSubcommand::Nircmd(args) => handle_cmd_nircmd(cli, bundle, args),
            CmdSubcommand::Run(args) => handle_cmd_run(cli, bundle, args),
            CmdSubcommand::FromProc(args) => handle_cmd_from_proc(cli, bundle, args),
            CmdSubcommand::Atnow(args) => handle_cmd_atnow(cli, bundle, args),
        },
        Commands::Audio { action } => match action {
            AudioSubcommand::View(args) => handle_audio_view(cli, bundle, args),
            AudioSubcommand::Svcl(args) => handle_audio_svcl(cli, bundle, args),
            AudioSubcommand::AppConfig(args) => handle_audio_app_config(cli, bundle, args),
            AudioSubcommand::Volumouse(args) => handle_audio_volumouse(cli, bundle, args),
        },
        Commands::Display { action } => match action {
            DisplaySubcommand::Monitors(args) => handle_display_monitors(cli, bundle, args),
            DisplaySubcommand::Ddc(args) => handle_display_ddc(cli, bundle, args),
            DisplaySubcommand::Info => query_and_emit_provider(cli, bundle, "display_info"),
            DisplaySubcommand::Edid(args) => handle_display_edid(cli, bundle, args),
        },
        Commands::Desktop { action } => match action {
            DesktopSubcommand::Props(args) => handle_desktop_props(cli, bundle, args),
            DesktopSubcommand::Windows(args) => handle_desktop_windows(cli, bundle, args),
            DesktopSubcommand::Tree(args) => handle_desktop_tree(cli, bundle, args),
            DesktopSubcommand::Sysexport => query_and_emit_provider(cli, bundle, "desktop_sysexport"),
            DesktopSubcommand::Clipboard(args) => handle_desktop_clipboard(cli, bundle, args),
            DesktopSubcommand::ClipHist(args) => handle_desktop_clip_hist(cli, bundle, args),
            DesktopSubcommand::Hotkeys(args) => handle_desktop_hotkeys(cli, bundle, args),
            DesktopSubcommand::Keys => query_and_emit_provider(cli, bundle, "desktop_keys"),
            DesktopSubcommand::Fonts(args) => handle_desktop_fonts(cli, bundle, args),
            DesktopSubcommand::Webcam(args) => handle_desktop_webcam(cli, bundle, args),
        },
        Commands::Export(args) => handle_export(cli, bundle, args),
        Commands::Mcp(args) => handle_mcp(bundle, args),
        Commands::Gui(args) => handle_gui(args),
        Commands::Catalog(args) => handle_catalog(cli, args),
    }
}

fn handle_cmd_nircmd(cli: &Cli, bundle: &ToolBundle, args: &NirCmdArgs) -> Result<()> {
    let script_opt = match (&args.script, &args.script_file) {
        (Some(s), _) => Some(s.clone()),
        (None, Some(path)) => {
            Some(fs::read_to_string(path).with_context(|| format!("failed reading script file {}", path.display()))?)
        }
        (None, None) => None,
    };

    let Some(script) = script_opt else {
        if args.parse_only {
            bail!("--parse-only requires --script or --script-file");
        }
        return query_and_emit_provider(cli, bundle, "cmd_nircmd_preview");
    };

    let vars = parse_kv_pairs(&args.vars, "--var")?;
    let payload = json!({ "script": script, "vars": vars });

    if args.parse_only {
        execute_cli_action(
            cli,
            bundle,
            "cmd_nircmd_parse",
            payload,
            Some(ActionRiskLevel::ReadOnly),
        )
    } else {
        execute_cli_action(cli, bundle, "cmd_nircmd", payload, None)
    }
}

fn handle_cmd_run(cli: &Cli, bundle: &ToolBundle, args: &RunArgs) -> Result<()> {
    let has_launch_input = args.exe.is_some() || args.ini.is_some() || args.profile.is_some();
    if !has_launch_input {
        if args.validate {
            bail!("--validate requires --exe, --ini, or --profile");
        }
        return query_and_emit_provider(cli, bundle, "cmd_run_profiles");
    }

    let payload = if let Some(ini_path) = &args.ini {
        let ini = fs::read_to_string(ini_path)
            .with_context(|| format!("failed reading AdvancedRun INI file {}", ini_path.display()))?;
        json!({ "ini": ini })
    } else if let Some(profile_path) = &args.profile {
        let text = fs::read_to_string(profile_path)
            .with_context(|| format!("failed reading LaunchProfile JSON {}", profile_path.display()))?;
        let val: Value = serde_json::from_str(&text)
            .with_context(|| format!("invalid LaunchProfile JSON in {}", profile_path.display()))?;
        if val.get("profile").is_some() {
            val
        } else {
            json!({ "profile": val })
        }
    } else {
        let exe_path = args
            .exe
            .as_ref()
            .ok_or_else(|| anyhow!("missing --exe path"))?
            .display()
            .to_string();
        let env_overrides = parse_kv_pairs(&args.env, "--env")?;
        let affinity_mask = match &args.affinity {
            Some(raw) => Some(parse_affinity_mask(raw)?),
            None => None,
        };
        let run_as_mode = match args.run_as.as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("administrator") || s.eq_ignore_ascii_case("admin") => "Administrator",
            Some(s) if s.eq_ignore_ascii_case("system") => {
                bail!("--run-as system is not supported by AdvancedRun; use `cmd atnow --run-as-system`")
            }
            Some(s)
                if s.eq_ignore_ascii_case("normal")
                    || s.eq_ignore_ascii_case("current_user")
                    || s.eq_ignore_ascii_case("current") =>
            {
                "CurrentUser"
            }
            Some(other) => bail!("unsupported --run-as value '{other}'"),
            None => "CurrentUser",
        };
        let priority_class = match args.priority.as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("low") || s.eq_ignore_ascii_case("idle") => "Idle",
            Some(s) if s.eq_ignore_ascii_case("below_normal") || s.eq_ignore_ascii_case("belownormal") => "BelowNormal",
            Some(s) if s.eq_ignore_ascii_case("normal") => "Normal",
            Some(s) if s.eq_ignore_ascii_case("above_normal") || s.eq_ignore_ascii_case("abovenormal") => "AboveNormal",
            Some(s) if s.eq_ignore_ascii_case("high") => "High",
            Some(s) if s.eq_ignore_ascii_case("realtime") => "Realtime",
            Some(other) => bail!("unsupported --priority value '{other}'"),
            None => "Normal",
        };
        let window_state = match args.window_state.as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("normal") => "Normal",
            Some(s) if s.eq_ignore_ascii_case("minimized") || s.eq_ignore_ascii_case("min") => "Minimized",
            Some(s) if s.eq_ignore_ascii_case("maximized") || s.eq_ignore_ascii_case("max") => "Maximized",
            Some(s) if s.eq_ignore_ascii_case("hidden") || s.eq_ignore_ascii_case("hide") => "Hidden",
            Some(other) => bail!("unsupported --window-state value '{other}'"),
            None => "Normal",
        };
        json!({
            "profile": {
                "exe_path": exe_path,
                "args": args.args,
                "working_dir": args.work_dir.as_ref().map(|p| p.display().to_string()),
                "run_as_mode": run_as_mode,
                "priority_class": priority_class,
                "affinity_mask": affinity_mask,
                "window_state": window_state,
                "compatibility_layer": args.compat,
                "env_overrides": env_overrides,
            }
        })
    };

    if args.validate {
        execute_cli_action(
            cli,
            bundle,
            "cmd_run_validate",
            payload,
            Some(ActionRiskLevel::ReadOnly),
        )
    } else {
        execute_cli_action(cli, bundle, "cmd_run", payload, None)
    }
}

fn handle_cmd_from_proc(cli: &Cli, bundle: &ToolBundle, args: &FromProcArgs) -> Result<()> {
    if args.parent_pid.is_none() && args.child_exe.is_none() {
        return query_and_emit_provider(cli, bundle, "cmd_from_proc_candidates");
    }
    let parent_pid = args
        .parent_pid
        .ok_or_else(|| anyhow!("`cmd from-proc` mutation requires --parent-pid"))?;
    let child_exe = args
        .child_exe
        .as_ref()
        .ok_or_else(|| anyhow!("`cmd from-proc` mutation requires --child-exe"))?;
    let payload = json!({
        "parent_pid": parent_pid,
        "child_exe": child_exe,
        "args": args.args,
        "working_dir": args.working_dir,
        "inherit_handles": args.inherit_handles,
        "new_console": args.new_console,
    });
    execute_cli_action(cli, bundle, "cmd_from_proc", payload, None)
}

fn handle_cmd_atnow(cli: &Cli, bundle: &ToolBundle, args: &AtnowArgs) -> Result<()> {
    if args.task_name.is_none() && args.command.is_none() && args.when.is_none() {
        return query_and_emit_provider(cli, bundle, "cmd_atnow_tasks");
    }
    let task_name = args
        .task_name
        .as_ref()
        .ok_or_else(|| anyhow!("`cmd atnow` scheduling requires --task-name"))?;
    let command = args
        .command
        .as_ref()
        .ok_or_else(|| anyhow!("`cmd atnow` scheduling requires --command"))?;
    let when = args
        .when
        .as_ref()
        .ok_or_else(|| anyhow!("`cmd atnow` scheduling requires --when"))?;

    let payload = json!({
        "task_name": task_name,
        "command": command,
        "when": when,
        "arguments": args.arguments,
        "working_dir": args.working_dir,
        "run_as_system": args.run_as_system,
        "interactive": !args.non_interactive,
        "highest_privileges": args.highest_privileges,
        "folder_path": args.folder_path.as_deref().unwrap_or("\\"),
    });
    execute_cli_action(cli, bundle, "cmd_atnow", payload, None)
}

fn handle_audio_view(cli: &Cli, bundle: &ToolBundle, args: &AudioViewArgs) -> Result<()> {
    if args.action.is_none() && args.target.is_none() {
        return query_and_emit_provider(cli, bundle, "audio_view");
    }
    let action = args
        .action
        .as_ref()
        .ok_or_else(|| anyhow!("`audio view` control requires --action"))?;
    let target = args
        .target
        .as_ref()
        .ok_or_else(|| anyhow!("`audio view` control requires --target"))?;

    let mut payload = json!({
        "action": action,
        "target": target,
    });
    if let Some(v) = args.volume_percent {
        payload["volume_percent"] = json!(v);
    }
    if !args.channel_volumes.is_empty() {
        payload["channel_volumes"] = json!(args.channel_volumes);
    }
    if let Some(db) = args.volume_db {
        payload["volume_db"] = json!(db);
    }
    if let Some(ref m) = args.mute_mode {
        payload["mute_mode"] = json!(m);
    }
    if !args.roles.is_empty() {
        payload["roles"] = json!(args.roles);
    }
    if let Some(ref app) = args.app_target {
        payload["app_target"] = json!(app);
    }
    execute_cli_action(cli, bundle, "audio_view_control", payload, None)
}

fn handle_audio_svcl(cli: &Cli, bundle: &ToolBundle, args: &AudioSvclArgs) -> Result<()> {
    if args.command_line.is_none() && args.verb.is_none() && args.svcl_args.is_empty() {
        return query_and_emit_provider(cli, bundle, "audio_svcl_status");
    }

    let (parsed, payload) = if let Some(ref cmd_line) = args.command_line {
        let p = parse_svcl_command(cmd_line).map_err(|e| anyhow!(e))?;
        (p, json!({ "command_line": cmd_line }))
    } else if let Some(ref verb) = args.verb {
        let target = args
            .target
            .as_ref()
            .ok_or_else(|| anyhow!("`audio svcl --verb` requires --target"))?;
        let verb_tok = if verb.starts_with('/') || verb.starts_with("--") {
            verb.clone()
        } else {
            format!("/{verb}")
        };
        let mut tokens = vec![verb_tok, target.clone()];
        tokens.extend(args.svcl_args.iter().cloned());
        let p = parse_svcl_tokens(&tokens).map_err(|e| anyhow!(e))?;
        (
            p,
            json!({
                "verb": verb,
                "target": target,
                "args": args.svcl_args,
            }),
        )
    } else {
        let p = parse_svcl_tokens(&args.svcl_args).map_err(|e| anyhow!(e))?;
        let joined = args
            .svcl_args
            .iter()
            .map(|t| {
                if t.contains(char::is_whitespace) && !t.starts_with('"') {
                    format!("\"{t}\"")
                } else {
                    t.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        (p, json!({ "command_line": joined }))
    };

    let risk_override = if parsed.verb.is_read_only() {
        Some(ActionRiskLevel::ReadOnly)
    } else {
        Some(ActionRiskLevel::Mutating)
    };
    execute_cli_action(cli, bundle, "audio_svcl", payload, risk_override)
}

fn handle_audio_app_config(cli: &Cli, bundle: &ToolBundle, args: &AudioAppConfigArgs) -> Result<()> {
    let Some(ref subkey) = args.subkey else {
        return query_and_emit_provider(cli, bundle, "audio_app_config");
    };
    let mut payload = json!({
        "subkey": subkey,
        "delete_subkey": args.delete_subkey,
    });
    if let Some(v) = args.volume_percent {
        payload["volume_percent"] = json!(v);
    }
    if !args.channel_balances.is_empty() {
        payload["channel_balances"] = json!(args.channel_balances);
    }
    if let Some(m) = args.muted {
        payload["muted"] = json!(m);
    }
    execute_cli_action(cli, bundle, "audio_app_config_update", payload, None)
}

fn handle_audio_volumouse(cli: &Cli, bundle: &ToolBundle, args: &AudioVolumouseArgs) -> Result<()> {
    if args.mode.is_none() && args.rules_file.is_none() && args.events_file.is_none() {
        return query_and_emit_provider(cli, bundle, "audio_volumouse_rules");
    }

    let mode = args
        .mode
        .as_deref()
        .unwrap_or("evaluate_events")
        .trim()
        .to_ascii_lowercase();
    let rules: Option<Vec<VolumouseRule>> = match &args.rules_file {
        Some(path) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("failed reading Volumouse rules {}", path.display()))?;
            Some(serde_json::from_str(&text).with_context(|| format!("invalid rules JSON in {}", path.display()))?)
        }
        None => None,
    };
    let events: Vec<PointerWheelEvent> = match &args.events_file {
        Some(path) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("failed reading Volumouse events {}", path.display()))?;
            serde_json::from_str(&text).with_context(|| format!("invalid events JSON in {}", path.display()))?
        }
        None => Vec::new(),
    };

    let mut payload = json!({
        "mode": mode,
        "events": events,
        "apply_to_backend": args.apply_to_backend,
    });
    if let Some(r) = rules {
        payload["rules"] = serde_json::to_value(r)?;
    }
    if let Some(init_v) = args.initial_volume {
        payload["initial_volume_percent"] = json!(init_v);
    }
    if let Some(d) = args.duration_ms {
        payload["duration_ms"] = json!(d);
    }
    if let Some(m) = args.max_events {
        payload["max_events"] = json!(m);
    }

    let risk_override = match mode.as_str() {
        "validate_rules" => Some(ActionRiskLevel::ReadOnly),
        "evaluate_events" if !args.apply_to_backend => Some(ActionRiskLevel::ReadOnly),
        _ => Some(ActionRiskLevel::Mutating),
    };
    execute_cli_action(cli, bundle, "audio_volumouse", payload, risk_override)
}

fn handle_display_monitors(cli: &Cli, bundle: &ToolBundle, args: &DisplayMonitorsArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "display_monitors");
    };
    let mut payload = json!({ "operation": op });
    if let Some(ref t) = args.target {
        payload["target"] = json!(t);
    }
    if let Some(x) = args.x {
        payload["x"] = json!(x);
    }
    if let Some(y) = args.y {
        payload["y"] = json!(y);
    }
    if let Some(w) = args.width {
        payload["width"] = json!(w);
    }
    if let Some(h) = args.height {
        payload["height"] = json!(h);
    }
    if let Some(hz) = args.refresh_hz {
        payload["refresh_hz"] = json!(hz);
    }
    if let Some(bpp) = args.bpp {
        payload["bpp"] = json!(bpp);
    }
    if let Some(o) = args.orientation {
        payload["orientation"] = json!(o);
    }
    if let Some(ref p) = args.path {
        payload["path"] = json!(p.display().to_string());
    }
    if let Some(ref fmt) = args.config_format {
        payload["format"] = json!(fmt);
    }
    execute_cli_action(cli, bundle, "display_monitors_apply", payload, None)
}

fn handle_display_ddc(cli: &Cli, bundle: &ToolBundle, args: &DisplayDdcArgs) -> Result<()> {
    if args.vcp_code.is_none() && args.value.is_none() {
        return query_and_emit_provider(cli, bundle, "display_ddc_vcp");
    }
    let monitor = args
        .monitor
        .as_ref()
        .ok_or_else(|| anyhow!("`display ddc` set requires --monitor"))?;
    let vcp_code = args
        .vcp_code
        .as_ref()
        .ok_or_else(|| anyhow!("`display ddc` set requires --vcp-code"))?;
    let value = args
        .value
        .ok_or_else(|| anyhow!("`display ddc` set requires --value"))?;
    let payload = json!({
        "monitor": monitor,
        "vcp_code": vcp_code,
        "value": value,
    });
    execute_cli_action(cli, bundle, "display_ddc_set", payload, None)
}

fn handle_display_edid(cli: &Cli, bundle: &ToolBundle, args: &DisplayEdidArgs) -> Result<()> {
    let query = cli
        .table_query()?
        .to_table_query(&winctl2rust_display::edid::edid_table_schema())?;
    if let Some(ref path) = args.file {
        let raw = fs::read(path).with_context(|| format!("failed reading EDID input {}", path.display()))?;
        let edid_bytes = parse_edid_input(&raw).map_err(|e| anyhow!(e))?;
        let label = path.display().to_string();
        let data = edid_table_from_bytes(&label, &edid_bytes, &query).map_err(|e| anyhow!(e))?;
        return emit_table_data(cli, &data);
    }
    if let Some(ref hex) = args.hex {
        let data = edid_table_from_hex("cli-hex", hex, &query).map_err(|e| anyhow!(e))?;
        return emit_table_data(cli, &data);
    }
    query_and_emit_provider(cli, bundle, "display_edid")
}

fn handle_desktop_props(cli: &Cli, bundle: &ToolBundle, args: &DesktopPropsArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "desktop_props");
    };
    let hwnd = args
        .hwnd
        .as_ref()
        .ok_or_else(|| anyhow!("`desktop props` control requires --hwnd"))?;
    let mut payload = json!({
        "operation": op,
        "hwnd": hwnd,
    });
    if let Some(opac) = args.opacity {
        payload["opacity"] = json!(opac);
    }
    execute_cli_action(cli, bundle, "desktop_props_control", payload, None)
}

fn handle_desktop_windows(cli: &Cli, bundle: &ToolBundle, args: &DesktopWindowsArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "desktop_windows");
    };
    let hwnd = args
        .hwnd
        .as_ref()
        .ok_or_else(|| anyhow!("`desktop windows` control requires --hwnd"))?;
    let payload = json!({
        "operation": op,
        "hwnd": hwnd,
    });
    execute_cli_action(cli, bundle, "desktop_windows_control", payload, None)
}

fn handle_desktop_tree(cli: &Cli, bundle: &ToolBundle, args: &DesktopTreeArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "desktop_tree");
    };
    let hwnd = args
        .hwnd
        .as_ref()
        .ok_or_else(|| anyhow!("`desktop tree` control requires --hwnd"))?;
    let mut payload = json!({
        "operation": op,
        "hwnd": hwnd,
    });
    if let Some(ref t) = args.text {
        payload["text"] = json!(t);
    }
    if let Some(x) = args.x {
        payload["x"] = json!(x);
    }
    if let Some(y) = args.y {
        payload["y"] = json!(y);
    }
    if let Some(w) = args.width {
        payload["width"] = json!(w);
    }
    if let Some(h) = args.height {
        payload["height"] = json!(h);
    }
    execute_cli_action(cli, bundle, "desktop_tree_control", payload, None)
}

fn handle_desktop_clipboard(cli: &Cli, bundle: &ToolBundle, args: &DesktopClipboardArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "desktop_clipboard");
    };
    let mut payload = json!({ "operation": op });
    if let Some(ref text) = args.text {
        payload["text"] = json!(text);
    }
    execute_cli_action(cli, bundle, "desktop_clipboard_control", payload, None)
}

fn handle_desktop_clip_hist(cli: &Cli, bundle: &ToolBundle, args: &DesktopClipHistArgs) -> Result<()> {
    let Some(ref op) = args.operation else {
        return query_and_emit_provider(cli, bundle, "desktop_clip_hist");
    };
    let mut payload = json!({
        "operation": op,
        "wait_for_update": args.wait_for_update,
    });
    if let Some(id) = args.entry_id {
        payload["entry_id"] = json!(id);
    }
    if let Some(ref p) = args.path {
        payload["path"] = json!(p.display().to_string());
    }
    if let Some(ref db) = args.db_path {
        payload["db_path"] = json!(db.display().to_string());
    }
    if let Some(ms) = args.timeout_ms {
        payload["timeout_ms"] = json!(ms);
    }
    execute_cli_action(cli, bundle, "desktop_clip_hist_control", payload, None)
}

fn handle_desktop_hotkeys(cli: &Cli, bundle: &ToolBundle, args: &DesktopHotkeysArgs) -> Result<()> {
    if let Some(ref lnk_path) = args.lnk {
        let info = parse_lnk_file(lnk_path).map_err(|e| anyhow!(e))?;
        let entry = winctl2rust_desktop::backend::HotkeyEntry {
            source: "LnkShortcut".to_string(),
            hotkey: info.hotkey_label,
            modifiers: info.modifiers_label,
            vk_code: u32::from(info.vk_code),
            status: if info.vk_code == 0 {
                "Unbound".to_string()
            } else {
                "ShortcutBound".to_string()
            },
            owner_hint: info.owner_hint,
            lnk_path: lnk_path.display().to_string(),
        };
        let schema = hotkeys_table_schema();
        let row = hotkey_to_row(1, &entry);
        let query = cli.table_query()?.to_table_query(&schema)?;
        let data = apply_query(&schema, &[row], &query)?;
        return emit_table_data(cli, &data);
    }
    query_and_emit_provider(cli, bundle, "desktop_hotkeys")
}

fn handle_desktop_fonts(cli: &Cli, bundle: &ToolBundle, args: &DesktopFontsArgs) -> Result<()> {
    if let Some(ref font_path) = args.file {
        let bytes = fs::read(font_path).with_context(|| format!("failed reading font file {}", font_path.display()))?;
        let faces = parse_font_bytes(&bytes).map_err(|e| anyhow!(e))?;
        let source_path = font_path.display().to_string();
        let rows: Vec<_> = faces
            .into_iter()
            .enumerate()
            .map(|(idx, face)| {
                let os2 = face.os2.as_ref();
                let entry = winctl2rust_desktop::backend::InstalledFontEntry {
                    family: face.family(),
                    subfamily: face.subfamily(),
                    full_name: face.full_name(),
                    postscript_name: face.postscript_name(),
                    font_format: face.format.label().to_string(),
                    weight_class: os2.map_or(400, |o| o.us_weight_class),
                    width_class: os2.map_or(5, |o| o.us_width_class),
                    fs_type: os2.map_or(0, |o| o.fs_type),
                    panose: os2.map_or_else(String::new, |o| o.panose_formatted()),
                    version: face.version(),
                    source_path: source_path.clone(),
                    face_index: face.face_index,
                };
                font_entry_to_row(idx as u64 + 1, &entry)
            })
            .collect();
        let schema = fonts_table_schema();
        let query = cli.table_query()?.to_table_query(&schema)?;
        let data = apply_query(&schema, &rows, &query)?;
        return emit_table_data(cli, &data);
    }
    query_and_emit_provider(cli, bundle, "desktop_fonts")
}

fn handle_desktop_webcam(cli: &Cli, bundle: &ToolBundle, args: &DesktopWebcamArgs) -> Result<()> {
    let Some(ref out_path) = args.output_path else {
        return query_and_emit_provider(cli, bundle, "desktop_webcam");
    };
    let mut payload = json!({
        "output_path": out_path.display().to_string(),
    });
    if let Some(idx) = args.device_index {
        payload["device_index"] = json!(idx);
    }
    if let Some(ref fmt) = args.image_format {
        payload["format"] = json!(fmt);
    }
    execute_cli_action(cli, bundle, "desktop_webcam_capture", payload, None)
}

fn handle_export(cli: &Cli, bundle: &ToolBundle, args: &ExportArgs) -> Result<()> {
    let out_path = cli
        .output
        .as_ref()
        .ok_or_else(|| anyhow!("`export` requires --output <PATH_OR_DIR>"))?;
    let format = resolve_export_format(cli.format, out_path);
    let opts = WinctlReportOptions {
        write_custody_sidecar: !args.no_custody_sidecar,
        dry_run: cli.dry_run,
        ..WinctlReportOptions::default()
    };

    if let Some(ref tool_id) = args.tool {
        let provider = resolve_provider(bundle, tool_id)?;
        let query = cli.table_query()?.to_table_query(&provider.schema())?;
        let data = provider.query(&query).map_err(|e| anyhow!(e))?;
        let report = write_report(&data, out_path, format, &opts).map_err(|e| anyhow!(e))?;
        if let Some(ref custody_path) = cli.custody
            && report.sidecar_path.as_deref() != Some(custody_path.as_path())
        {
            write_custody_json(&report, custody_path)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "exported": true,
                "tool_id": provider.tool_id(),
                "format": format.to_string(),
                "output": out_path.display().to_string(),
                "rows": data.rows.len(),
                "sidecar": report.sidecar_path.as_ref().map(|p| p.display().to_string()),
                "merkle_root_sha256": report.custody.merkle_root_sha256,
            }))?
        );
        return Ok(());
    }

    if !args.all && args.category.is_none() {
        bail!("`export` requires either --tool <TOOL_ID>, --category <CAT>, or --all");
    }

    let selected_tool_ids: Vec<&'static str> = if let Some(cat) = args.category {
        let target_cat: ToolCategory = cat.into();
        builtin_suite_catalog()
            .iter()
            .filter(|t| t.category == target_cat)
            .flat_map(|t| t.table_ids.iter().copied())
            .collect()
    } else {
        bundle.providers.iter().map(|p| p.tool_id()).collect()
    };

    let mut collected = Vec::with_capacity(selected_tool_ids.len());
    for tid in selected_tool_ids {
        let provider = resolve_provider(bundle, tid)?;
        let query = cli.table_query()?.to_table_query(&provider.schema())?;
        let data = provider.query(&query).map_err(|e| anyhow!(e))?;
        collected.push(data);
    }
    let refs: Vec<&TableData> = collected.iter().collect();
    let report = write_report_bundle(&refs, out_path, &[format], &opts).map_err(|e| anyhow!(e))?;
    if let Some(ref custody_path) = cli.custody
        && report.sidecar_path.as_deref() != Some(custody_path.as_path())
    {
        write_custody_json(&report, custody_path)?;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "exported": true,
            "bundle_dir": out_path.display().to_string(),
            "format": format.to_string(),
            "table_count": collected.len(),
            "artifact_count": report.artifact_paths.len(),
            "sidecar": report.sidecar_path.as_ref().map(|p| p.display().to_string()),
            "merkle_root_sha256": report.custody.merkle_root_sha256,
        }))?
    );
    Ok(())
}

fn handle_mcp(bundle: &ToolBundle, args: &McpArgs) -> Result<()> {
    let policy = if args.allow_mutating {
        PolicyMode::AllowMutating
    } else {
        PolicyMode::ReadOnlyOnly
    };
    let registry = build_registry_from_bundle(policy, bundle);
    if args.tools {
        let tools = registry.list_tools();
        println!("{}", serde_json::to_string_pretty(&tools)?);
        return Ok(());
    }
    registry.serve_stdio()
}

fn handle_gui(args: &GuiArgs) -> Result<()> {
    let exe_name = if args.iced || !cfg!(windows) {
        if cfg!(windows) {
            "winctl2rust-iced.exe"
        } else {
            "winctl2rust-iced"
        }
    } else {
        "winctl2rust-win32.exe"
    };

    let candidate = resolve_sibling_binary(exe_name);
    let mut cmd = std::process::Command::new(&candidate);
    cmd.spawn()
        .with_context(|| format!("failed to launch GUI executable {}", candidate.display()))?;
    Ok(())
}

fn resolve_sibling_binary(name: &str) -> PathBuf {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let sibling = dir.join(name);
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from(name)
}

fn handle_catalog(cli: &Cli, args: &CatalogArgs) -> Result<()> {
    let catalog = builtin_suite_catalog();
    if args.json || cli.format == CliOutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(catalog)?);
        return Ok(());
    }
    println!(
        "{:<22} {:<8} {:<24} {:<18} Summary",
        "Tool ID", "Group", "NirSoft Name", "Risk"
    );
    println!("{}", "-".repeat(100));
    for t in catalog {
        println!(
            "{:<22} {:<8} {:<24} {:<18} {}",
            t.tool_id,
            t.category.as_str(),
            t.nirsoft_name,
            format!("{:?}", t.risk_level),
            t.summary
        );
    }
    Ok(())
}

fn resolve_provider<'a>(bundle: &'a ToolBundle, tool_id: &str) -> Result<&'a std::sync::Arc<dyn TableProvider>> {
    bundle
        .providers
        .iter()
        .find(|p| p.tool_id() == tool_id)
        .ok_or_else(|| anyhow!("unknown TableProvider '{tool_id}'"))
}

fn query_and_emit_provider(cli: &Cli, bundle: &ToolBundle, tool_id: &str) -> Result<()> {
    let provider = resolve_provider(bundle, tool_id)?;
    let query = cli.table_query()?.to_table_query(&provider.schema())?;
    let data = provider.query(&query).map_err(|e| anyhow!(e))?;
    emit_table_data(cli, &data)
}

fn emit_table_data(cli: &Cli, data: &TableData) -> Result<()> {
    let opts = WinctlReportOptions {
        write_custody_sidecar: cli.custody.is_some(),
        dry_run: cli.dry_run,
        ..WinctlReportOptions::default()
    };

    if let Some(export_fmt) = cli.format.to_export_format() {
        let bytes = render_table(data, export_fmt, &opts).map_err(|e| anyhow!(e))?;
        if let Some(ref out_path) = cli.output {
            write_bytes_with_parents(out_path, &bytes)?;
            if let Some(ref custody_path) = cli.custody {
                write_custody_for_bytes(&data.schema.tool_id, out_path, &bytes, custody_path, &opts)?;
            }
        } else {
            io::stdout().write_all(&bytes)?;
            if !bytes.ends_with(b"\n") {
                println!();
            }
            if let Some(ref custody_path) = cli.custody {
                write_custody_for_bytes(&data.schema.tool_id, Path::new("stdout"), &bytes, custody_path, &opts)?;
            }
        }
        return Ok(());
    }

    let rendered = format_ascii_table(data);
    if let Some(ref out_path) = cli.output {
        write_bytes_with_parents(out_path, rendered.as_bytes())?;
        if let Some(ref custody_path) = cli.custody {
            write_custody_for_bytes(&data.schema.tool_id, out_path, rendered.as_bytes(), custody_path, &opts)?;
        }
    } else {
        print!("{rendered}");
        if let Some(ref custody_path) = cli.custody {
            write_custody_for_bytes(
                &data.schema.tool_id,
                Path::new("stdout"),
                rendered.as_bytes(),
                custody_path,
                &opts,
            )?;
        }
    }
    Ok(())
}

pub fn format_ascii_table(data: &TableData) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let cols = &data.schema.columns;
    if cols.is_empty() {
        let _ = writeln!(out, "{} (0 columns)", data.schema.title);
        return out;
    }

    let mut widths: Vec<usize> = cols.iter().map(|c| c.label.chars().count().max(3)).collect();
    let mut formatted_rows: Vec<Vec<String>> = Vec::with_capacity(data.rows.len());
    for row in &data.rows {
        let mut cells = Vec::with_capacity(cols.len());
        for (idx, col) in cols.iter().enumerate() {
            let txt = row
                .cells
                .get(idx)
                .map(|c| c.format_human(col.col_type))
                .unwrap_or_default();
            let clean = txt.replace(['\r', '\n'], " ");
            let char_len = clean.chars().count().min(64);
            if char_len > widths[idx] {
                widths[idx] = char_len;
            }
            cells.push(truncate_display(&clean, 64));
        }
        formatted_rows.push(cells);
    }

    for (idx, col) in cols.iter().enumerate() {
        if idx > 0 {
            out.push_str(" | ");
        }
        let _ = write!(out, "{:<width$}", col.label, width = widths[idx]);
    }
    out.push('\n');

    for (idx, w) in widths.iter().enumerate() {
        if idx > 0 {
            out.push_str("-+-");
        }
        for _ in 0..*w {
            out.push('-');
        }
    }
    out.push('\n');

    for cells in &formatted_rows {
        for (idx, cell) in cells.iter().enumerate() {
            if idx > 0 {
                out.push_str(" | ");
            }
            let _ = write!(out, "{:<width$}", cell, width = widths[idx]);
        }
        out.push('\n');
    }
    let _ = writeln!(out, "({} row(s) shown; {} total)", data.rows.len(), data.total_rows);
    out
}

fn truncate_display(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{prefix}...")
}

fn execute_cli_action(
    cli: &Cli,
    bundle: &ToolBundle,
    command_id: &str,
    payload: Value,
    risk_override: Option<ActionRiskLevel>,
) -> Result<()> {
    let action = bundle
        .find_action(command_id)
        .ok_or_else(|| anyhow!("unknown ActionCommand '{command_id}'"))?;
    let effective_risk = risk_override.unwrap_or_else(|| action.risk_level());
    let exec_opts = cli.execution_options();

    if let Err(err) = exec_opts.validate_action(command_id, effective_risk) {
        if let Some(ref custody_path) = cli.custody {
            let rec = record_action_audit(
                command_id,
                effective_risk,
                cli.dry_run,
                false,
                err.to_string(),
                &payload,
            );
            let _ = append_audit_jsonl(custody_path, &rec);
        }
        return Err(anyhow!(err));
    }

    let exec_res = action.execute(&payload, cli.dry_run).map_err(|e| anyhow!("{e}"));

    match exec_res {
        Ok(value) => {
            if let Some(ref custody_path) = cli.custody {
                let rec = record_action_audit(
                    command_id,
                    effective_risk,
                    cli.dry_run,
                    true,
                    "executed via winctl2rust CLI",
                    &payload,
                );
                append_audit_jsonl(custody_path, &rec).map_err(|e| anyhow!(e))?;
            }
            let rendered = serde_json::to_string_pretty(&value)?;
            if let Some(ref out_path) = cli.output {
                write_bytes_with_parents(out_path, rendered.as_bytes())?;
            } else {
                println!("{rendered}");
            }
            Ok(())
        }
        Err(err) => {
            if let Some(ref custody_path) = cli.custody {
                let rec = record_action_audit(
                    command_id,
                    effective_risk,
                    cli.dry_run,
                    false,
                    err.to_string(),
                    &payload,
                );
                let _ = append_audit_jsonl(custody_path, &rec);
            }
            Err(err)
        }
    }
}

fn parse_kv_pairs(raw: &[String], flag_name: &str) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for item in raw {
        let Some((k, v)) = item.split_once('=') else {
            bail!("invalid {flag_name} '{item}': expected KEY=VAL");
        };
        if k.trim().is_empty() {
            bail!("invalid {flag_name} '{item}': key must not be empty");
        }
        validate_r4_compliance(k, &[v.to_string()]).map_err(|e| anyhow!(e))?;
        map.insert(k.to_string(), v.to_string());
    }
    Ok(map)
}

fn parse_affinity_mask(raw: &str) -> Result<usize> {
    let trimmed = raw.trim();
    let val = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).with_context(|| format!("invalid hex affinity mask '{raw}'"))?
    } else {
        trimmed
            .parse::<usize>()
            .with_context(|| format!("invalid affinity mask '{raw}'"))?
    };
    if val == 0 {
        return Err(anyhow!(WinctlError::InvalidInput(
            "affinity mask must be non-zero".to_string()
        )));
    }
    Ok(val)
}

fn resolve_export_format(cli_fmt: CliOutputFormat, path: &Path) -> ExportFormat {
    if let Some(fmt) = cli_fmt.to_export_format() {
        return fmt;
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && let Ok(parsed) = ExportFormat::parse(ext)
    {
        return parsed;
    }
    ExportFormat::Json
}

fn write_bytes_with_parents(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn write_custody_for_bytes(
    tool_id: &str,
    artifact_path: &Path,
    bytes: &[u8],
    custody_path: &Path,
    opts: &WinctlReportOptions,
) -> Result<()> {
    let leaf = guap_export::CustodyLeaf {
        index: 0,
        path: artifact_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        sha512: sha512_hex(bytes),
    };
    let manifest = build_custody_manifest(tool_id, &opts.export.tool_version, vec![leaf]);
    let report = winctl2rust_report::WinctlCustodyReport {
        suite_name: "winctl2rust".to_string(),
        suite_version: opts.export.tool_version.clone(),
        operator: opts.operator.clone(),
        case_id: opts.case_id.clone(),
        host_name: opts.host_name.clone(),
        dry_run: opts.dry_run,
        custody: manifest,
        artifact_paths: vec![artifact_path.to_path_buf()],
        sidecar_path: Some(custody_path.to_path_buf()),
    };
    write_custody_json(&report, custody_path)
}

fn write_custody_json(report: &winctl2rust_report::WinctlCustodyReport, path: &Path) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    write_bytes_with_parents(path, &json)
}
