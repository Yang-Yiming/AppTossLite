use std::collections::VecDeque;
use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::core::error::{Result, TossError};
use crate::core::interaction::{WorkflowAdapter, WorkflowEvent};

pub struct RatatuiAdapter<'a, B: Backend> {
    terminal: &'a mut Terminal<B>,
    logs: &'a mut VecDeque<String>,
}

impl<'a, B: Backend> RatatuiAdapter<'a, B> {
    pub fn new(terminal: &'a mut Terminal<B>, logs: &'a mut VecDeque<String>) -> Self {
        Self { terminal, logs }
    }
}

impl<B: Backend> WorkflowAdapter for RatatuiAdapter<'_, B> {
    fn emit(&mut self, event: WorkflowEvent) -> Result<()> {
        append_log(self.logs, format_event(&event));
        Ok(())
    }

    fn choose(&mut self, prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
        choose_from_list(self.terminal, self.logs, prompt, items, default)
    }
}

pub fn append_log(logs: &mut VecDeque<String>, line: impl Into<String>) {
    logs.push_back(line.into());
    while logs.len() > 200 {
        logs.pop_front();
    }
}

pub fn format_event(event: &WorkflowEvent) -> String {
    match event {
        WorkflowEvent::Warning { message } => format!("warning: {}", message),
        WorkflowEvent::Building {
            project,
            scheme,
            device_udid,
        } => format!("building {} ({}) for {}...", project, scheme, device_udid),
        WorkflowEvent::BuildSucceeded => "build succeeded".into(),
        WorkflowEvent::Installing {
            app_path,
            device_name,
        } => format!("installed {} on {}", app_path.display(), device_name),
        WorkflowEvent::Launching {
            bundle_id,
            device_name,
        } => format!("launched {} on {}", bundle_id, device_name),
        WorkflowEvent::Signing {
            ipa_name,
            device_name,
        } => format!("signing {} -> {}", ipa_name, device_name),
        WorkflowEvent::ExtractedBundle {
            bundle_id,
            app_name,
        } => format!("extracted {} ({})", bundle_id, app_name),
        WorkflowEvent::UsingIdentity { identity_name } => format!("identity: {}", identity_name),
        WorkflowEvent::SigningPlanStep {
            kind,
            original_bundle_id,
            final_bundle_id,
            profile_name,
        } => format!(
            "plan: {} {} -> {} using {}",
            kind, original_bundle_id, final_bundle_id, profile_name
        ),
        WorkflowEvent::TemporaryBundleId {
            original_bundle_id,
            temporary_bundle_id,
        } => format!(
            "switching {} to temporary bundle id {}",
            original_bundle_id, temporary_bundle_id
        ),
        WorkflowEvent::AutoProvisioning {
            kind,
            bundle_id,
            device_udid,
        } => format!(
            "auto-provisioning {} {} for device {}",
            kind, bundle_id, device_udid
        ),
        WorkflowEvent::BundleIdRewritten { from, to } => {
            format!("bundle id rewritten: {} -> {}", from, to)
        }
        WorkflowEvent::CleanedTemporaryProfiles { count } => {
            format!("cleaned {} temporary provisioning profile(s)", count)
        }
    }
}

pub fn draw_logs(frame: &mut Frame<'_>, area: Rect, logs: &VecDeque<String>) {
    let mut items: Vec<ListItem<'_>> = logs
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|line| ListItem::new(Line::from(line.as_str())))
        .collect();
    items.reverse();

    let list = List::new(items).block(Block::default().title("Logs").borders(Borders::ALL));
    frame.render_widget(list, area);
}

pub fn choose_from_list<B: Backend>(
    terminal: &mut Terminal<B>,
    logs: &VecDeque<String>,
    prompt: &str,
    items: &[String],
    default: usize,
) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut selected = default.min(items.len().saturating_sub(1));

    loop {
        terminal
            .draw(|frame| {
                let size = frame.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(6), Constraint::Length(10)])
                    .split(size);

                draw_logs(frame, chunks[1], logs);

                let popup = centered_rect(70, 60, chunks[0]);
                frame.render_widget(Clear, popup);
                let block = Block::default().title(prompt).borders(Borders::ALL);
                frame.render_widget(block, popup);

                let inner = popup.inner(ratatui::layout::Margin {
                    vertical: 1,
                    horizontal: 1,
                });
                let list_items: Vec<ListItem<'_>> = items
                    .iter()
                    .map(|item| ListItem::new(Line::from(item.as_str())))
                    .collect();
                let list = List::new(list_items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("› ");
                let mut state = ListState::default();
                state.select(Some(selected));
                frame.render_stateful_widget(list, inner, &mut state);
            })
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < items.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => return Ok(Some(selected)),
                KeyCode::Esc => return Err(TossError::UserCancelled(prompt.to_string())),
                _ => {}
            }
        }
    }
}

pub fn prompt_input<B: Backend>(
    terminal: &mut Terminal<B>,
    logs: &VecDeque<String>,
    title: &str,
    initial: &str,
    allow_empty: bool,
) -> Result<Option<String>> {
    let mut value = initial.to_string();

    loop {
        terminal
            .draw(|frame| {
                let size = frame.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(6), Constraint::Length(10)])
                    .split(size);

                draw_logs(frame, chunks[1], logs);

                let popup = centered_rect(70, 35, chunks[0]);
                frame.render_widget(Clear, popup);
                let block = Block::default().title(title).borders(Borders::ALL);
                frame.render_widget(block, popup);

                let inner = popup.inner(ratatui::layout::Margin {
                    vertical: 1,
                    horizontal: 1,
                });
                let text = vec![
                    Line::from(value.as_str()),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(" confirm  "),
                        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(" cancel"),
                    ]),
                ];
                let paragraph = Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .block(Block::default());
                frame.render_widget(paragraph, inner);
            })
            .map_err(|e| TossError::Io(io::Error::other(e)))?;

        if let Event::Key(key) = event::read().map_err(|e| TossError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Enter => {
                    if allow_empty || !value.trim().is_empty() {
                        return Ok(Some(value.trim().to_string()));
                    }
                }
                KeyCode::Esc => return Ok(None),
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(ch) => value.push(ch),
                _ => {}
            }
        }
    }
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
