# `winctl2rust` — 22-Tool Pure-Rust Non-Hooking Windows Control, CoreAudio, Display & Desktop Suite

Part of the **GUAP** Ecosystem (`guap.ca`) • [**Project Hub & Web Documentation: https://guap.ca/winctl2rust/**](https://guap.ca/winctl2rust/) • [**Project Wiki**](https://github.com/adpaolucci/winctl2rust/wiki)

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Rust: 2024](https://img.shields.io/badge/Rust-2024%20(1.88%2B)-orange.svg)](Cargo.toml)
[![Workspace: 10 Crates](https://img.shields.io/badge/Workspace-10%20Crates-emerald.svg)](Cargo.toml)
[![GUI: WinSafe Win32 + Iced 0.14](https://img.shields.io/badge/GUI-WinSafe%20Win32%20%2B%20Iced%200.14-cyan.svg)](crates/winctl2rust-win32)
[![MCP: stdio JSON-RPC](https://img.shields.io/badge/MCP-stdio%20JSON--RPC%202.0-purple.svg)](crates/winctl2rust-mcp)

Ready-to-submit **WinGet** & **Homebrew** manifests live in [`packaging/`](packaging/README.md).

**`winctl2rust`** is a 100% pure-Rust suite of 22 non-hooking Windows control, CoreAudio, DDC/CI & VESA EDID display, and desktop inspection utilities on top of [`guap-kit`](https://github.com/adpaolucci/guap-kit):
1. **`winctl2rust-win32`** (**WinSafe `0.0.29`**) — Native zero-GPU Win32 virtual list console (`LVS_OWNERDATA`, ComCtl v6, Per-Monitor DPI v2).
2. **`winctl2rust-iced`** (**Iced `0.14`**) — Cross-platform GUI (`wgpu` + `tiny-skia` CPU fallback) supporting offline EDID/DSL/OpenType/`.lnk` analysis across OSes.
3. **`winctl2rust`** — Unified `clap 4.6` CLI (`cmd`, `audio`, `display`, `desktop`, `export`, `mcp`, `gui`, `catalog`) with `--dry-run` previews and `--allow-mutating --confirm-destructive` mutation gates.
4. **`winctl2rust-mcp`** — Policy-gated JSON-RPC 2.0 (`2024-11-05`) `stdio` MCP server exposing **40 tools** (**24** read-only + **16** mutating/privileged-mutating under `ReadOnlyOnly` default `-32003` enforcement).

---

## Comparison: `winctl2rust` vs. Legacy Windows Utilities

| Capability | `winctl2rust` | Legacy Ad-Hoc Utilities / Scripts |
|---|---|---|
| **Policy & Dry-Run Safety** | **Mandatory `--dry-run` preview & `PolicyMode::ReadOnlyOnly` (`-32003` SAFETY REFUSAL)** | Immediate unguarded OS mutation |
| **Offline Determinism** | **Trait-isolated OS backends + pure-Rust Script DSL, INI/JSON launch profiles, VESA EDID/CEA-861, OpenType, & SHLLINK parsers** | Tightly coupled to live Win32 handles |
| **Audit & Chain of Custody** | **7 `guap-export` formats + SHA-256/SHA-512 Merkle `CustodyManifest` + JSONL action audit** | Unverified plaintext output |
| **Agent Integration** | **Native MCP JSON-RPC `2024-11-05` `stdio` server (40 endpoints)** | Fragile console scraping |
| **Zero-Injection R4 Boundary** | **Zero `RunAsDate`, zero `CreateRemoteThread`, zero external-DLL hooks, zero credential recovery** | Legacy hooking / injection risk |

---

## Workspace Architecture (10 Crates, 4 Binaries)

```text
winctl2rust/
├── Cargo.toml                  # 10-crate workspace (Edition 2024, MSRV 1.88, Apache-2.0)
├── deny.toml                   # cargo-deny license & advisory policy
├── mcp.json / smithery.yaml    # MCP & Smithery manifests (40 endpoints)
├── metadata.json               # Machine-readable 22-tool suite manifest
├── packaging/                  # WinGet & Homebrew manifests
└── crates/
    ├── winctl2rust-core/       # `#![forbid(unsafe_code)]` ExecutionOptions, R4 guards, privilege probe, 22-tool catalog
    ├── winctl2rust-report/     # `#![forbid(unsafe_code)]` guap-export integration + JSONL action audit log
    ├── winctl2rust-cmd/        # cmd: script DSL, launch profiles, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, Task Scheduler v2
    ├── winctl2rust-audio/      # audio: CoreAudio inspector, CLI controller, PropertyStore routing, WM_INPUT wheel rules
    ├── winctl2rust-display/    # display: CCD topology, DDC/CI VCP, display target summary, VESA/CEA-861 EDID parser
    ├── winctl2rust-desktop/    # desktop: window props, top-level windows, HWND trees, control export, clipboard, hotkeys, keys, fonts, webcam
    ├── winctl2rust-mcp/        # `#![forbid(unsafe_code)]` Policy-gated MCP JSON-RPC 2.0 stdio server (`winctl2rust-mcp`)
    ├── winctl2rust-cli/        # `#![forbid(unsafe_code)]` Unified CLI (`winctl2rust`)
    ├── winctl2rust-win32/      # `#![forbid(unsafe_code)]` Native Win32 GUI (`winctl2rust-win32`)
    └── winctl2rust-iced/       # `#![forbid(unsafe_code)]` Cross-platform Iced 0.14 GUI (`winctl2rust-iced`)
```

---

## 22-Tool Matrix & 40 MCP Endpoints

### `winctl2rust-cmd` (4 Tools)
| CLI Subcommand | Display Name | Read-Only Provider(s) | Policy-Gated Mutating Action(s) |
|---|---|---|---|
| `cmd script` | `Command Script Runner` | `cmd_script_preview`, `cmd_script_parse` | `cmd_script` |
| `cmd run` | `Process Launch Profiles` | `cmd_run_profiles`, `cmd_run_validate` | `cmd_run` |
| `cmd from-proc` | `Parent-Attributed Spawner` | `cmd_from_proc_candidates` | `cmd_from_proc` |
| `cmd atnow` | `One-Shot Task Scheduler` | `cmd_atnow_tasks` | `cmd_atnow` |

### `winctl2rust-audio` (4 Tools)
| CLI Subcommand | Display Name | Read-Only Provider | Policy-Gated Mutating Action |
|---|---|---|---|
| `audio view` | `CoreAudio Inspector` | `audio_view` | `audio_view_control` |
| `audio ctl` | `CoreAudio CLI Controller` | `audio_ctl_status` | `audio_ctl` |
| `audio app-config` | `Per-App Audio Routing` | `audio_app_config` | `audio_app_config_update` |
| `audio wheel` | `Wheel Volume Rules` | `audio_wheel_rules` | `audio_wheel` |

### `winctl2rust-display` (4 Tools)
| CLI Subcommand | Display Name | Read-Only Provider | Policy-Gated Mutating Action |
|---|---|---|---|
| `display monitors` | `Multi-Monitor Topology` | `display_monitors` | `display_monitors_apply` |
| `display ddc` | `DDC/CI VCP Controller` | `display_ddc_vcp` | `display_ddc_set` |
| `display info` | `Display & EDID Summary` | `display_info` | *(read-only)* |
| `display edid` | `VESA EDID Decoder` | `display_edid` | *(read-only)* |

### `winctl2rust-desktop` (10 Tools)
| CLI Subcommand | Display Name | Read-Only Provider | Policy-Gated Mutating Action |
|---|---|---|---|
| `desktop props` | `Window Property Inspector` | `desktop_props` | `desktop_props_control` |
| `desktop windows` | `Top-Level Window Lister` | `desktop_windows` | `desktop_windows_control` |
| `desktop tree` | `Control Hierarchy Explorer` | `desktop_tree` | `desktop_tree_control` |
| `desktop ctrl-export` | `Control Data Exporter` | `desktop_ctrl_export` | *(read-only cross-process messaging)* |
| `desktop clipboard` | `Clipboard Format Inspector` | `desktop_clipboard` | `desktop_clipboard_control` |
| `desktop clip-hist` | `Clipboard History Archive` | `desktop_clip_hist` | `desktop_clip_hist_control` |
| `desktop hotkeys` | `Global Hotkey Inspector` | `desktop_hotkeys` | *(read-only `.lnk` + `RegisterHotKey` probe)* |
| `desktop keys` | `Keyboard State Inspector` | `desktop_keys` | *(read-only)* |
| `desktop fonts` | `Installed Font Inspector` | `desktop_fonts` | *(read-only)* |
| `desktop webcam` | `Webcam Frame Capture` | `desktop_webcam` | `desktop_webcam_capture` |

---

## R4 Security & Non-Hooking Boundaries

- **Zero API Hooking**: Excludes `RunAsDate`, Detours, EAT/IAT patching, and external-DLL `SetWindowsHookExW` injection. (`audio wheel` supports safe non-injecting `WM_INPUT` raw-input and same-process `WH_MOUSE_LL` listener modes without injecting external DLLs.)
- **Zero Remote Thread Injection**: Excludes `CreateRemoteThread`, `NtCreateThreadEx`, `QueueUserAPC`, and `WriteProcessMemory` code injection. (`desktop ctrl-export` uses strictly passive data-struct `SendMessageTimeoutW` / `VirtualAllocEx` buffer reads for `LVITEMW`/`TVITEMW`/`HDITEMW` controls, with zero executable code.)
- **Zero Credential Dumping & Retired Tools**: Strictly excludes `cred2rust` tools and all 39 retired legacy utilities.

---

## CLI, GUI & MCP Quick Start

```powershell
# Inspect read-only catalog, live monitors, offline EDID, OpenType fonts & .lnk hotkeys
winctl2rust catalog
winctl2rust display monitors --format json
winctl2rust display edid --file monitor.bin --format csv --output edid.csv --custody edid.custody.json
winctl2rust desktop fonts --file font.ttf --format json
winctl2rust desktop hotkeys --lnk shortcut.lnk --format json

# Export all 22 tables with Merkle custody manifest
winctl2rust export --all --format json --output .\audit_bundle

# Policy-gated mutations: preview with --dry-run, or execute with explicit confirmation
winctl2rust cmd script --script "beep 440 100" --dry-run
winctl2rust cmd script --script "beep 440 100" --allow-mutating --confirm-destructive

# Launch GUIs or MCP server
winctl2rust gui              # Native Win32 virtual list GUI (winctl2rust-win32)
winctl2rust gui --iced       # Cross-platform Iced 0.14 GUI (winctl2rust-iced)
winctl2rust mcp --tools      # Dump JSON Schemas for all 40 MCP endpoints
winctl2rust-mcp              # Start ReadOnlyOnly JSON-RPC 2.0 stdio server (see mcp.json)
```

---

## Building & Testing

```powershell
cargo check --workspace --offline --locked
cargo test --workspace --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
cargo build --workspace --release --offline --locked
```

---

## Wiki Documentation

Browse the full [`winctl2rust` Project Wiki](https://github.com/adpaolucci/winctl2rust/wiki):
- [Architecture](https://github.com/adpaolucci/winctl2rust/wiki/Architecture)
- [Non-Hooking Automation & 22-Tool Matrix](https://github.com/adpaolucci/winctl2rust/wiki/Non-Hooking-Automation-and-22-Tool-Matrix)
- [CLI Reference](https://github.com/adpaolucci/winctl2rust/wiki/CLI-Reference)
- [Dual-GUI Guide (Win32 & Iced)](https://github.com/adpaolucci/winctl2rust/wiki/Dual-GUI-Guide)
- [GUI Framework Decision & Comparison](https://github.com/adpaolucci/winctl2rust/wiki/GUI-Framework-Decision-and-Comparison)
- [MCP Server Guide](https://github.com/adpaolucci/winctl2rust/wiki/MCP-Server-Guide)
- [Reports & Forensic Exports](https://github.com/adpaolucci/winctl2rust/wiki/Reports-and-Forensic-Exports)
- [Validation Rules & Status Codes](https://github.com/adpaolucci/winctl2rust/wiki/Validation-Rules-and-Status-Codes)
- [Building, Testing & CI/CD](https://github.com/adpaolucci/winctl2rust/wiki/Building-Testing-and-CI-CD)
- [Security & Stability](https://github.com/adpaolucci/winctl2rust/wiki/Security-and-Stability)

## License

Licensed under the [**Apache License, Version 2.0**](LICENSE). See [`NOTICE`](NOTICE) for attribution.
