use std::path::Path;

use crate::core::clean::{self, CleanCategory};
use crate::core::config::Config;
use crate::core::error::Result;

pub fn run(config: &Config, delete: &[String], all_safe: bool, cwd: &Path) -> Result<()> {
    let report = clean::collect_report(config, cwd)?;
    print_report(&report);

    let categories = clean::parse_delete_categories(delete, all_safe)?;
    if categories.is_empty() {
        println!();
        println!(
            "No cleanup requested. Use `toss clean --delete {}` or `toss clean --all-safe`.",
            CleanCategory::all()
                .iter()
                .filter(|category| category.supports_delete())
                .map(|category| category.key())
                .collect::<Vec<_>>()
                .join(",")
        );
        return Ok(());
    }

    println!();
    println!("Deleting categories:");
    for category in &categories {
        println!("  {} ({})", category.display_name(), category.key());
    }

    let summary = clean::delete_categories(&report, &categories)?;
    println!(
        "Deleted {} path(s), reclaimed {}.",
        summary.deleted_paths,
        clean::format_bytes(summary.reclaimed_bytes)
    );
    Ok(())
}

pub fn run_legacy_cleanup(config: &Config) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let report = clean::collect_report(config, &cwd)?;
    if let Some(item) = report
        .items
        .iter()
        .find(|item| item.category == CleanCategory::TempProfiles)
    {
        println!(
            "Removing {} temporary provisioning profile(s) for '{}':",
            item.path_count,
            config
                .signing
                .temp_bundle_prefix
                .as_deref()
                .unwrap_or("<unset>")
        );
        for path in &item.paths {
            println!("  {}", path.display());
        }
    } else {
        println!("No temporary provisioning profiles found.");
        return Ok(());
    }

    let summary = clean::legacy_temp_profile_cleanup(config)?;
    println!(
        "Cleanup complete. Deleted {} path(s), reclaimed {}.",
        summary.deleted_paths,
        clean::format_bytes(summary.reclaimed_bytes)
    );
    Ok(())
}

fn print_report(report: &clean::CleanReport) {
    println!("Local clean inventory");
    if report.items.is_empty() {
        println!("  <nothing found>");
    }

    for item in &report.items {
        println!();
        println!("{} ({})", item.category.display_name(), item.category.key());
        println!("  owner: {}", item.category.owner());
        println!("  safety: {}", item.category.safety());
        println!("  size: {}", clean::format_bytes(item.size_bytes));
        println!("  path count: {}", item.path_count);
        println!("  purpose: {}", item.category.purpose());
        println!(
            "  delete: {}",
            if item.deletable {
                format!("supported via `--delete {}`", item.category.key())
            } else {
                "report only".into()
            }
        );

        let shown = display_paths(item);
        for path in &shown {
            println!("  path: {}", path.display());
        }
        if item.paths.len() > shown.len() {
            println!("  path: ... and {} more", item.paths.len() - shown.len());
        }
    }

    if !report.notes.is_empty() {
        println!();
        println!("Notes");
        for note in &report.notes {
            println!("  - {}", note);
        }
    }
}

fn display_paths(item: &clean::CleanItem) -> Vec<&Path> {
    if item.paths.len() <= 5 {
        return item.paths.iter().map(|path| path.as_path()).collect();
    }
    item.paths.iter().take(3).map(|path| path.as_path()).collect()
}
