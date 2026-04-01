mod adapters;

use std::collections::{HashMap, VecDeque};
use std::io::{self, stdout};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::core::actions;
use crate::core::clean::{self, CleanCategory};
use crate::core::config::{Config, ProjectConfig, ProjectKind};
use crate::core::device::{self, Device, DeviceState};
use crate::core::doctor;
use crate::core::error::{Result, TossError};
use crate::core::project;
use crate::core::state;
use crate::core::time::format_last_tossed;
use crate::core::xcrun;
use crate::tui::adapters::{
    append_log, draw_logs, format_event, prompt_input, BackgroundAdapter, BackgroundRequest,
    RatatuiAdapter,
};

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
    Devices,
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
    device_selected: usize,
    action_selected: usize,
    menu_selected: usize,
    devices: Vec<Device>,
    device_error: Option<String>,
}

impl AppState {
    fn new() -> Self {
        let mut app = Self {
            logs: VecDeque::from([
                "projects are now the primary view".to_string(),
                "left dock keeps devices visible; Enter refreshes, a aliases".to_string(),
            ]),
            focus: Focus::Projects,
            project_selected: 0,
            device_selected: 0,
            action_selected: 0,
            menu_selected: 0,
            devices: Vec::new(),
            device_error: None,
        };
        refresh_device_panel(&mut app, false);
        app
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

        if self.devices.is_empty() {
            self.device_selected = 0;
        } else if self.device_selected >= self.devices.len() {
            self.device_selected = self.devices.len() - 1;
        }

        if self.menu_selected >= MENU_ITEMS.len() {
            self.menu_selected = MENU_ITEMS.len().saturating_sub(1);
        }
    }

    fn move_focus_next(&mut self, config: &Config) {
        self.clamp(config);
        self.focus = match self.focus {
            Focus::Projects => Focus::Devices,
            Focus::Devices => {
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
            Focus::Devices => Focus::Projects,
            Focus::Actions => Focus::Devices,
            Focus::Menu => {
                if config.projects.is_empty() {
                    Focus::Devices
                } else {
                    Focus::Actions
                }
            }
        };
    }

    fn move_focus_right(&mut self, config: &Config) {
        self.clamp(config);
        self.focus = match self.focus {
            Focus::Projects | Focus::Devices => {
                if config.projects.is_empty() {
                    Focus::Menu
                } else {
                    Focus::Actions
                }
            }
            Focus::Actions => Focus::Menu,
            Focus::Menu => Focus::Menu,
        };
    }

    fn move_focus_left(&mut self, _config: &Config) {
        self.focus = match self.focus {
            Focus::Projects | Focus::Devices => Focus::Projects,
            Focus::Actions => Focus::Projects,
            Focus::Menu => Focus::Actions,
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
                KeyCode::Tab => app.move_focus_next(&config),
                KeyCode::BackTab => app.move_focus_prev(&config),
                KeyCode::Right => app.move_focus_right(&config),
                KeyCode::Left => app.move_focus_left(&config),
                KeyCode::Char('m') => app.focus = Focus::Menu,
                KeyCode::Esc => app.focus = Focus::Projects,
                KeyCode::Up | KeyCode::Char('k') => move_selection_up(&mut app),
                KeyCode::Down | KeyCode::Char('j') => move_selection_down(&mut app, &config),
                KeyCode::Char('a') if app.focus == Focus::Devices => {
                    alias_selected_device(terminal, &mut config, &mut app)?;
                }
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
        Focus::Devices => {
            if app.device_selected == 0 {
                app.focus = Focus::Projects;
            } else {
                app.device_selected = app.device_selected.saturating_sub(1);
            }
        }
        Focus::Actions => app.action_selected = app.action_selected.saturating_sub(1),
        Focus::Menu => app.menu_selected = app.menu_selected.saturating_sub(1),
    }
}

fn move_selection_down(app: &mut AppState, config: &Config) {
    match app.focus {
        Focus::Projects => {
            if app.project_selected + 1 < config.projects.len() {
                app.project_selected += 1;
            } else {
                app.focus = Focus::Devices;
            }
        }
        Focus::Devices => {
            if app.device_selected + 1 < app.devices.len() {
                app.device_selected += 1;
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
                app.focus = Focus::Devices;
            }
            Ok(false)
        }
        Focus::Devices => {
            refresh_device_panel(app, true);
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
            let task_project = project_name.clone();
            let result = run_progress_task(
                terminal,
                config,
                app,
                "Run app",
                format!("starting '{}'...", project_name),
                move |config, adapter| {
                    actions::run(
                        config,
                        Some(task_project.as_str()),
                        None,
                        None,
                        false,
                        adapter,
                    )
                },
            )?;
            append_log(
                &mut app.logs,
                format!(
                    "running '{}' on '{}'",
                    result.project_name, result.device_name
                ),
            );
        }
        ProjectAction::Install => {
            let task_project = project_name.clone();
            let result = run_progress_task(
                terminal,
                config,
                app,
                "Install app",
                format!("starting '{}'...", project_name),
                move |config, adapter| {
                    actions::install(
                        config,
                        Some(task_project.as_str()),
                        None,
                        None,
                        false,
                        adapter,
                    )
                },
            )?;
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
    config: &mut Config,
) -> Result<()> {
    let Some(path) = prompt_input(terminal, logs, "IPA file path", "", false)? else {
        return Ok(());
    };

    let launch_items = vec!["Install only".to_string(), "Install + Launch".to_string()];
    let launch =
        adapters::choose_from_list(terminal, logs, "After signing", &launch_items, 0)? == Some(1);

    let task_path = path.trim().to_string();
    let result = run_progress_task_for_logs(
        terminal,
        config,
        logs,
        "Sign IPA",
        format!("preparing '{}'...", task_path),
        move |config, adapter| {
            actions::sign_ipa(
                config,
                std::path::Path::new(task_path.as_str()),
                None,
                None,
                None,
                launch,
                adapter,
            )
        },
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

fn run_progress_task<T, F>(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    app: &mut AppState,
    title: &str,
    initial_status: String,
    action: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Config, &mut BackgroundAdapter) -> Result<T> + Send + 'static,
{
    run_progress_task_for_logs(
        terminal,
        config,
        &mut app.logs,
        title,
        initial_status,
        action,
    )
}

fn run_progress_task_for_logs<T, F>(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    logs: &mut VecDeque<String>,
    title: &str,
    initial_status: String,
    action: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Config, &mut BackgroundAdapter) -> Result<T> + Send + 'static,
{
    let worker_config = config.clone();
    let (request_tx, request_rx) = mpsc::channel::<BackgroundRequest>();
    let (result_tx, result_rx) = mpsc::sync_channel::<(Config, Result<T>)>(1);

    let worker = thread::spawn(move || {
        let mut worker_config = worker_config;
        let mut adapter = BackgroundAdapter::new(request_tx);
        let result = action(&mut worker_config, &mut adapter);
        let _ = result_tx.send((worker_config, result));
    });

    let mut status = initial_status;
    let mut tick = 0usize;

    loop {
        terminal
            .draw(|frame| draw_progress_overlay(frame, title, &status, tick, logs))
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        tick = tick.wrapping_add(1);

        while let Ok(request) = request_rx.try_recv() {
            match request {
                BackgroundRequest::Event(event) => {
                    status = format_event(&event);
                    append_log(logs, status.clone());
                }
                BackgroundRequest::Choose {
                    prompt,
                    items,
                    default,
                    response_tx,
                } => {
                    status = format!("waiting for input: {}", prompt);
                    let selection = match adapters::choose_from_list(
                        terminal, logs, &prompt, &items, default,
                    ) {
                        Ok(selection) => selection,
                        Err(TossError::UserCancelled(_)) => None,
                        Err(err) => {
                            let _ = response_tx.send(None);
                            let _ = worker.join();
                            return Err(err);
                        }
                    };
                    let _ = response_tx.send(selection);
                }
            }
        }

        match result_rx.try_recv() {
            Ok((updated_config, result)) => {
                let join_result = worker.join();
                if join_result.is_err() {
                    return Err(TossError::Io(io::Error::other("tui worker panicked")));
                }
                if result.is_ok() {
                    *config = updated_config;
                }
                return result;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                let _ = worker.join();
                return Err(TossError::Io(io::Error::other(
                    "tui worker disconnected unexpectedly",
                )));
            }
        }

        if event::poll(Duration::from_millis(120))
            .map_err(|e| TossError::Io(io::Error::other(e)))?
        {
            let _ = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?;
        }
    }
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

fn alias_selected_device(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &mut Config,
    app: &mut AppState,
) -> Result<()> {
    let Some(device) = app.devices.get(app.device_selected) else {
        append_log(&mut app.logs, "no device selected");
        return Ok(());
    };

    let Some(name) = prompt_input(terminal, &mut app.logs, "Alias name", "", false)? else {
        return Ok(());
    };

    let aliased = device::alias_device(config, &app.devices, &device.identifier, &name)?;
    append_log(
        &mut app.logs,
        format!(
            "aliased '{}' -> {} ({})",
            aliased.alias, aliased.device_name, aliased.udid
        ),
    );
    if aliased.is_default {
        append_log(&mut app.logs, "default device set automatically");
    }
    refresh_device_panel(app, false);
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
            Constraint::Percentage(33),
            Constraint::Percentage(45),
            Constraint::Percentage(22),
        ])
        .split(chunks[1]);

    draw_projects_panel(frame, content[0], config, app);
    draw_project_detail_panel(frame, content[1], config, app);
    draw_menu_panel(frame, content[2], app);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(20)])
        .split(chunks[2]);

    draw_devices_panel(frame, bottom[0], config, app);
    draw_logs(frame, bottom[1], &app.logs);

    let footer = Paragraph::new(
        "Tab cycle  ←/→ columns  ↑/↓ move  Enter select/refresh  a alias  m menu  q quit",
    )
    .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[3]);
}

fn draw_progress_overlay(
    frame: &mut Frame<'_>,
    title: &str,
    status: &str,
    tick: usize,
    logs: &VecDeque<String>,
) {
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
    draw_logs(frame, chunks[2], logs);

    let footer = Paragraph::new("Working... this modal stays in front until the task completes.")
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, chunks[3]);

    let popup = centered_rect(64, 34, size);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(block, popup);

    let inner = popup.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 2,
    });
    let text = vec![
        Line::from(Span::styled(
            "Operation in progress",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(status.to_string()),
        Line::from(""),
        Line::from(indeterminate_bar(
            inner.width.saturating_sub(2) as usize,
            tick,
        )),
        Line::from(""),
        Line::from("Keyboard input is temporarily disabled."),
    ];
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default());
    frame.render_widget(paragraph, inner);
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
        .block(panel_block("Projects", app.focus == Focus::Projects).border_style(border_style));
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
            panel_block(
                format!("Projects {}", config.projects.len()),
                app.focus == Focus::Projects,
            )
            .border_style(border_style),
        )
        .highlight_style(selected_style(app.focus == Focus::Projects))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(app.project_selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_devices_panel(frame: &mut Frame<'_>, area: Rect, config: &Config, app: &AppState) {
    let border_style = panel_border_style(app.focus == Focus::Devices);
    let title = format!("Devices {}", device_summary(&app.devices));

    if app.devices.is_empty() {
        let mut lines = vec![
            Line::from("○ no devices"),
            Line::from(""),
            Line::from("↻ refresh"),
            Line::from("a alias"),
        ];
        if let Some(error) = &app.device_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                error.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        }
        let empty = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel_block(title, app.focus == Focus::Devices).border_style(border_style));
        frame.render_widget(empty, area);
        return;
    }

    let alias_map: HashMap<&str, &str> = config
        .devices
        .aliases
        .iter()
        .map(|(name, udid)| (udid.as_str(), name.as_str()))
        .collect();

    let items: Vec<ListItem<'_>> = app
        .devices
        .iter()
        .map(|device| {
            let state_symbol = device_state_symbol(&device.state);
            let state_style = device_state_style(&device.state);
            let mut line = vec![
                Span::styled(format!("{} ", state_symbol), state_style),
                Span::styled(
                    device.name.as_str(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ];
            if is_default_device(config, device) {
                line.push(Span::styled(" ★", Style::default().fg(Color::Yellow)));
            }

            let alias = alias_map
                .get(device.udid.as_str())
                .map(|alias| format!(" @{}", alias))
                .unwrap_or_default();
            line.push(Span::styled(
                format!("  {} · iOS {}{}", device.model, device.os_version, alias),
                Style::default().fg(Color::Gray),
            ));

            ListItem::new(Line::from(line))
        })
        .collect();

    let list = List::new(items)
        .block(panel_block(title, app.focus == Focus::Devices).border_style(border_style))
        .highlight_style(selected_style(app.focus == Focus::Devices))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(app.device_selected));
    frame.render_stateful_widget(list, area, &mut state);

    let inner = area.inner(ratatui::layout::Margin {
        vertical: 0,
        horizontal: 1,
    });
    let hint_y = inner.y.saturating_add(inner.height.saturating_sub(1));
    let hint_area = Rect {
        x: inner.x,
        y: hint_y,
        width: inner.width,
        height: 1,
    };
    let hint_style = if app.device_error.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let hint_text = app
        .device_error
        .as_deref()
        .unwrap_or("↻ Enter refresh  a alias");
    frame.render_widget(Paragraph::new(hint_text).style(hint_style), hint_area);
}

fn draw_project_detail_panel(frame: &mut Frame<'_>, area: Rect, config: &Config, app: &AppState) {
    let Some((project_name, project)) = selected_project(config, app.project_selected) else {
        let empty = Paragraph::new("Select a project after adding one.")
            .block(panel_block("Project Detail", false));
        frame.render_widget(empty, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(9)])
        .split(area);

    let detail_block = panel_block(format!("{} Detail", project_name), false)
        .border_style(panel_border_style(false));
    let detail_inner = detail_block.inner(chunks[0]);
    frame.render_widget(detail_block, chunks[0]);

    let detail_grid = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(detail_inner);
    let detail_top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(detail_grid[0]);
    let detail_bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(detail_grid[1]);

    draw_detail_section(
        frame,
        detail_top[0],
        "Identity",
        &project_identity_lines(config, project_name, project),
    );
    draw_detail_section(
        frame,
        detail_top[1],
        "Source",
        &project_source_lines(project),
    );
    draw_detail_section(
        frame,
        detail_bottom[0],
        "Artifact",
        &project_artifact_lines(project),
    );
    draw_detail_section(
        frame,
        detail_bottom[1],
        "Recent",
        &project_recent_lines(config, project_name, project),
    );

    let actions = project_actions(project);
    let action_items: Vec<ListItem<'_>> = actions
        .iter()
        .map(|action| ListItem::new(Line::from(action.label(project))))
        .collect();
    let actions_list = List::new(action_items)
        .block(
            panel_block("Project Actions", app.focus == Focus::Actions)
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
        .constraints([Constraint::Min(10), Constraint::Length(5)])
        .split(area);

    let menu_items: Vec<ListItem<'_>> = MENU_ITEMS
        .iter()
        .map(|item| ListItem::new(Line::from(item.label())))
        .collect();
    let list = List::new(menu_items)
        .block(
            panel_block("Tools", app.focus == Focus::Menu)
                .border_style(panel_border_style(app.focus == Focus::Menu)),
        )
        .highlight_style(selected_style(app.focus == Focus::Menu))
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(Some(app.menu_selected));
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let helper = Paragraph::new(MENU_ITEMS[app.menu_selected].description())
        .wrap(Wrap { trim: false })
        .block(panel_block("Hint", false));
    frame.render_widget(helper, chunks[1]);
}

fn panel_block(title: impl Into<String>, active: bool) -> Block<'static> {
    let title = title.into();
    let title_style = if active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    };
    let prefix = if active { "● " } else { "· " };
    Block::default()
        .title(Line::from(vec![
            Span::styled(prefix, title_style),
            Span::styled(title, title_style),
        ]))
        .borders(Borders::ALL)
}

fn draw_detail_section(frame: &mut Frame<'_>, area: Rect, title: &str, lines: &[String]) {
    let visible = area.height.saturating_sub(2) as usize;
    let text: Vec<Line<'_>> = lines
        .iter()
        .take(visible.max(1))
        .map(|line| Line::from(line.as_str()))
        .collect();
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(panel_block(title, false));
    frame.render_widget(paragraph, area);
}

fn refresh_device_panel(app: &mut AppState, emit_log: bool) {
    match xcrun::list_devices() {
        Ok(devices) => {
            let summary = device_summary(&devices);
            app.devices = devices;
            app.device_error = None;
            if app.devices.is_empty() {
                app.device_selected = 0;
            } else if app.device_selected >= app.devices.len() {
                app.device_selected = app.devices.len() - 1;
            }
            if emit_log {
                append_log(&mut app.logs, format!("devices refreshed: {}", summary));
            }
        }
        Err(err) => {
            app.device_error = Some("! refresh failed".to_string());
            if emit_log {
                append_log(&mut app.logs, format!("device refresh failed: {}", err));
            }
        }
    }
}

fn device_summary(devices: &[Device]) -> String {
    let mut connected = 0usize;
    let mut paired = 0usize;
    let mut disconnected = 0usize;
    let mut unknown = 0usize;

    for device in devices {
        match device.state {
            DeviceState::Connected => connected += 1,
            DeviceState::Paired => paired += 1,
            DeviceState::Disconnected => disconnected += 1,
            DeviceState::Unknown(_) => unknown += 1,
        }
    }

    let mut parts = vec![format!("●{}", connected)];
    if paired > 0 {
        parts.push(format!("◐{}", paired));
    }
    if disconnected > 0 {
        parts.push(format!("○{}", disconnected));
    }
    if unknown > 0 {
        parts.push(format!("?{}", unknown));
    }
    parts.join(" ")
}

fn device_state_symbol(state: &DeviceState) -> char {
    match state {
        DeviceState::Connected => '●',
        DeviceState::Paired => '◐',
        DeviceState::Disconnected => '○',
        DeviceState::Unknown(_) => '?',
    }
}

fn device_state_style(state: &DeviceState) -> Style {
    match state {
        DeviceState::Connected => Style::default().fg(Color::Green),
        DeviceState::Paired => Style::default().fg(Color::Yellow),
        DeviceState::Disconnected => Style::default().fg(Color::DarkGray),
        DeviceState::Unknown(_) => Style::default().fg(Color::Magenta),
    }
}

fn is_default_device(config: &Config, device: &Device) -> bool {
    let Some(default_device) = config.defaults.device.as_deref() else {
        return false;
    };

    default_device == device.identifier
        || default_device == device.udid
        || config
            .devices
            .aliases
            .get(default_device)
            .is_some_and(|udid| udid == &device.udid)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn indeterminate_bar(width: usize, tick: usize) -> String {
    let width = width.max(20);
    let segment = (width / 4).max(6).min(width);
    let travel = width.saturating_sub(segment);
    let start = if travel == 0 {
        0
    } else {
        let cycle = travel * 2;
        let step = tick % cycle.max(1);
        if step <= travel {
            step
        } else {
            cycle - step
        }
    };

    let mut bar = String::with_capacity(width + 2);
    bar.push('[');
    for index in 0..width {
        if index >= start && index < start + segment {
            bar.push('=');
        } else {
            bar.push(' ');
        }
    }
    bar.push(']');
    bar
}

fn project_identity_lines(
    config: &Config,
    project_name: &str,
    project: &ProjectConfig,
) -> Vec<String> {
    let mut lines = vec![
        format!(
            "kind  {}",
            match project.kind {
                ProjectKind::Xcode => "xcode/app",
                ProjectKind::Ipa => "ipa",
            }
        ),
        format!(
            "default  {}",
            if config.defaults.project.as_deref() == Some(project_name) {
                "★ yes"
            } else {
                "· no"
            }
        ),
    ];

    if let Some(bundle_id) = &project.bundle_id {
        lines.push(format!("bundle  {}", bundle_id));
    } else {
        lines.push("bundle  <missing>".into());
    }

    if let Some(app_name) = &project.app_name {
        lines.push(format!("app     {}", app_name));
    }

    lines
}

fn project_source_lines(project: &ProjectConfig) -> Vec<String> {
    match project.kind {
        ProjectKind::Xcode => {
            let mut lines = vec![format!("build   {}", project.build_dir)];
            if let Some(path) = &project.path {
                lines.push(format!("source  {}", path));
            } else {
                lines.push("source  <unset>".into());
            }
            lines
        }
        ProjectKind::Ipa => {
            let mut lines = Vec::new();
            if let Some(path) = &project.ipa_path {
                lines.push(format!("cache   {}", path));
            } else {
                lines.push("cache   <unset>".into());
            }
            if let Some(name) = &project.original_name {
                lines.push(format!("file    {}", name));
            } else {
                lines.push("file    <unknown>".into());
            }
            lines
        }
    }
}

fn project_artifact_lines(project: &ProjectConfig) -> Vec<String> {
    match project.kind {
        ProjectKind::Xcode => vec![
            "mode    live build".into(),
            format!("dir     {}", project.build_dir),
            project
                .path
                .as_ref()
                .map(|path| format!("sync    {}", path))
                .unwrap_or_else(|| "sync    <unset>".into()),
        ],
        ProjectKind::Ipa => vec![
            "mode    cached ipa".into(),
            project
                .ipa_path
                .as_ref()
                .map(|path| format!("asset   {}", path))
                .unwrap_or_else(|| "asset   <unset>".into()),
            project
                .original_name
                .as_ref()
                .map(|name| format!("name    {}", name))
                .unwrap_or_else(|| "name    <unknown>".into()),
        ],
    }
}

fn project_recent_lines(
    config: &Config,
    project_name: &str,
    project: &ProjectConfig,
) -> Vec<String> {
    let mut lines = vec![
        format!(
            "tossed  {}",
            format_last_tossed(project.last_tossed_at.as_deref())
        ),
        format!(
            "role    {}",
            if config.defaults.project.as_deref() == Some(project_name) {
                "default"
            } else {
                "secondary"
            }
        ),
    ];

    match project.kind {
        ProjectKind::Xcode => {
            if let Some(app_name) = &project.app_name {
                lines.push(format!("target  {}", app_name));
            }
        }
        ProjectKind::Ipa => {
            if let Some(name) = &project.original_name {
                lines.push(format!("origin  {}", name));
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
