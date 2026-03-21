pub mod actions;
pub mod config;
pub mod devices;
pub mod projects;
pub mod sign;

use clap::{Parser, Subcommand};

use crate::core::config::Config;
use crate::core::error::Result;

#[derive(Parser)]
#[command(name = "toss", version, about = "Deploy iOS apps to connected devices")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List connected devices and manage aliases
    Devices {
        #[command(subcommand)]
        action: Option<DevicesAction>,
    },
    /// Manage registered projects
    Projects {
        #[command(subcommand)]
        action: ProjectsAction,
    },
    /// Manage toss configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Install an app onto a device
    Install {
        /// Project alias (uses default if omitted)
        project: Option<String>,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
        /// Use pre-built .app from build_dir instead of building from source
        #[arg(long)]
        prebuilt: bool,
        /// Show full xcodebuild output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Launch an app on a device
    Launch {
        /// Project alias (uses default if omitted)
        project: Option<String>,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
    },
    /// Install and launch an app (build → deploy → run)
    Run {
        /// Project alias (uses default if omitted)
        project: Option<String>,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
        /// Use pre-built .app from build_dir instead of building from source
        #[arg(long)]
        prebuilt: bool,
        /// Show full xcodebuild output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Resign an IPA and deploy to device
    Sign {
        /// Path to .ipa file
        ipa: String,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
        /// Launch after installing
        #[arg(short, long)]
        launch: bool,
        /// Signing identity (name substring or hash prefix)
        #[arg(long)]
        identity: Option<String>,
        /// Path to .mobileprovision file
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DevicesAction {
    /// Alias a device by UDID, index, or name
    Alias {
        /// Device identifier (UDID or index from `toss devices`)
        device: String,
        /// Alias name to assign
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ProjectsAction {
    /// Register a project (source dir with .xcodeproj, build dir, or .app path)
    Add {
        /// Path to project source directory, build directory, or .app bundle
        path: String,
        /// Alias for the project
        #[arg(long)]
        alias: Option<String>,
    },
    /// List registered projects
    List,
    /// Remove a registered project
    Remove {
        /// Project alias to remove
        alias: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show the current configuration
    Show,
    /// Print the config file path
    Path,
    /// Set the default device
    #[command(name = "set-default-device")]
    SetDefaultDevice {
        /// Device alias or UDID
        name: String,
    },
    /// Set the default project
    #[command(name = "set-default-project")]
    SetDefaultProject {
        /// Project alias
        name: String,
    },
}

pub fn dispatch(command: Commands) -> Result<()> {
    let mut config = Config::load()?;

    match command {
        Commands::Devices { action } => match action {
            None => devices::list(&config),
            Some(DevicesAction::Alias { device, name }) => {
                devices::alias(&mut config, &device, &name)
            }
        },
        Commands::Projects { action } => match action {
            ProjectsAction::Add { path, alias } => {
                projects::add(&mut config, &path, alias.as_deref())
            }
            ProjectsAction::List => projects::list(&config),
            ProjectsAction::Remove { alias } => projects::remove(&mut config, &alias),
        },
        Commands::Config { action } => match action {
            ConfigAction::Show => config::show(&config),
            ConfigAction::Path => config::path(),
            ConfigAction::SetDefaultDevice { name } => {
                config::set_default_device(&mut config, &name)
            }
            ConfigAction::SetDefaultProject { name } => {
                config::set_default_project(&mut config, &name)
            }
        },
        Commands::Install {
            project,
            device,
            prebuilt,
            verbose,
        } => actions::install(
            &config,
            project.as_deref(),
            device.as_deref(),
            prebuilt,
            verbose,
        ),
        Commands::Launch { project, device } => {
            actions::launch(&config, project.as_deref(), device.as_deref())
        }
        Commands::Run {
            project,
            device,
            prebuilt,
            verbose,
        } => actions::run(
            &config,
            project.as_deref(),
            device.as_deref(),
            prebuilt,
            verbose,
        ),
        Commands::Sign {
            ipa,
            device,
            launch,
            identity,
            profile,
        } => sign::sign(
            &config,
            &ipa,
            device.as_deref(),
            identity.as_deref(),
            profile.as_deref(),
            launch,
        ),
    }
}
