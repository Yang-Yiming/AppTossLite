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

# Install only
toss install [project] [-d device]

# Launch only
toss launch [project] [-d device]
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

### Interactive Mode

Run `toss` without arguments for an interactive menu.

## How It Works

When you register a project with a source directory containing `.xcodeproj`, toss automatically:
1. Scans `~/Library/Developer/Xcode/DerivedData/` for matching builds
2. Finds the `.app` bundle in `Build/Products/Debug-iphoneos/`
3. Extracts the bundle ID from `Info.plist`

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
path = "/Users/you/Projects/MyApp"
build_dir = "/Users/you/Library/Developer/Xcode/DerivedData/MyApp-abc123/Build/Products/Debug-iphoneos"
bundle_id = "com.example.myapp"
app_name = "MyApp.app"
```
