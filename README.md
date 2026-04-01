# toss

Fast CLI to deploy self-built iOS apps to connected devices via `xcrun devicectl`.

## Requirements

- macOS with Xcode 15.1+ (for `xcrun devicectl`)
- Rust toolchain

## Installation

```bash
cargo install --path .
```

## Quick Start

```bash
# Register your Xcode project (points at source directory)
toss projects add ~/Projects/MyApp

# Or register an IPA into project management
toss projects add ~/Downloads/WeChat.ipa --ipa --alias wechat

# Alias your device
toss devices alias 1 phone

# Deploy and launch (uses defaults)
toss run
```

## Usage

### Projects

```bash
# Add project (source dir, build dir, or .app path)
toss projects add ~/Projects/MyApp
toss projects add ~/Projects/MyApp --alias myapp

# Add a managed IPA project
toss projects add ~/Downloads/WeChat.ipa --ipa
toss projects add ~/Downloads/WeChat.ipa --ipa --alias wechat

# List registered projects
toss projects list

# Remove project
toss projects remove myapp
```

### Devices

```bash
# List connected devices
toss devices

# Alias a device
toss devices alias 1 phone
toss devices alias <UDID> phone
```

### Deploy

```bash
# Install + launch (uses defaults if omitted)
toss run [project] [-d device]
toss run [project] [-d device] [--dry-run]

# Install only
toss install [project] [-d device]
toss install [project] [-d device] [--dry-run]

# Launch only (IPA projects should use `run`)
toss launch [project] [-d device]

# Direct one-off IPA signing still exists
toss sign /path/to/app.ipa [-d device] [--launch]
toss sign /path/to/app.ipa [-d device] [--launch] [--dry-run]
```

### Config

```bash
# Show current config
toss config show

# Print config file path
toss config path

# Set defaults
toss config set-default-device phone
toss config set-default-project myapp

# Set prefix for temporary signing bundle IDs
toss config set-temp-bundle-prefix cn.yangym.tmp

# Set the Apple developer team ID used for temporary signing
toss config set-team-id FRR2796948

# Show all local toss state and signing cache
toss state

# Remove temporary signing cache created by toss
toss cleanup

# Run environment diagnostics
toss doctor
```

### Signing

```bash
# List codesigning identities from the keychain
toss signing identities

# List local provisioning profiles parsed from Xcode caches
toss signing profiles

# Show team IDs seen in config, identities, and profiles
toss signing teams

# Diagnose signing readiness for an IPA project
toss signing doctor [project] [-d device]
```

### Interactive Mode

Run `toss` without arguments for an interactive menu.

## How It Works

Projects can be backed by either:
1. An Xcode/source/app path
2. A managed `.ipa` copied into toss cache

When you register a project with a source directory containing `.xcodeproj`, toss automatically:
1. Scans `~/Library/Developer/Xcode/DerivedData/` for matching builds
2. Finds the `.app` bundle in `Build/Products/Debug-iphoneos/`
3. Extracts the bundle ID from `Info.plist`

When you register a managed IPA with `--ipa`, toss copies it into `~/Library/Caches/toss/ipas/`
and `toss install/run <alias>` will always re-sign from that cached IPA.

The first device alias and first project registration are automatically set as defaults, so `toss run` works immediately after setup.

## Config File

Located at `~/.config/toss/config.toml`:

```toml
[defaults]
device = "phone"
project = "myapp"

[signing]
temp_bundle_prefix = "com.myapp.tmp"
team_id = "FRR2796948"

[devices.aliases]
phone = "00008110-001234567890001E"

[projects.myapp]
kind = "xcode"
path = "/Users/you/Projects/MyApp"
build_dir = "/Users/you/Library/Developer/Xcode/DerivedData/MyApp-abc123/Build/Products/Debug-iphoneos"
bundle_id = "com.example.myapp"
app_name = "MyApp.app"
last_tossed_at = "2026-03-25T12:34:56Z"

[projects.wechat]
kind = "ipa"
build_dir = ""
ipa_path = "/Users/you/Library/Caches/toss/ipas/wechat-a1b2c3d4.ipa"
original_name = "WeChat.ipa"
bundle_id = "com.tencent.xin"
last_tossed_at = "2026-03-25T12:34:56Z"
```
