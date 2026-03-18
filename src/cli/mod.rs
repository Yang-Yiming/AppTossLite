pub mod actions;
pub mod devices;
pub mod projects;

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
    /// Install an app onto a device
    Install {
        /// Project alias
        project: String,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
    },
    /// Launch an app on a device
    Launch {
        /// Project alias
        project: String,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
    },
    /// Install and launch an app (build → deploy → run)
    Run {
        /// Project alias
        project: String,
        /// Device alias, UDID, or index
        #[arg(short, long)]
        device: Option<String>,
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
    /// Register a project build directory
    Add {
        /// Path to the build directory containing the .app bundle
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
        Commands::Install { project, device } => {
            actions::install(&config, &project, device.as_deref())
        }
        Commands::Launch { project, device } => {
            actions::launch(&config, &project, device.as_deref())
        }
        Commands::Run { project, device } => actions::run(&config, &project, device.as_deref()),
    }
}
