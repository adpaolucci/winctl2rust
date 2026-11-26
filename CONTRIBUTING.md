# Contributing to `winctl2rust`

Thank you for your interest in contributing to `winctl2rust`!

## Code Architecture

The workspace targets a 10-crate modular architecture (staged across M3.1–M3.4):
- `crates/winctl2rust-core` (M3.1): Core types, `WinctlError`, `ExecutionOptions`, R4 compliance validators, privilege probe traits, 22-tool catalog, and stderr logging (`#![forbid(unsafe_code)]`).
- `crates/winctl2rust-report` (M3.1): Multi-format tabular exports (CSV, TSV, JSON, JSONL, HTML, Markdown), SHA-256/SHA-512 Merkle chain-of-custody manifests via `guap-export`, and JSONL action audit logging (`#![forbid(unsafe_code)]`).
- `crates/winctl2rust-cmd` (M3.1): Non-hooking command execution (`NirCmd`, `AdvancedRun`, `RunFromProcess`, `AtNow`) with `unsafe` Win32/COM calls isolated behind `CmdBackend` in `#[cfg(windows)] mod win32_backend;`.
- `crates/winctl2rust-audio` (M3.2): CoreAudio & input rule tools (`SoundVolumeView`, `svcl`, `AppAudioConfig`, `Volumouse`).
- `crates/winctl2rust-display` (M3.2): Multi-monitor, DDC/CI, and pure-Rust VESA EDID 1.3/1.4 + CEA-861 tools (`MultiMonitorTool`, `ControlMyMonitor`, `MonitorInfoView`, `DumpEDID`).
- `crates/winctl2rust-desktop` (M3.3): Window, clipboard, hotkey, keyboard, OpenType font, and Media Foundation webcam tools.
- `crates/winctl2rust-mcp` (M3.4): Policy-gated JSON-RPC 2.0 `stdio` server built on `guap-mcp`.
- `crates/winctl2rust-cli` (M3.4): Unified `clap` CLI binary (`winctl2rust`).
- `crates/winctl2rust-win32` (M3.4): Native Win32 GUI binary (`winctl2rust-win32`) built on `guap-gui-win32` / `winsafe`.
- `crates/winctl2rust-iced` (M3.4): Cross-platform Iced 0.14 GUI binary (`winctl2rust-iced`) built on `guap-gui-iced`.

## Development Workflow

1. Fork and clone the repository alongside `guap-kit`.
2. Ensure you have Rust 1.88+ (`edition = "2024"`, `resolver = "2"`).
3. Create a feature branch (`git checkout -b feature/my-enhancement`).
4. Run formatting, checks, linter, and tests:
   ```bash
   cargo fmt --all -- --check
   cargo check --workspace --all-targets
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
5. Open a Pull Request with clear commit messages and test coverage.

## Guidelines

- **R4 Prohibitions (Zero Tolerance)**: Never introduce `RunAsDate`, DLL injection, API hooking (`CreateRemoteThread`, `NtCreateThreadEx`, `QueueUserAPC`, `SetWindowsHookExW` DLL injection, IAT patching), or credential extraction/dumping (`cred2rust`).
- **Safety & Policy Gates**: Every mutating `ActionCommand` must honor `dry_run` deterministically and gate execution through `guap-mcp` `PolicyMode`.
- **`unsafe` Isolation**: Keep `winctl2rust-core`, `winctl2rust-report`, `winctl2rust-mcp`, `winctl2rust-cli`, and `winctl2rust-iced` strictly `#![forbid(unsafe_code)]`. Domain crates must isolate `#[allow(unsafe_code)]` solely to `#[cfg(windows)]` OS backend modules behind traits.
- **Logging**: Direct `tracing` output to `std::io::stderr()` so CLI `stdout` and MCP `stdio` JSON-RPC frames remain uncorrupted.
