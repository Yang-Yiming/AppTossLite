# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**toss** — A Rust CLI that deploys self-built iOS `.app` bundles to connected devices via `xcrun devicectl`. macOS-only (requires Xcode 15.1+).

## Build & Run

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo run -- <subcommand>      # Run in development
cargo test                     # Run tests (none yet)
cargo clippy                   # Lint
cargo fmt -- --check           # Check formatting
```

## Architecture

Two-module split: `cli/` (clap command parsing + handlers) and `core/` (business logic).

**Core modules:**
- `config.rs` — TOML config at `~/.config/toss/config.toml` storing device aliases and project registrations
- `device.rs` — Device discovery and resolution (alias → UDID, index → UDID, or direct UDID)
- `project.rs` — `.app` bundle detection in build dirs, bundle ID extraction via `plutil`
- `xcrun.rs` — Wrapper around `xcrun devicectl` (list devices, install app, launch app)
- `error.rs` — `TossError` enum with `thiserror` derives

**CLI modules:**
- `mod.rs` — Clap derive subcommand definitions
- `actions.rs` — `install`, `launch`, `run` (install+launch) commands
- `devices.rs` — Device listing and aliasing
- `projects.rs` — Project registration CRUD

**Flow:** `main.rs` → `cli::dispatch()` → reads config → calls core functions → invokes `xcrun`/`plutil` externally.

## Key Patterns

- **Resolver pattern**: `resolve_device_id()` and `resolve_project()` convert user-facing identifiers (aliases, indices) into concrete values (UDIDs, paths)
- **Interactive fallback**: When no device is specified and multiple are connected, `dialoguer` prompts the user to select one
- **Config persistence**: `BTreeMap` used for consistent TOML key ordering
- **Structured JSON parsing**: `xcrun devicectl --json-output` writes to a tempfile, then parsed with serde

## External Dependencies (System)

Requires macOS with:
- `xcrun devicectl` (Xcode 15.1+) — device management
- `plutil` (built-in) — Info.plist parsing
