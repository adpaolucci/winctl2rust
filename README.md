# `winctl2rust`

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Rust: 2024 (1.88+)](https://img.shields.io/badge/Rust-2024%20(1.88%2B)-orange.svg)](Cargo.toml)
[![Workspace: 10 Crates](https://img.shields.io/badge/Workspace-10%20Crates%20(M3.1--M3.4)-informational.svg)](Cargo.toml)
[![GUI: WinSafe Win32 + Iced 0.14](https://img.shields.io/badge/GUI-WinSafe%20Win32%20%2B%20Iced%200.14-purple.svg)](metadata.json)
[![MCP: stdio JSON-RPC](https://img.shields.io/badge/MCP-stdio%20JSON--RPC%202.0-green.svg)](mcp.json)

**`winctl2rust`** is a 100% pure-Rust, open-source GUAP suite replacing 22 non-hooking Windows control, CoreAudio, DDC/CI & EDID display, and desktop automation utilities, backed by `guap-kit`, dual native Win32 (`winsafe`) and cross-platform (`iced 0.14`) GUIs, Merkle chain-of-custody exports, and a policy-gated JSON-RPC 2.0 `stdio` MCP server.

- **Website**: <https://winctl2rust.guap.ca>
- **Repository**: <https://github.com/adpaolucci/winctl2rust>

## Why `winctl2rust`?

| Capability | Legacy Ad-Hoc Utilities / Scripts | `winctl2rust` |
|---|---|---|
| **Safety & Policy Gates** | Immediate unguarded OS mutation | `dry_run` preview + `PolicyMode::ReadOnlyOnly` default (`-32003` block) |
| **Offline Testability** | Coupled directly to live Win32 handles | Trait-isolated OS backends + pure-Rust DSL/EDID/INI/OpenType parsers |
| **Audit & Forensics** | Unverified text dumps | 7 `guap-export` formats + SHA-256/SHA-512 Merkle `CustodyManifest` |
| **Agent Automation** | Fragile stdout scraping | Native MCP JSON-RPC `2024-11-05` `stdio` tools (`TableProvider` + `ActionCommand`) |
| **R4 Security Boundary** | Risk of hooking/injection tools | **Zero** `RunAsDate`, **zero** DLL injection, **zero** API hooking, **zero** credential dumping |

## Workspace Architecture (10 Crates, 4 Binaries)

| Crate | Purpose | Status |
|---|---|---|
| `winctl2rust-core` | Shared errors, `ExecutionOptions`, R4 compliance guards, privilege probe, 22-tool catalog (`#![forbid(unsafe_code)]`) | **M3.1 Implemented** |
| `winctl2rust-report` | Multi-format exports (CSV/TSV/JSON/JSONL/HTML/MD), Merkle custody manifests, JSONL action audit (`#![forbid(unsafe_code)]`) | **M3.1 Implemented** |
| `winctl2rust-cmd` | `NirCmd`, `AdvancedRun`, `RunFromProcess`, `AtNow` parsers, providers, actions & Win32/COM backend | **M3.1 Implemented** |
| `winctl2rust-audio` | `SoundVolumeView`, `SoundVolumeCommandLine` (`svcl`), `AppAudioConfig`, `Volumouse` CoreAudio & `WM_INPUT`/`WH_MOUSE_LL` | **M3.2 Implemented** |
| `winctl2rust-display` | `MultiMonitorTool`, `ControlMyMonitor` DDC/CI, `MonitorInfoView`, `DumpEDID` VESA EDID 1.3/1.4 + CEA-861 parser | **M3.2 Implemented** |
| `winctl2rust-desktop` | `GUIPropView`, `WinLister`, `WinExplorer`, `SysExporter`, `InsideClipboard`, `Clipboardic`, `HotKeysList`, `KeyboardStateView`, `WinFontsView`, `WebCamImageSave` | **M3.3 Implemented** |
| `winctl2rust-mcp` | Policy-gated JSON-RPC 2.0 `stdio` MCP server & `winctl2rust-mcp` binary (40 endpoints, `#![forbid(unsafe_code)]`) | **M3.4 Implemented** |
| `winctl2rust-cli` | Unified `clap 4.6` CLI (`winctl2rust` binary, `#![forbid(unsafe_code)]`) | **M3.4 Implemented** |
| `winctl2rust-win32` | Native Win32 virtual `SysListView32` GUI (`winctl2rust-win32`, `guap-gui-win32` / `winsafe`, `#![forbid(unsafe_code)]`) | **M3.4 Implemented** |
| `winctl2rust-iced` | Cross-platform Iced 0.14 GUI (`winctl2rust-iced`, `guap-gui-iced`, `#![forbid(unsafe_code)]`) | **M3.4 Implemented** |

## 22-Tool Matrix & R4 Exclusions

- **`-cmd` (4 tools, M3.1)**: `NirCmd`, `AdvancedRun`, `RunFromProcess`, `AtNow`
- **`-audio` (4 tools, M3.2)**: `SoundVolumeView`, `SoundVolumeCommandLine` (`svcl`), `AppAudioConfig`, `Volumouse`
- **`-display` (4 tools, M3.2)**: `MultiMonitorTool`, `ControlMyMonitor`, `MonitorInfoView`, `DumpEDID`
- **`-desktop` (10 tools, M3.3)**: `GUIPropView`, `WinLister`, `WinExplorer`, `SysExporter`, `InsideClipboard`, `Clipboardic`, `HotKeysList`, `KeyboardStateView`, `WinFontsView`, `WebCamImageSave`

> **R4 Exclusions (Zero Tolerance):** `winctl2rust` strictly excludes `RunAsDate` (API hooking), `cred2rust` (credential extraction/dumping), DLL injection (`CreateRemoteThread`, `NtCreateThreadEx`, `QueueUserAPC`, `SetWindowsHookExW` injection, IAT patching), and all 39 retired legacy NirSoft utilities.

## CLI & GUI Quick Start

```bash
# Build, lint, and test offline
cargo check --workspace --offline --locked
cargo test --workspace --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings

# Inspect read-only tables & filter/sort/export
winctl2rust catalog
winctl2rust display monitors --format json
winctl2rust display edid --file monitor.bin --format csv --output edid.csv --custody edid.custody.json
winctl2rust desktop fonts --file font.ttf --format json
winctl2rust desktop hotkeys --lnk shortcut.lnk --format json
winctl2rust export --all --format json --output ./audit_bundle

# Dry-run preview vs. explicit mutating gate
winctl2rust cmd nircmd --script "beep 440 100" --dry-run
winctl2rust cmd nircmd --script "beep 440 100" --allow-mutating --confirm-destructive

# Launch GUIs or MCP server
winctl2rust gui              # Launches winctl2rust-win32 on Windows
winctl2rust gui --iced       # Launches winctl2rust-iced
winctl2rust mcp --tools      # Prints all 40 MCP tool JSON Schemas
winctl2rust-mcp              # Runs JSON-RPC 2.0 stdio server (ReadOnlyOnly default)
```

## MCP Policy-Gate Model (40 Endpoints)

`winctl2rust-mcp` registers all 22 read-only `TableProvider`s and 18 risk-classified `ActionCommand`s (2 `ReadOnly`, 15 `Mutating`, 1 `PrivilegedMutating`) on `guap_mcp::McpServerRegistry`:
- `PolicyMode::ReadOnlyOnly` (default): permits all 22 table queries and 2 read-only parse/validate actions (`cmd_nircmd_parse`, `cmd_run_validate`); rejects all 16 mutating/privileged-mutating invocations with JSON-RPC `-32003` (`POLICY_DENIED`).
- `PolicyMode::AllowMutating` (`--allow-mutating`): still requires explicit `confirm_destructive: true` per mutating call, and honors `dry_run: true` by returning a deterministic execution plan without altering OS state.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
