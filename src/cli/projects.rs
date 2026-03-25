use console::Style;

use crate::cli::adapters::StrictCliAdapter;
use crate::core::config::{Config, ProjectConfig, ProjectKind};
use crate::core::error::Result;
use crate::core::project;
use crate::core::time::format_last_tossed;

pub fn add(config: &mut Config, path: &str, ipa: bool, alias: Option<&str>) -> Result<()> {
    let added = if ipa {
        project::add_ipa_project(config, path, alias)?
    } else {
        let mut adapter = StrictCliAdapter;
        project::add_project(config, path, alias, &mut adapter)?
    };

    let green = Style::new().green().bold();
    println!("{} Added project '{}'", green.apply_to("✓"), added.name);
    println!(
        "  type:      {}",
        match added.kind {
            ProjectKind::Xcode => "xcode/app",
            ProjectKind::Ipa => "ipa",
        }
    );
    if let Some(build_dir) = &added.build_dir {
        println!("  build_dir: {}", build_dir.display());
    }
    if let Some(src) = &added.source_dir {
        println!("  source:    {}", src.display());
    }
    if let Some(path) = &added.cached_ipa_path {
        println!("  cached:    {}", path.display());
    }
    if let Some(name) = &added.original_name {
        println!("  original:  {}", name);
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
    let mut xcode_projects = Vec::new();
    let mut ipa_projects = Vec::new();

    for (name, project) in &config.projects {
        if project.kind == ProjectKind::Ipa {
            ipa_projects.push((name, project));
        } else {
            xcode_projects.push((name, project));
        }
    }

    print_project_group("Xcode/App Projects", &xcode_projects, default_project);
    print_project_group("IPA Projects", &ipa_projects, default_project);

    Ok(())
}

pub fn remove(config: &mut Config, alias: &str) -> Result<()> {
    let removed = project::remove_project(config, alias)?;
    println!("Removed project '{}'", removed.name);
    Ok(())
}

fn print_project_group(
    title: &str,
    projects: &[(&String, &ProjectConfig)],
    default_project: Option<&str>,
) {
    println!("{title}");
    if projects.is_empty() {
        println!("  <none>");
        println!();
        return;
    }

    for (name, project) in projects {
        let marker = if Some(name.as_str()) == default_project {
            " (default)"
        } else {
            ""
        };
        println!("{}{}:", name, marker);
        print_project_details(project);
        println!();
    }
}

fn print_project_details(project: &ProjectConfig) {
    println!(
        "  type:      {}",
        match project.kind {
            ProjectKind::Xcode => "xcode/app",
            ProjectKind::Ipa => "ipa",
        }
    );
    if project.kind == ProjectKind::Ipa {
        if let Some(path) = &project.ipa_path {
            println!("  cached:    {}", path);
        }
        if let Some(name) = &project.original_name {
            println!("  original:  {}", name);
        }
    } else {
        println!("  build_dir: {}", project.build_dir);
        if let Some(src) = &project.path {
            println!("  source:    {}", src);
        }
        if let Some(app) = &project.app_name {
            println!("  app_name:  {}", app);
        }
    }
    if let Some(bid) = &project.bundle_id {
        println!("  bundle_id: {}", bid);
    }
    println!(
        "  last tossed at: {}",
        format_last_tossed(project.last_tossed_at.as_deref())
    );
}
