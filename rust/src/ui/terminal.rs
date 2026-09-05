//! Ratatui dashboard and noninteractive line-rendering mode.

use std::io::{self, IsTerminal, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use unicode_width::UnicodeWidthChar;

use super::command::{CommandInput, CommandSubmission, EditAction, UiCommand};
use super::state::{DashboardSnapshot, TaskSnapshot, TaskStatus, event_message};
use crate::runtime::RuntimeEvent;

static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const TIMING_WIDTH: u16 = 19;

pub(super) fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub(super) fn render_plain(event: &RuntimeEvent<'_>) -> io::Result<()> {
    if let Some(message) = event_message(event) {
        writeln!(io::stderr().lock(), "[{message}]")?;
    }
    Ok(())
}

pub(super) struct DashboardTerminal {
    terminal: Terminal<CrosstermBackend<io::Stderr>>,
    command: CommandInput,
    tick: usize,
    lease: TerminalLease,
    task_offset: usize,
    task_anchor: Option<(u64, String)>,
    message_end: Option<u64>,
    last_message: u64,
}

impl DashboardTerminal {
    pub(super) fn enter() -> io::Result<Self> {
        TERMINAL_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| io::Error::other("another Workflow dashboard owns the terminal"))?;
        let lease = TerminalLease;
        if let Err(source) = enable_raw_mode() {
            drop(lease);
            return Err(source);
        }
        let mut stderr = io::stderr();
        if let Err(source) = execute!(
            stderr,
            EnterAlternateScreen,
            Clear(ClearType::All),
            MoveTo(0, 0),
            Hide,
            EnableMouseCapture
        ) {
            restore_terminal();
            drop(lease);
            return Err(source);
        }
        let backend = CrosstermBackend::new(io::stderr());
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(source) => {
                restore_terminal();
                drop(lease);
                return Err(source);
            }
        };
        Ok(Self {
            terminal,
            command: CommandInput::default(),
            tick: 0,
            lease,
            task_offset: 0,
            task_anchor: None,
            message_end: None,
            last_message: 0,
        })
    }

    pub(super) fn poll_command(&mut self) -> io::Result<Option<CommandSubmission>> {
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if key.code == KeyCode::PageDown {
                        self.task_offset += 5;
                        self.task_anchor = None;
                        continue;
                    }
                    if key.code == KeyCode::PageUp {
                        self.task_offset = self.task_offset.saturating_sub(5);
                        self.task_anchor = None;
                        continue;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Up {
                        self.message_end = Some(
                            self.message_end
                                .unwrap_or(self.last_message)
                                .saturating_sub(3)
                                .max(1),
                        );
                        continue;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Down {
                        let next = self
                            .message_end
                            .unwrap_or(self.last_message)
                            .saturating_add(3);
                        self.message_end = (next < self.last_message).then_some(next);
                        continue;
                    }
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(Some(CommandSubmission::Parsed(UiCommand::Interrupt)));
                    }
                    if let Some(action) = edit_action(key.code, key.modifiers)
                        && let Some(submission) = self.command.edit(action)
                    {
                        return Ok(Some(submission));
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn clear_command(&mut self) {
        self.command.clear();
    }

    pub(super) fn draw(&mut self, snapshot: &DashboardSnapshot) -> io::Result<()> {
        if let Some(anchor) = &self.task_anchor
            && let Some(index) = snapshot
                .tasks
                .iter()
                .position(|task| (task.replicate, &task.identity) == (anchor.0, &anchor.1))
        {
            self.task_offset = index;
        }
        self.task_offset = self.task_offset.min(snapshot.tasks.len().saturating_sub(1));
        self.task_anchor = snapshot
            .tasks
            .get(self.task_offset)
            .map(|t| (t.replicate, t.identity.clone()));
        self.last_message = snapshot.messages.last().map_or(0, |m| m.sequence);
        let task_offset = self.task_offset;
        let message_end = self.message_end;
        let tick = self.tick;
        let command = self.command.text();
        let command_cursor = self.command.cursor();
        self.tick = self.tick.wrapping_add(1);
        self.terminal.draw(|frame| {
            let area = frame.area();
            let [header, tasks, messages, command_area] = Layout::vertical([
                Constraint::Length(if area.height >= 24 { 6 } else { 5 }),
                Constraint::Min(4),
                Constraint::Length(if area.height >= 24 { 9 } else { 5 }),
                Constraint::Length(3),
            ])
            .areas(area);
            render_header(frame, header, snapshot);
            render_tasks(frame, tasks, snapshot, tick, task_offset);
            render_messages(frame, messages, snapshot, message_end);
            render_command(
                frame,
                command_area,
                &command,
                command_cursor,
                snapshot.exit_requested,
                snapshot.execution_finished,
            );
        })?;
        Ok(())
    }
}

impl Drop for DashboardTerminal {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        restore_terminal();
        let _ = &self.lease;
    }
}

struct TerminalLease;

impl Drop for TerminalLease {
    fn drop(&mut self) {
        TERMINAL_OWNED.store(false, Ordering::Release);
    }
}

fn restore_terminal() {
    let mut stderr = io::stderr();
    let _ = execute!(stderr, DisableMouseCapture, Show, LeaveAlternateScreen);
    let _ = stderr.flush();
    let _ = disable_raw_mode();
}

fn edit_action(code: KeyCode, modifiers: KeyModifiers) -> Option<EditAction> {
    match code {
        KeyCode::Char(character)
            if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(EditAction::Insert(character))
        }
        KeyCode::Backspace => Some(EditAction::Backspace),
        KeyCode::Delete => Some(EditAction::Delete),
        KeyCode::Left => Some(EditAction::Left),
        KeyCode::Right => Some(EditAction::Right),
        KeyCode::Home => Some(EditAction::Home),
        KeyCode::End => Some(EditAction::End),
        KeyCode::Esc => Some(EditAction::Clear),
        KeyCode::Enter => Some(EditAction::Submit),
        _ => None,
    }
}

fn render_header(frame: &mut ratatui::Frame<'_>, area: Rect, snapshot: &DashboardSnapshot) {
    let counts = counts(snapshot);
    let elapsed = format_duration(snapshot.started.elapsed());
    let output = snapshot
        .output
        .as_deref()
        .map_or_else(|| "planning".to_owned(), |path| path.display().to_string());
    let text = vec![
        Line::from(vec![
            Span::styled(
                "Scientific Workflow",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " · Total time {elapsed} · {}",
                if snapshot.execution_finished {
                    "finished"
                } else {
                    snapshot.control_status
                }
            )),
        ]),
        Line::from(format!(
            "running={} pending={} completed={} failed={} cancelled={} skipped={}",
            counts.running,
            counts.pending,
            counts.completed,
            counts.failed,
            counts.cancelled,
            counts.skipped
        )),
        Line::from(format!(
            "replicates={} · visible active tasks={}",
            snapshot.replicate_count,
            snapshot.tasks.len()
        )),
        Line::from(format!("output={output}")),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Study ")),
        area,
    );
}

fn render_tasks(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    tick: usize,
    offset: usize,
) {
    let header = Row::new(["task", "status", "progress", "elapsed / ETA"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let available = usize::from(area.height.saturating_sub(3));
    let mut rows = snapshot
        .tasks
        .iter()
        .skip(offset)
        .take(available)
        .map(|task| task_row(task, tick, snapshot.now))
        .collect::<Vec<_>>();
    if snapshot.tasks.len().saturating_sub(offset) > available && available > 0 {
        rows.truncate(available.saturating_sub(1));
        rows.push(Row::new([
            Cell::from(format!(
                "… {} more tasks",
                snapshot.tasks.len() - offset - rows.len()
            )),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ]));
    }
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(37),
            Constraint::Length(12),
            Constraint::Percentage(32),
            Constraint::Length(TIMING_WIDTH),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(task_panel_title(snapshot)),
    );
    frame.render_widget(table, area);
}

fn task_panel_title(snapshot: &DashboardSnapshot) -> String {
    format!(
        " Active groups · {} tasks · PgUp/PgDn · outcomes in Messages ",
        snapshot.tasks.len()
    )
}

fn task_row(task: &TaskSnapshot, tick: usize, now: Instant) -> Row<'static> {
    let status_style = match task.status {
        TaskStatus::Pending | TaskStatus::Skipped => Style::default().fg(Color::DarkGray),
        TaskStatus::Running => Style::default().fg(Color::Cyan),
        TaskStatus::Completed => Style::default().fg(Color::Green),
        TaskStatus::Failed => Style::default().fg(Color::Red),
        TaskStatus::Cancelled => Style::default().fg(Color::Yellow),
    };
    let progress = progress_text(task, tick);
    let timing = timing_text(task, now);
    let task_label = if task.detail.is_empty() {
        format!("{} · {}", task.label, display_kind(&task.kind))
    } else {
        format!("{} · {}", task.label, task.detail)
    };
    Row::new([
        Cell::from(format!(
            "r{} / {} · {task_label}",
            task.replicate, task.phase
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(task.status.label()).style(status_style),
        Cell::from(progress).style(Style::default().fg(Color::Cyan)),
        Cell::from(timing),
    ])
}

fn display_kind(kind: &str) -> &str {
    match kind {
        "execution_unit" => "unit",
        other => other,
    }
}

fn progress_text(task: &TaskSnapshot, tick: usize) -> String {
    if task.status == TaskStatus::Pending || task.status == TaskStatus::Skipped {
        return String::new();
    }
    if let Some(progress) = &task.program_progress {
        return progress.clone();
    }
    if task.kind != "execution_unit" {
        return if task.status == TaskStatus::Running {
            format!("{} {}", SPINNER[tick % SPINNER.len()], task.kind)
        } else {
            task.kind.clone()
        };
    }
    match task.target {
        Some(target) => {
            let ratio = if target == 0 {
                1.0
            } else {
                (task.iteration as f64 / target as f64).clamp(0.0, 1.0)
            };
            let filled = (ratio * 16.0).round() as usize;
            format!(
                "{}{} {}/{}",
                "█".repeat(filled),
                "░".repeat(16 - filled),
                task.iteration,
                target
            )
        }
        None => format!(
            "{} iteration {}",
            SPINNER[tick % SPINNER.len()],
            task.iteration
        ),
    }
}

fn timing_text(task: &TaskSnapshot, now: Instant) -> String {
    let Some(started) = task.started else {
        return String::new();
    };
    let elapsed = task
        .finished
        .unwrap_or(now)
        .saturating_duration_since(started);
    timing_text_for(
        elapsed,
        task.iteration,
        if task.kind == "execution_unit" {
            task.target
        } else {
            None
        },
    )
}

fn timing_text_for(elapsed: Duration, iteration: u64, target: Option<u64>) -> String {
    let eta = target.and_then(|target| {
        if iteration == 0 || iteration >= target {
            None
        } else {
            let remaining = target - iteration;
            elapsed
                .checked_mul(u32::try_from(remaining).ok()?)?
                .checked_div(u32::try_from(iteration).ok()?)
        }
    });
    match eta {
        Some(eta) => format!("{} / {}", format_duration(elapsed), format_duration(eta)),
        None => format_duration(elapsed),
    }
}

fn render_messages(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    end: Option<u64>,
) {
    let width = usize::from(area.width.saturating_sub(2)).max(1);
    let visible = usize::from(area.height.saturating_sub(2));
    let mut lines = Vec::new();
    for message in snapshot
        .messages
        .iter()
        .filter(|m| end.is_none_or(|end| m.sequence <= end))
    {
        let color = match message.level.as_str() {
            "debug" => Color::DarkGray,
            "warning" => Color::Yellow,
            "error" => Color::Red,
            "success" => Color::Green,
            _ => Color::White,
        };
        for source_line in message.text.split('\n') {
            let mut line = String::new();
            let mut used = 0;
            for character in source_line.chars() {
                let extent = character.width().unwrap_or(0);
                if used + extent > width && !line.is_empty() {
                    lines.push(Line::styled(
                        std::mem::take(&mut line),
                        Style::default().fg(color),
                    ));
                    used = 0;
                }
                line.push(character);
                used += extent;
            }
            lines.push(Line::styled(line, Style::default().fg(color)));
        }
    }
    let text = lines
        .into_iter()
        .rev()
        .take(visible)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Messages · last 100 · Ctrl-Up/Down "),
        ),
        area,
    );
}

fn render_command(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    command: &str,
    cursor: usize,
    exit_requested: bool,
    execution_finished: bool,
) {
    let title = if execution_finished {
        " Command · finished · type exit then Enter "
    } else if exit_requested {
        " Command · cancelling · waiting for process-tree cleanup "
    } else {
        " Command · pause / resume / exit / exit --force "
    };
    frame.render_widget(
        Paragraph::new(command)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
    let x = area.x.saturating_add(1).saturating_add(
        u16::try_from(cursor)
            .unwrap_or(u16::MAX)
            .min(area.width.saturating_sub(2)),
    );
    frame.set_cursor_position(Position::new(x, area.y.saturating_add(1)));
}

#[derive(Default)]
struct Counts {
    pending: usize,
    running: usize,
    completed: usize,
    failed: usize,
    cancelled: usize,
    skipped: usize,
}

fn counts(snapshot: &DashboardSnapshot) -> Counts {
    let [pending, running, completed, failed, cancelled, skipped] = snapshot.totals;
    Counts {
        pending,
        running,
        completed,
        failed,
        cancelled,
        skipped,
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_value_fits_its_column_without_clipping_eta() {
        let timing = timing_text_for(Duration::from_secs(3_661), 25, Some(100));

        assert_eq!(timing, "01:01:01 / 03:03:03");
        assert_eq!(timing.chars().count(), usize::from(TIMING_WIDTH));
    }

    #[test]
    fn execution_unit_uses_concise_display_kind() {
        assert_eq!(display_kind("execution_unit"), "unit");
        assert_eq!(display_kind("program"), "program");
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    #[test]
    fn narrow_messages_keep_newest_wrapped_message_visible() {
        let mut state = super::super::state::DashboardState::new();
        state.push_message("old long message ".repeat(80));
        state.push_message("newest visible".into());
        let snapshot = state.snapshot();
        let mut terminal = ratatui::Terminal::new(TestBackend::new(35, 6)).unwrap();
        terminal
            .draw(|frame| render_messages(frame, frame.area(), &snapshot, None))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("newest visible"));
    }
}
