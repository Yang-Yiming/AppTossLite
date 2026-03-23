use console::Style;
use dialoguer::{Input, Select};

use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::project;
use crate::tui::adapters::DialoguerAdapter;

pub fn menu(config: &mut Config) -> Result<()> {
    loop {
        let items = &["List projects", "Add project", "Remove project", "Back"];

        let selection = Select::new()
            .with_prompt("Projects")
            .items(items)
            .default(0)
            .interact()
            .map_err(|e| TossError::UserCancelled(e.to_string()))?;

        match selection {
            0 => list(config)?,
            1 => {
                if let Err(e) = add(config) {
                    let red = Style::new().red().bold();
                    eprintln!("{} {}", red.apply_to("error:"), e);
                }
            }
            2 => {
                if let Err(e) = remove(config) {
                    let red = Style::new().red().bold();
                    eprintln!("{} {}", red.apply_to("error:"), e);
                }
            }
            3 => return Ok(()),
            _ => unreachable!(),
        }
    }
}

fn list(config: &Config) -> Result<()> {
    if config.projects.is_empty() {
        println!("No projects registered.");
        return Ok(());
    }

    let default_project = config.defaults.project.as_deref();
    for (name, proj) in &config.projects {
        let marker = if Some(name.as_str()) == default_project {
            " (default)"
        } else {
            ""
        };
        println!("{}{}:", name, marker);
        println!("  build_dir: {}", proj.build_dir);
        if let Some(src) = &proj.path {
            println!("  source:    {}", src);
        }
        if let Some(app) = &proj.app_name {
            println!("  app_name:  {}", app);
        }
        if let Some(bid) = &proj.bundle_id {
            println!("  bundle_id: {}", bid);
        }
        println!();
    }

    Ok(())
}

fn add(config: &mut Config) -> Result<()> {
    let path: String = Input::new()
        .with_prompt("Project path (source dir, build dir, or .app)")
        .interact_text()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let alias: String = Input::new()
        .with_prompt("Project alias (leave empty for auto)")
        .allow_empty(true)
        .interact_text()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let alias_opt = if alias.is_empty() {
        None
    } else {
        Some(alias.as_str())
    };

    let mut adapter = DialoguerAdapter;
    let added = project::add_project(config, &path, alias_opt, &mut adapter)?;
    let green = Style::new().green().bold();
    println!("{} Added project '{}'", green.apply_to("✓"), added.name);
    println!("  build_dir: {}", added.build_dir.display());
    if let Some(src) = &added.source_dir {
        println!("  source:    {}", src.display());
    }
    if let Some(app) = &added.app_name {
        println!("  app_name:  {}", app);
    }
    if let Some(bid) = &added.bundle_id {
        println!("  bundle_id: {}", bid);
    }
    if added.is_default {
        let dim = Style::new().dim();
        println!("{}", dim.apply_to("  (set as default project)"));
    }
    Ok(())
}

fn remove(config: &mut Config) -> Result<()> {
    if config.projects.is_empty() {
        println!("No projects registered.");
        return Ok(());
    }

    let names: Vec<&String> = config.projects.keys().collect();

    let selection = Select::new()
        .with_prompt("Select project to remove")
        .items(&names)
        .default(0)
        .interact()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let alias = names[selection].clone();
    let removed = project::remove_project(config, &alias)?;
    println!("Removed project '{}'", removed.name);
    Ok(())
}
