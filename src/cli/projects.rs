use console::Style;

use crate::cli::adapters::StrictCliAdapter;
use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::project;

pub fn add(config: &mut Config, path: &str, alias: Option<&str>) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let added = project::add_project(config, path, alias, &mut adapter)?;

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

pub fn list(config: &Config) -> Result<()> {
    if config.projects.is_empty() {
        println!("No projects registered. Use `toss projects add <path>` to add one.");
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

pub fn remove(config: &mut Config, alias: &str) -> Result<()> {
    let removed = project::remove_project(config, alias)?;
    println!("Removed project '{}'", removed.name);
    Ok(())
}
