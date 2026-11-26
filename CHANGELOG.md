# Changelog

All notable changes to `winctl2rust` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-23

### Added
- **Workspace Scaffold**: Root `Cargo.toml` (Edition 2024, `resolver = "2"`, `rust-version = "1.88"`, Apache-2.0, `guap-kit` integration, `sha2 = "0.11"`, `windows = "0.62"`), `.cargo/config.toml` (16 MiB stack for GNU & MSVC), `rustfmt.toml`, `deny.toml`, `mcp.json`, `smithery.yaml`, `metadata.json`, `packaging/` (WinGet + Homebrew), and Apache-2.0 governance docs.
- **`winctl2rust-core` (`#![forbid(unsafe_code)]`)**: `WinctlError` / `WinctlResult`, `ExecutionOptions` (`dry_run`, `allow_mutating`, `confirm_destructive`), `validate_r4_compliance`, `dry_run_preview`, `PrivilegeProbe`, 22-tool `builtin_suite_catalog()`, `ToolBundle`, and `stderr` tracing initialization.
- **`winctl2rust-report` (`#![forbid(unsafe_code)]`)**: `guap-export` wrapper supporting all 7 export formats (CSV with formula neutralization, TSV, JSON, JSONL, HTML, vertical HTML, Markdown), SHA-256/SHA-512 Merkle `CustodyManifest` sidecars, multi-table `write_report_bundle`, and `ActionAuditRecord` JSONL audit trails.
- **`winctl2rust-cmd` (M3.1)**: 4 non-hooking command execution & scheduling tools behind `CmdBackend` with pure-Rust offline parsers/validators, `MockCmdBackend`, deterministic `dry_run` previews, `guap-mcp` policy gates, and `#[cfg(windows)]` Win32/COM implementations (`cmd nircmd`, `cmd run`, `cmd from-proc`, `cmd atnow`).
- **`winctl2rust-audio` (M3.2)**: 4 CoreAudio & wheel-rule tools behind `AudioBackend` (`audio view` / `SoundVolumeView`, `audio svcl` / `SoundVolumeCommandLine`, `audio app-config` / `AppAudioConfig` `PROPVARIANT` + `1SPS` `SERIALIZEDPROPSTORAGE` binary codecs, and `audio volumouse` / `Volumouse` in-process `WH_MOUSE_LL`/`WM_INPUT` engine).
- **`winctl2rust-display` (M3.2)**: 4 display, DDC/CI, and EDID tools behind `DisplayBackend` (`display monitors` / `MultiMonitorTool` CCD + INI/JSON profiles, `display ddc` / `ControlMyMonitor` `dxva2.dll` + MCCS 2.2a dictionary, `display info` / `MonitorInfoView`, and `display edid` / `DumpEDID` pure-Rust VESA EDID 1.3/1.4 + CEA-861 parser).
- **`winctl2rust-desktop` (M3.3)**: 10 desktop, window, clipboard, input, font, and webcam tools behind `DesktopBackend` (`desktop props` / `GUIPropView`, `desktop windows` / `WinLister`, `desktop tree` / `WinExplorer`, `desktop sysexport` / `SysExporter` non-injecting `LVM_GETITEMW`/`TVM_GETITEMW`, `desktop clipboard` / `InsideClipboard`, `desktop clip-hist` / `Clipboardic` SQLite + JSON archive, `desktop hotkeys` / `HotKeysList` + `[MS-SHLLINK]` `.lnk` parser, `desktop keys` / `KeyboardStateView`, `desktop fonts` / `WinFontsView` + pure-Rust OpenType `name`/`OS/2` parser, and `desktop webcam` / `WebCamImageSave` Media Foundation + 24-bit BMP/P6 PPM encoder).
- **`winctl2rust-mcp` (M3.4, `#![forbid(unsafe_code)]`)**: Policy-gated JSON-RPC `2024-11-05` `stdio` server (`winctl2rust-mcp`) exposing all 22 `TableProvider`s and 18 `ActionCommand`s (40 MCP endpoints) with `PolicyMode::ReadOnlyOnly` default (`-32003`) and `confirm_destructive: true` enforcement.
- **`winctl2rust-cli` (M3.4, `#![forbid(unsafe_code)]`)**: Unified `clap 4.6` CLI (`winctl2rust`) supporting `cmd`, `audio`, `display`, `desktop`, `export`, `mcp`, `gui`, and `catalog` subcommands, `GetNirQuery` filtering/sorting/pagination, offline EDID/`.lnk`/font inspection, and Merkle custody sidecars.
- **`winctl2rust-win32` & `winctl2rust-iced` (M3.4, `#![forbid(unsafe_code)]`)**: Dual desktop GUIs powered by `guap-gui-win32` (`winsafe` virtual `SysListView32`, Common-Controls v6 + `PerMonitorV2` manifest via GNU `windres` / MSVC `embed-resource`) and `guap-gui-iced` (`iced 0.14` data-grid + inspector pane).
