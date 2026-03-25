mod adapters;

use std::collections::{HashMap, VecDeque};
use std::io::{self, stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::core::actions;
use crate::core::clean::{self, CleanCategory};
use crate::core::config::{Config, ProjectConfig, ProjectKind};
use crate::core::device::{self, DeviceState};
use crate::core::doctor;
use crate::core::error::{Result, TossError};
use crate::core::project;
use crate::core::state;
use crate::core::time::format_last_tossed;
use crate::core::xcrun;
use crate::tui::adapters::{RatatuiAdapter, append_log, draw_logs, prompt_input};

const MENU_ITEMS: &[MenuAction] = &[
    MenuAction::AddProject,
    MenuAction::Devices,
    MenuAction::State,
    MenuAction::Doctor,
    MenuAction::Clean,
    MenuAction::SignIpa,
    MenuAction::Quit,
];

const DEVICE_ITEMS: &[&str] = &["List devices", "Alias a device", "Back"];
const STATE_ITEMS: &[&str] = &["Refresh", "Back"];
const DOCTOR_ITEMS: &[&str] = &["Refresh", "Back"];
const CLEAN_ITEMS: &[&str] = &[
    "Refresh inventory",
    "Clean temp profiles",
    "Clean selected category",
    "Back",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Projects,
    Actions,
    Menu,
}

#[derive(Debug, Clone, Copy)]
enum ProjectAction {
    Run,
    Install,
    Launch,
    SetDefault,
    Remove,
}

impl ProjectAction {
    fn label(self, project: &ProjectConfig) -> &'static str {
        match (self, project.kind) {
            (Self::Run, ProjectKind::Ipa) => "Install + Launch IPA",
            (Self::Run, ProjectKind::Xcode) => "Run app",
            (Self::Install, ProjectKind::Ipa) => "Install IPA",
            (Self::Install, ProjectKind::Xcode) => "Install app",
            (Self::Launch, _) => "Launch app",
            (Self::SetDefault, _) => "Set as default",
            (Self::Remove, _) => "Remove project",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum MenuAction {
    AddProject,
    Devices,
    State,
    Doctor,
    Clean,
    SignIpa,
    Quit,
}

impl MenuAction {
    fn label(self) -> &'static str {
        match self {
            Self::AddProject => "Add project",
            Self::Devices => "Devices",
            Self::State => "State",
            Self::Doctor => "Doctor",
            Self::Clean => "Clean",
            Self::SignIpa => "Sign IPA",
            Self::Quit => "Quit",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::AddProject => "Register a new Xcode/App or IPA project",
            Self::Devices => "Inspect devices and manage aliases",
            Self::State => "Show local toss state and caches",
            Self::Doctor => "Run environment diagnostics",
            Self::Clean => "Inspect and delete local artifacts",
            Self::SignIpa => "Resign any IPA and deploy it",
            Self::Quit => "Exit the TUI",
        }
    }
}

struct AppState {
    logs: VecDeque<String>,
    focus: Focus,
    project_selected: usize,
    action_selected: usize,
    menu_selected: usize,
}

impl AppState {
    fn new() -> Self {
        Self {
            logs: VecDeque::from([
                "projects are now the primary view".to_string(),
                "use Tab or ←/→ to switch panels, Enter to open project actions".to_string(),
            ]),
            focus: Focus::Projects,
            project_selected: 0,
            action_selected: 0,
            menu_selected: 0,
        }
    }

    fn clamp(&mut self, config: &Config) {
        let project_count = config.projects.len();
        if project_count == 0 {
            self.project_selected = 0;
            if self.focus == Focus::Actions {
                self.focus = Focus::Projects;
            }
        } else if self.project_selected >= project_count {
            self.project_selected = project_count - 1;
        }

        let action_count = selected_project(config, self.project_selected)
            .map(|(_, project)| project_actions(project).len())
            .unwrap_or(0);
        if action_count == 0 {
            self.action_selected = 0;
        } else if self.action_selected >= action_count {
            self.action_selected = action_count - 1;
        }

        if self.menu_selected >= MENU_ITEMS.len() {
            self.menu_selected = MENU_ITEMS.len().saturating_sub(1);
        }
    }

    fn move_focus_next(&mut self, config: &Config) {
        self.clamp(config);
        self.focus = match self.focus {
            Focus::Projects => {
                if config.projects.is_empty() {
                    Focus::Menu
                } else {
                    Focus::Actions
                }
            }
            Focus::Actions => Focus::Menu,
            Focus::Menu => Focus::Projects,
        };
    }

    fn move_focus_prev(&mut self, config: &Config) {
        self.clamp(config);
        self.focus = match self.focus {
            Focus::Projects => Focus::Menu,
            Focus::Actions => Focus::Projects,
            Focus::Menu => {
                if config.projects.is_empty() {
                    Focus::Projects
                } else {
                    Focus::Actions
                }
            }
        };
    }
}

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
    let mut config = Config::load()?;
    let mut app = AppState::new();
    app.clamp(&config);

    loop {
        terminal
            .draw(|frame| draw_home(frame, &config, &app))
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Tab | KeyCode::Right => app.move_focus_next(&config),
                KeyCode::BackTab | KeyCode::Left => app.move_focus_prev(&config),
                KeyCode::Char('m') => app.focus = Focus::Menu,
                KeyCode::Esc => app.focus = Focus::Projects,
                KeyCode::Up | KeyCode::Char('k') => move_selection_up(&mut app),
                KeyCode::Down | KeyCode::Char('j') => move_selection_down(&mut app, &config),
                KeyCode::Enter => {
                    if handle_enter(terminal, &mut config, &mut app)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        app.clamp(&config);
    }
}

fn move_selection_up(app: &mut AppState) {
    match app.focus {
        Focus::Projects => app.project_selected = app.project_selected.saturating_sub(1),
        Focus::Actions => app.action_selected = app.action_selected.saturating_sub(1),
        Focus::Menu => app.menu_selected = app.menu_selected.saturating_sub(1),
    }
}

fn move_selection_down(app: &mut AppState, config: &Config) {
    match app.focus {
        Focus::Projects => {
            if app.project_selected + 1 < config.projects.len() {
                app.project_selected += 1;
            }
        }
        Focus::Actions => {
            let count = selected_project(config, app.project_selected)
                .map(|(_, project)| project_actions(project).len())
                .unwrap_or(0);
            if app.action_selected + 1 < count {
                app.action_selected += 1;
            }
        }
        Focus::Menu => {
            if app.menu_selected + 1 < MENU_ITEMS.len() {
                app.menu_selected += 1;
            }
        }
    }
}

fn handle_enter(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    app: &mut AppState,
) -> Result<bool> {
    match app.focus {
        Focus::Projects => {
            if !config.projects.is_empty() {
                app.action_selected = 0;
                app.focus = Focus::Actions;
            } else {
                app.focus = Focus::Menu;
            }
            Ok(false)
        }
        Focus::Actions => {
            handle_project_action(terminal, config, app)?;
            Ok(false)
        }
        Focus::Menu => handle_menu_action(terminal, config, app),
    }
}

fn handle_project_action(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    app: &mut AppState,
) -> Result<()> {
    let Some((project_name, project)) = selected_project(config, app.project_selected) else {
        return Ok(());
    };
    let project_name = project_name.to_string();
    let actions = project_actions(project);
    let Some(action) = actions.get(app.action_selected).copied() else {
        return Ok(());
    };

    match action {
        ProjectAction::Run => {
            let mut adapter = RatatuiAdapter::new(terminal, &mut app.logs);
            let result =
                actions::run(config, Some(&project_name), None, None, false, &mut adapter)?;
            append_log(
                &mut app.logs,
                format!(
                    "running '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        ProjectAction::Install => {
            let mut adapter = RatatuiAdapter::new(terminal, &mut app.logs);
            let result =
                actions::install(config, Some(&project_name), None, None, false, &mut adapter)?;
            append_log(
                &mut app.logs,
                format!(
                    "installed '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        ProjectAction::Launch => {
            let mut adapter = RatatuiAdapter::new(terminal, &mut app.logs);
            let result = actions::launch(config, Some(&project_name), None, &mut adapter)?;
            append_log(
                &mut app.logs,
                format!(
                    "launched '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        ProjectAction::SetDefault => {
            config.defaults.project = Some(project_name.clone());
            config.save()?;
            append_log(
                &mut app.logs,
                format!("default project set to '{}'", project_name),
            );
        }
        ProjectAction::Remove => {
            let confirm_items = vec!["Remove project".to_string(), "Cancel".to_string()];
            let confirmed = adapters::choose_from_list(
                terminal,
                &app.logs,
                &format!("Remove '{}'?", project_name),
                &confirm_items,
                1,
            )? == Some(0);
            if confirmed {
                let removed = project::remove_project(config, &project_name)?;
                append_log(&mut app.logs, format!("removed project '{}'", removed.name));
                if removed.cleared_default {
                    append_log(&mut app.logs, "default project cleared");
                }
                app.focus = Focus::Projects;
            }
        }
    }

    Ok(())
}

fn handle_menu_action(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    app: &mut AppState,
) -> Result<bool> {
    let action = MENU_ITEMS[app.menu_selected];
    match action {
        MenuAction::AddProject => {
            add_project(terminal, &mut app.logs, config)?;
            app.focus = if config.projects.is_empty() {
                Focus::Menu
            } else {
                Focus::Projects
            };
        }
        MenuAction::Devices => devices_menu(terminal, &mut app.logs, config)?,
        MenuAction::State => state_menu(terminal, &mut app.logs, config)?,
        MenuAction::Doctor => doctor_menu(terminal, &mut app.logs, config)?,
        MenuAction::Clean => clean_menu(terminal, &mut app.logs, config)?,
        MenuAction::SignIpa => sign_ipa(terminal, &mut app.logs, config)?,
        MenuAction::Quit => return Ok(true),
    }

    Ok(false)
}

fn selected_project(config: &Config, selected: usize) -> Option<(&str, &ProjectConfig)> {
    config
        .projects
        .iter()
        .nth(selected)
        .map(|(name, project)| (name.as_str(), project))
}

fn project_actions(project: &ProjectConfig) -> Vec<ProjectAction> {
    let mut actions = vec![ProjectAction::Run, ProjectAction::Install];
    if !project.is_ipa() {
        actions.push(ProjectAction::Launch);
    }
    actions.push(ProjectAction::SetDefault);
    actions.push(ProjectAction::Remove);
    actions
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

fn log_devices(logs: &mut VecDeque<String>, config: &Config) -> Result<()> {
    let devices = xcrun::list_devices()?;
    if devices.is_empty() {
        append_log(logs, "no devices found");
        return Ok(());
    }

    let alias_map: HashMap<&str, &str> = config
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
        push_state_project_group(
            &mut lines,
            "  Xcode/App Projects",
            &snapshot.projects,
            ProjectKind::Xcode,
        );
        push_state_project_group(
            &mut lines,
            "  IPA Projects",
            &snapshot.projects,
            ProjectKind::Ipa,
        );
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
    let add_items = vec!["Xcode/App Project".to_string(), "IPA Project".to_string()];
    let Some(kind_index) =
        adapters::choose_from_list(terminal, logs, "Add project type", &add_items, 0)?
    else {
        return Ok(());
    };

    let prompt = if kind_index == 1 {
        "IPA file path"
    } else {
        "Project path (.app / build dir / source dir)"
    };
    let Some(path) = prompt_input(terminal, logs, prompt, "", false)? else {
        return Ok(());
    };
    let alias = prompt_input(terminal, logs, "Project alias (optional)", "", true)?;

    let added = if kind_index == 1 {
        project::add_ipa_project(config, &path, alias.as_deref())?
    } else {
        let mut adapter = RatatuiAdapter::new(terminal, logs);
        project::add_project(config, &path, alias.as_deref(), &mut adapter)?
    };
    append_log(logs, format!("added project '{}'", added.name));
    append_log(
        logs,
        format!(
            "type: {}",
            match added.kind {
                ProjectKind::Xcode => "xcode/app",
                ProjectKind::Ipa => "ipa",
            }
        ),
    );
    if let Some(build_dir) = &added.build_dir {
        append_log(logs, format!("build_dir: {}", build_dir.display()));
    }
    if let Some(src) = &added.source_dir {
        append_log(logs, format!("source: {}", src.display()));
    }
    if let Some(path) = &added.cached_ipa_path {
        append_log(logs, format!("cached_ipa: {}", path.display()));
    }
    if added.is_default {
        append_log(logs, "default project set automatically");
    }

    Ok(())
}

fn draw_home(frame: &mut Frame<'_>, config: &Config, app: &AppState) {
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(16),
            Constraint::Length(9),
            Constraint::Length(2),
        ])
        .split(size);

    let header = Paragraph::new("toss").block(
        Block::default()
            .borders(Borders::ALL)
            .title("Project Workspace"),
    );
    frame.render_widget(header, chunks[0]);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(41),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    draw_projects_panel(frame, content[0], config, app);
    draw_project_detail_panel(frame, content[1], config, app);
    draw_menu_panel(frame, content[2], app);

    draw_logs(frame, chunks[2], &app.logs);

    let footer = Paragraph::new(
        "Tab/←/→ switch  ↑/↓ move  Enter open/select  Esc back to projects  m menu  q quit",
    )
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[3]);
}

fn draw_projects_panel(frame: &mut Frame<'_>, area: Rect, config: &Config, app: &AppState) {
    let border_style = panel_border_style(app.focus == Focus::Projects);
    if config.projects.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from("No projects registered."),
            Line::from(""),
            Line::from("Use the Menu panel to add one."),
        ])
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title("Projects")
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(empty, area);
        return;
    }

    let list_items: Vec<ListItem<'_>> = config
        .projects
        .iter()
        .map(|(name, project)| {
            let mut lines = vec![Line::from(Span::styled(
                name.as_str(),
                Style::default().add_modifier(Modifier::BOLD),
            ))];

            let kind = match project.kind {
                ProjectKind::Xcode => "Xcode/App",
                ProjectKind::Ipa => "IPA",
            };
            let default_marker = if config.defaults.project.as_deref() == Some(name.as_str()) {
                "  default"
            } else {
                ""
            };
            lines.push(Line::from(format!("{kind}{default_marker}")));
            if let Some(bundle_id) = &project.bundle_id {
                lines.push(Line::from(bundle_id.as_str()));
            } else {
                lines.push(Line::from("<bundle id unavailable>"));
            }
            ListItem::new(lines)
        })
        .collect();

    let list = List::new(list_items)
        .block(
            Block::default()
                .title(format!("Projects ({})", config.projects.len()))
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(selected_style(app.focus == Focus::Projects))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(app.project_selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_project_detail_panel(frame: &mut Frame<'_>, area: Rect, config: &Config, app: &AppState) {
    let Some((project_name, project)) = selected_project(config, app.project_selected) else {
        let empty = Paragraph::new("Select a project after adding one.").block(
            Block::default()
                .title("Project Detail")
                .borders(Borders::ALL),
        );
        frame.render_widget(empty, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(9)])
        .split(area);

    let details = project_detail_lines(config, project_name, project);
    let detail_text: Vec<Line<'_>> = details.into_iter().map(Line::from).collect();
    let detail = Paragraph::new(detail_text)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(format!("{} Detail", project_name))
                .borders(Borders::ALL)
                .border_style(panel_border_style(app.focus == Focus::Projects)),
        );
    frame.render_widget(detail, chunks[0]);

    let actions = project_actions(project);
    let action_items: Vec<ListItem<'_>> = actions
        .iter()
        .map(|action| ListItem::new(Line::from(action.label(project))))
        .collect();
    let actions_list = List::new(action_items)
        .block(
            Block::default()
                .title("Project Actions")
                .borders(Borders::ALL)
                .border_style(panel_border_style(app.focus == Focus::Actions)),
        )
        .highlight_style(selected_style(app.focus == Focus::Actions))
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(Some(app.action_selected));
    frame.render_stateful_widget(actions_list, chunks[1], &mut state);
}

fn draw_menu_panel(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(6)])
        .split(area);

    let menu_items: Vec<ListItem<'_>> = MENU_ITEMS
        .iter()
        .map(|item| ListItem::new(Line::from(item.label())))
        .collect();
    let list = List::new(menu_items)
        .block(
            Block::default()
                .title("Menu")
                .borders(Borders::ALL)
                .border_style(panel_border_style(app.focus == Focus::Menu)),
        )
        .highlight_style(selected_style(app.focus == Focus::Menu))
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(Some(app.menu_selected));
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let helper = Paragraph::new(MENU_ITEMS[app.menu_selected].description())
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Hint").borders(Borders::ALL));
    frame.render_widget(helper, chunks[1]);
}

fn project_detail_lines(
    config: &Config,
    project_name: &str,
    project: &ProjectConfig,
) -> Vec<String> {
    let mut lines = vec![
        format!(
            "kind: {}",
            match project.kind {
                ProjectKind::Xcode => "xcode/app",
                ProjectKind::Ipa => "ipa",
            }
        ),
        format!(
            "default: {}",
            if config.defaults.project.as_deref() == Some(project_name) {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "last tossed: {}",
            format_last_tossed(project.last_tossed_at.as_deref())
        ),
    ];

    if let Some(bundle_id) = &project.bundle_id {
        lines.push(format!("bundle id: {}", bundle_id));
    }

    match project.kind {
        ProjectKind::Xcode => {
            lines.push(format!("build dir: {}", project.build_dir));
            if let Some(path) = &project.path {
                lines.push(format!("source dir: {}", path));
            }
            if let Some(app_name) = &project.app_name {
                lines.push(format!("app name: {}", app_name));
            }
        }
        ProjectKind::Ipa => {
            if let Some(path) = &project.ipa_path {
                lines.push(format!("cached ipa: {}", path));
            }
            if let Some(name) = &project.original_name {
                lines.push(format!("original file: {}", name));
            }
        }
    }

    lines
}

fn panel_border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn selected_style(active: bool) -> Style {
    if active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

fn push_state_project_group(
    lines: &mut Vec<String>,
    title: &str,
    projects: &[(String, ProjectConfig)],
    kind: ProjectKind,
) {
    lines.push(title.into());
    let mut found = false;
    for (name, project) in projects {
        if project.kind != kind {
            continue;
        }
        found = true;
        lines.push(format!("    {}", name));
        lines.push(format!(
            "      type: {}",
            match project.kind {
                ProjectKind::Xcode => "xcode/app",
                ProjectKind::Ipa => "ipa",
            }
        ));
        if project.kind == ProjectKind::Ipa {
            if let Some(path) = &project.ipa_path {
                lines.push(format!("      cached_ipa: {}", path));
            }
            if let Some(name) = &project.original_name {
                lines.push(format!("      original_name: {}", name));
            }
        } else {
            lines.push(format!("      build_dir: {}", project.build_dir));
            if let Some(path) = &project.path {
                lines.push(format!("      source: {}", path));
            }
            if let Some(app_name) = &project.app_name {
                lines.push(format!("      app_name: {}", app_name));
            }
        }
        if let Some(bundle_id) = &project.bundle_id {
            lines.push(format!("      bundle_id: {}", bundle_id));
        }
        lines.push(format!(
            "      last_tossed_at: {}",
            format_last_tossed(project.last_tossed_at.as_deref())
        ));
    }
    if !found {
        lines.push("    <none>".into());
    }
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

fn draw_details(frame: &mut Frame<'_>, area: Rect, title: &str, details: &[String]) {
    let visible = area.height.saturating_sub(2) as usize;
    let text: Vec<Line<'_>> = details
        .iter()
        .take(visible)
        .map(|line| Line::from(line.as_str()))
        .collect();
    let paragraph = Paragraph::new(text).block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn draw_menu(frame: &mut Frame<'_>, area: Rect, items: &[&str], selected: usize, title: &str) {
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
