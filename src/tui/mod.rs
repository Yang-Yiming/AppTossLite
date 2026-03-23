mod actions;
mod adapters;
mod devices;
mod projects;

use console::Style;
use dialoguer::Select;

use crate::core::config::Config;
use crate::core::error::{Result, TossError};

pub fn run() -> Result<()> {
    let bold = Style::new().bold();
    println!("\n{}", bold.apply_to("toss — iOS App Deployer"));

    loop {
        println!();
        let items = &[
            "Run app (install + launch)",
            "Install app",
            "Launch app",
            "Sign IPA (resign + deploy)",
            "Local state",
            "Clean inventory / cleanup",
            "Doctor",
            "Devices",
            "Projects",
            "Quit",
        ];

        let selection = Select::new()
            .with_prompt("What would you like to do?")
            .items(items)
            .default(0)
            .interact()
            .map_err(|e| TossError::UserCancelled(e.to_string()))?;

        let mut config = Config::load()?;

        let result = match selection {
            0 => actions::run(&config),
            1 => actions::install(&config),
            2 => actions::launch(&config),
            3 => actions::sign(&config),
            4 => crate::cli::state::show(&config),
            5 => {
                let cwd = std::env::current_dir()?;
                crate::cli::clean::run(&config, &[], false, &cwd)
            }
            6 => crate::cli::doctor::run(&config),
            7 => devices::menu(&mut config),
            8 => projects::menu(&mut config),
            9 => return Ok(()),
            _ => unreachable!(),
        };

        if let Err(e) = result {
            let red = Style::new().red().bold();
            eprintln!("{} {}", red.apply_to("error:"), e);
        }
    }
}
