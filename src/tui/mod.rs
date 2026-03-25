mod adapters;

use std::collections::VecDeque;
use std::io::{self, stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};

use crate::core::actions;
use crate::core::clean::{self, CleanCategory};
use crate::core::config::Config;
use crate::core::device::{self, DeviceState};
use crate::core::doctor;
use crate::core::error::{Result, TossError};
use crate::core::project;
use crate::core::state;
use crate::core::time::format_last_tossed;
use crate::core::xcrun;
use crate::tui::adapters::{RatatuiAdapter, append_log, draw_logs, prompt_input};

const MAIN_ITEMS: &[&str] = &[
    "Run app",
    "Install app",
    "Launch app",
    "Sign IPA",
    "Devices",
    "Projects",
    "State",
    "Doctor",
    "Clean",
    "Quit",
];

const DEVICE_ITEMS: &[&str] = &["List devices", "Alias a device", "Back"];
const PROJECT_ITEMS: &[&str] = &["List projects", "Add project", "Remove project", "Back"];
const STATE_ITEMS: &[&str] = &["Refresh", "Back"];
const DOCTOR_ITEMS: &[&str] = &["Refresh", "Back"];
const CLEAN_ITEMS: &[&str] = &[
    "Refresh inventory",
    "Clean temp profiles",
    "Clean selected category",
    "Back",
];

pub fn run() -> Result<()> {
    enable_raw_mode().map_err(|e| TossError::Io(io::Error::other(e)))?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).map_err(|e| TossError::Io(io::Error::other(e)))?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).map_err(|e| TossError::Io(io::Error::other(e)))?;
    terminal
        .clear()
        .map_err(|e| TossError::Io(io::Error::other(e)))?;

    let result = run_app(&mut terminal);

    disable_raw_mode().map_err(|e| TossError::Io(io::Error::other(e)))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|e| TossError::Io(io::Error::other(e)))?;
    terminal
        .show_cursor()
        .map_err(|e| TossError::Io(io::Error::other(e)))?;

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    let mut logs = VecDeque::from([
        "ratatui tui ready".to_string(),
        "use ↑/↓ to move, Enter to select, q to quit".to_string(),
    ]);
    let mut selected = 0usize;

    loop {
        terminal
            .draw(|frame| draw_main(frame, selected, &logs))
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < MAIN_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if handle_main_selection(terminal, &mut logs, selected)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }
}

fn handle_main_selection(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    selected: usize,
) -> Result<bool> {
    let mut config = Config::load()?;

    match selected {
        0 => {
            let mut adapter = RatatuiAdapter::new(terminal, logs);
            let result = actions::run(&mut config, None, None, None, false, &mut adapter)?;
            append_log(
                logs,
                format!(
                    "running '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        1 => {
            let mut adapter = RatatuiAdapter::new(terminal, logs);
            let result = actions::install(&mut config, None, None, None, false, &mut adapter)?;
            append_log(
                logs,
                format!(
                    "installed '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        2 => {
            let mut adapter = RatatuiAdapter::new(terminal, logs);
            let result = actions::launch(&config, None, None, &mut adapter)?;
            append_log(
                logs,
                format!(
                    "launched '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        3 => sign_ipa(terminal, logs, &config)?,
        4 => devices_menu(terminal, logs, &mut config)?,
        5 => projects_menu(terminal, logs, &mut config)?,
        6 => state_menu(terminal, logs, &config)?,
        7 => doctor_menu(terminal, logs, &config)?,
        8 => clean_menu(terminal, logs, &config)?,
        9 => return Ok(true),
        _ => {}
    }

    Ok(false)
}

fn sign_ipa(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &Config,
) -> Result<()> {
    let Some(path) = prompt_input(terminal, logs, "IPA file path", "", false)? else {
        return Ok(());
    };

    let launch_items = vec!["Install only".to_string(), "Install + Launch".to_string()];
    let launch =
        adapters::choose_from_list(terminal, logs, "After signing", &launch_items, 0)? == Some(1);

    let mut adapter = RatatuiAdapter::new(terminal, logs);
    let result = actions::sign_ipa(
        config,
        std::path::Path::new(path.trim()),
        None,
        None,
        None,
        launch,
        &mut adapter,
    )?;

    if result.launched {
        append_log(
            logs,
            format!("running signed app {}", result.final_bundle_id),
        );
    } else {
        append_log(
            logs,
            format!("installed signed app {}", result.final_bundle_id),
        );
    }

    Ok(())
}

fn devices_menu(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &mut Config,
) -> Result<()> {
    let mut selected = 0usize;

    loop {
        terminal
            .draw(|frame| draw_submenu(frame, "Devices", DEVICE_ITEMS, selected, logs))
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < DEVICE_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => match selected {
                    0 => log_devices(logs, config)?,
                    1 => alias_device(terminal, logs, config)?,
                    2 => return Ok(()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn projects_menu(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &mut Config,
) -> Result<()> {
    let mut selected = 0usize;

    loop {
        terminal
            .draw(|frame| draw_submenu(frame, "Projects", PROJECT_ITEMS, selected, logs))
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < PROJECT_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => match selected {
                    0 => log_projects(logs, config),
                    1 => add_project(terminal, logs, config)?,
                    2 => remove_project(terminal, logs, config)?,
                    3 => return Ok(()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn log_devices(logs: &mut VecDeque<String>, config: &Config) -> Result<()> {
    let devices = xcrun::list_devices()?;
    if devices.is_empty() {
        append_log(logs, "no devices found");
        return Ok(());
    }

    let alias_map: std::collections::HashMap<&str, &str> = config
        .devices
        .aliases
        .iter()
        .map(|(name, udid)| (udid.as_str(), name.as_str()))
        .collect();

    append_log(logs, "devices:");
    for (i, d) in devices.iter().enumerate() {
        let alias = alias_map
            .get(d.udid.as_str())
            .map(|a| format!(" alias={}", a))
            .unwrap_or_default();
        append_log(
            logs,
            format!(
                "  {}. {} {} {} {}{}",
                i + 1,
                d.name,
                d.model,
                d.os_version,
                d.state,
                alias
            ),
        );
    }

    Ok(())
}

fn state_menu(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &Config,
) -> Result<()> {
    let mut selected = 0usize;
    let mut details = build_state_details(config)?;

    loop {
        terminal
            .draw(|frame| {
                draw_detail_submenu(
                    frame,
                    "State",
                    STATE_ITEMS,
                    selected,
                    "State Details",
                    &details,
                    logs,
                )
            })
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < STATE_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => match selected {
                    0 => details = build_state_details(config)?,
                    1 => return Ok(()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn doctor_menu(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &Config,
) -> Result<()> {
    let mut selected = 0usize;
    let mut details = build_doctor_details(config)?;

    loop {
        terminal
            .draw(|frame| {
                draw_detail_submenu(
                    frame,
                    "Doctor",
                    DOCTOR_ITEMS,
                    selected,
                    "Doctor Report",
                    &details,
                    logs,
                )
            })
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < DOCTOR_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => match selected {
                    0 => details = build_doctor_details(config)?,
                    1 => return Ok(()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn clean_menu(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &Config,
) -> Result<()> {
    let mut selected = 0usize;
    let mut details = build_clean_details(config)?;

    loop {
        terminal
            .draw(|frame| {
                draw_detail_submenu(
                    frame,
                    "Clean",
                    CLEAN_ITEMS,
                    selected,
                    "Clean Inventory",
                    &details,
                    logs,
                )
            })
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < CLEAN_ITEMS.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => match selected {
                    0 => details = build_clean_details(config)?,
                    1 => {
                        clean_temp_profiles(logs, config)?;
                        details = build_clean_details(config)?;
                    }
                    2 => {
                        clean_selected_category(terminal, logs, config)?;
                        details = build_clean_details(config)?;
                    }
                    3 => return Ok(()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn alias_device(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &mut Config,
) -> Result<()> {
    let devices = xcrun::list_devices()?;
    let connected: Vec<_> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Connected)
        .collect();

    if connected.is_empty() {
        append_log(logs, "no connected devices found");
        return Ok(());
    }

    let items: Vec<String> = connected
        .iter()
        .map(|d| format!("{} ({})", d.name, d.model))
        .collect();
    let Some(index) =
        adapters::choose_from_list(terminal, logs, "Select device to alias", &items, 0)?
    else {
        return Ok(());
    };
    let Some(name) = prompt_input(terminal, logs, "Alias name", "", false)? else {
        return Ok(());
    };

    let aliased = device::alias_device(config, &devices, &connected[index].identifier, &name)?;
    append_log(
        logs,
        format!(
            "aliased '{}' -> {} ({})",
            aliased.alias, aliased.device_name, aliased.udid
        ),
    );
    if aliased.is_default {
        append_log(logs, "default device set automatically");
    }

    Ok(())
}

fn log_projects(logs: &mut VecDeque<String>, config: &Config) {
    if config.projects.is_empty() {
        append_log(logs, "no projects registered");
        return;
    }

    append_log(logs, "projects:");
    let default_project = config.defaults.project.as_deref();
    for (name, proj) in &config.projects {
        let marker = if Some(name.as_str()) == default_project {
            " default"
        } else {
            ""
        };
        append_log(logs, format!("  {}{}", name, marker));
        append_log(logs, format!("    build_dir: {}", proj.build_dir));
        if let Some(src) = &proj.path {
            append_log(logs, format!("    source: {}", src));
        }
        if let Some(app) = &proj.app_name {
            append_log(logs, format!("    app_name: {}", app));
        }
        if let Some(bundle_id) = &proj.bundle_id {
            append_log(logs, format!("    bundle_id: {}", bundle_id));
        }
        append_log(
            logs,
            format!(
                "    last tossed at: {}",
                format_last_tossed(proj.last_tossed_at.as_deref())
            ),
        );
    }
}

fn build_state_details(config: &Config) -> Result<Vec<String>> {
    let snapshot = state::collect(config)?;
    let mut lines = vec![
        "Local state".to_string(),
        format!("config file: {}", snapshot.config_path.display()),
        format!(
            "temp_bundle_prefix: {}",
            snapshot.temp_bundle_prefix.as_deref().unwrap_or("<unset>")
        ),
        format!(
            "team_id: {}",
            snapshot.team_id.as_deref().unwrap_or("<unset>")
        ),
        format!(
            "default_device: {}",
            snapshot.default_device.as_deref().unwrap_or("<unset>")
        ),
        format!(
            "default_project: {}",
            snapshot.default_project.as_deref().unwrap_or("<unset>")
        ),
        "".into(),
        format!("Device aliases ({})", snapshot.device_aliases.len()),
    ];
    if snapshot.device_aliases.is_empty() {
        lines.push("  <none>".into());
    } else {
        for (alias, udid) in &snapshot.device_aliases {
            lines.push(format!("  {} -> {}", alias, udid));
        }
    }

    lines.push("".into());
    lines.push(format!("Projects ({})", snapshot.projects.len()));
    if snapshot.projects.is_empty() {
        lines.push("  <none>".into());
    } else {
        for (name, project) in &snapshot.projects {
            lines.push(format!("  {}", name));
            lines.push(format!("    build_dir: {}", project.build_dir));
            if let Some(path) = &project.path {
                lines.push(format!("    source: {}", path));
            }
            if let Some(bundle_id) = &project.bundle_id {
                lines.push(format!("    bundle_id: {}", bundle_id));
            }
            if let Some(app_name) = &project.app_name {
                lines.push(format!("    app_name: {}", app_name));
            }
            lines.push(format!(
                "    last_tossed_at: {}",
                format_last_tossed(project.last_tossed_at.as_deref())
            ));
        }
    }

    lines.push("".into());
    lines.push(format!(
        "Provisioning profile dirs ({})",
        snapshot.profile_dirs.len()
    ));
    if snapshot.profile_dirs.is_empty() {
        lines.push("  <none>".into());
    } else {
        for dir in &snapshot.profile_dirs {
            lines.push(format!(
                "  {} ({} files)",
                dir.path.display(),
                dir.file_count
            ));
        }
    }

    lines.push("".into());
    lines.push("Provisioning profiles".into());
    match snapshot.profile_inspections {
        Ok(inspections) => {
            if inspections.is_empty() {
                lines.push("  <none>".into());
            } else {
                let prefix = snapshot.temp_bundle_prefix.as_deref();
                for inspection in inspections {
                    match inspection.profile {
                        Some(profile) => {
                            let marker = if prefix
                                .map(|value| profile.bundle_id_pattern.starts_with(value))
                                .unwrap_or(false)
                            {
                                " [temp]"
                            } else {
                                ""
                            };
                            lines.push(format!("  {}{}", profile.name, marker));
                            lines.push(format!("    bundle: {}", profile.bundle_id_pattern));
                            if !profile.team_ids.is_empty() {
                                lines.push(format!("    team: {}", profile.team_ids.join(", ")));
                            }
                            lines.push(format!("    path: {}", profile.path.display()));
                        }
                        None => {
                            lines.push("  <parse failed>".into());
                            lines.push(format!("    path: {}", inspection.path.display()));
                            lines.push(format!(
                                "    error: {}",
                                inspection.error.as_deref().unwrap_or("unknown error")
                            ));
                        }
                    }
                }
            }
        }
        Err(err) => lines.push(format!("  unavailable: {}", err)),
    }

    Ok(lines)
}

fn build_doctor_details(config: &Config) -> Result<Vec<String>> {
    let report = doctor::collect(config)?;
    let mut lines = vec!["toss doctor".into(), "".into()];
    for section in &report.sections {
        lines.push(section.title.into());
        for line in &section.lines {
            lines.push(format!(
                "  [{:<4}] {:<20} {}",
                line.status, line.label, line.detail
            ));
        }
        lines.push("".into());
    }
    lines.push(format!(
        "summary: {} failure(s), {} warning(s)",
        report.failures, report.warnings
    ));
    Ok(lines)
}

fn build_clean_details(config: &Config) -> Result<Vec<String>> {
    let cwd = std::env::current_dir()?;
    let report = clean::collect_report(config, &cwd)?;
    let mut lines = vec!["Local clean inventory".into()];
    if report.items.is_empty() {
        lines.push("  <nothing found>".into());
    }

    for item in &report.items {
        lines.push("".into());
        lines.push(format!(
            "{} ({})",
            item.category.display_name(),
            item.category.key()
        ));
        lines.push(format!("  owner: {}", item.category.owner()));
        lines.push(format!("  safety: {}", item.category.safety()));
        lines.push(format!("  size: {}", clean::format_bytes(item.size_bytes)));
        lines.push(format!("  path count: {}", item.path_count));
        lines.push(format!("  purpose: {}", item.category.purpose()));
        lines.push(format!(
            "  delete: {}",
            if item.deletable {
                format!("supported via {}", item.category.key())
            } else {
                "report only".into()
            }
        ));
        for path in item.paths.iter().take(3) {
            lines.push(format!("  path: {}", path.display()));
        }
        if item.paths.len() > 3 {
            lines.push(format!("  path: ... and {} more", item.paths.len() - 3));
        }
    }

    for note in &report.notes {
        lines.push(format!("note: {}", note));
    }

    Ok(lines)
}

fn clean_temp_profiles(logs: &mut VecDeque<String>, config: &Config) -> Result<()> {
    let summary = clean::legacy_temp_profile_cleanup(config)?;
    append_log(
        logs,
        format!(
            "cleanup complete: deleted {} path(s), reclaimed {}",
            summary.deleted_paths,
            clean::format_bytes(summary.reclaimed_bytes)
        ),
    );
    Ok(())
}

fn clean_selected_category(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &Config,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let report = clean::collect_report(config, &cwd)?;
    let categories: Vec<CleanCategory> = report
        .items
        .iter()
        .filter(|item| item.deletable)
        .map(|item| item.category)
        .collect();

    if categories.is_empty() {
        append_log(logs, "no deletable clean categories found");
        return Ok(());
    }

    let items: Vec<String> = categories
        .iter()
        .map(|category| format!("{} ({})", category.display_name(), category.key()))
        .collect();
    let Some(index) =
        adapters::choose_from_list(terminal, logs, "Select category to delete", &items, 0)?
    else {
        return Ok(());
    };

    let summary = clean::delete_categories(&report, &[categories[index]])?;
    append_log(
        logs,
        format!(
            "deleted {} path(s), reclaimed {}",
            summary.deleted_paths,
            clean::format_bytes(summary.reclaimed_bytes)
        ),
    );
    Ok(())
}

fn add_project(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &mut Config,
) -> Result<()> {
    let Some(path) = prompt_input(
        terminal,
        logs,
        "Project path (.app / build dir / source dir)",
        "",
        false,
    )?
    else {
        return Ok(());
    };
    let alias = prompt_input(terminal, logs, "Project alias (optional)", "", true)?;

    let mut adapter = RatatuiAdapter::new(terminal, logs);
    let added = project::add_project(config, &path, alias.as_deref(), &mut adapter)?;
    append_log(logs, format!("added project '{}'", added.name));
    append_log(logs, format!("build_dir: {}", added.build_dir.display()));
    if let Some(src) = &added.source_dir {
        append_log(logs, format!("source: {}", src.display()));
    }

    Ok(())
}

fn remove_project(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    logs: &mut VecDeque<String>,
    config: &mut Config,
) -> Result<()> {
    if config.projects.is_empty() {
        append_log(logs, "no projects registered");
        return Ok(());
    }

    let aliases: Vec<String> = config.projects.keys().cloned().collect();
    let Some(index) =
        adapters::choose_from_list(terminal, logs, "Select project to remove", &aliases, 0)?
    else {
        return Ok(());
    };

    let removed = project::remove_project(config, &aliases[index])?;
    append_log(logs, format!("removed project '{}'", removed.name));
    Ok(())
}

fn draw_main(frame: &mut Frame<'_>, selected: usize, logs: &VecDeque<String>) {
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(size);

    let title = Paragraph::new("toss").block(
        Block::default()
            .borders(Borders::ALL)
            .title("iOS App Deployer"),
    );
    frame.render_widget(title, chunks[0]);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(20)])
        .split(chunks[1]);

    draw_menu(frame, content[0], MAIN_ITEMS, selected, "Menu");
    draw_logs(frame, content[1], logs);

    let footer = Paragraph::new("Enter select  q quit  Esc back")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_submenu(
    frame: &mut Frame<'_>,
    title: &str,
    items: &[&str],
    selected: usize,
    logs: &VecDeque<String>,
) {
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(size);

    let header =
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("Section"));
    frame.render_widget(header, chunks[0]);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(20)])
        .split(chunks[1]);

    draw_menu(frame, content[0], items, selected, title);
    draw_logs(frame, content[1], logs);

    let footer = Paragraph::new("Enter select  Esc back")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_detail_submenu(
    frame: &mut Frame<'_>,
    title: &str,
    items: &[&str],
    selected: usize,
    detail_title: &str,
    details: &[String],
    logs: &VecDeque<String>,
) {
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(size);

    let header =
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("Section"));
    frame.render_widget(header, chunks[0]);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(20)])
        .split(chunks[1]);

    draw_menu(frame, content[0], items, selected, title);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(8)])
        .split(content[1]);

    draw_details(frame, right[0], detail_title, details);
    draw_logs(frame, right[1], logs);

    let footer = Paragraph::new("Enter select  Esc back")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_details(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &str,
    details: &[String],
) {
    let visible = area.height.saturating_sub(2) as usize;
    let text: Vec<Line<'_>> = details
        .iter()
        .take(visible)
        .map(|line| Line::from(line.as_str()))
        .collect();
    let paragraph = Paragraph::new(text).block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn draw_menu(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    items: &[&str],
    selected: usize,
    title: &str,
) {
    let list_items: Vec<ListItem<'_>> = items
        .iter()
        .map(|item| ListItem::new(Line::from(*item)))
        .collect();

    let list = List::new(list_items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}
