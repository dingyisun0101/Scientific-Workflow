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
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

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
        })
    }

    pub(super) fn poll_command(&mut self) -> io::Result<Option<CommandSubmission>> {
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
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
        let tick = self.tick;
        let command = self.command.text();
        let command_cursor = self.command.cursor();
        self.tick = self.tick.wrapping_add(1);
        self.terminal.draw(|frame| {
            let area = frame.area();
            let [header, tasks, messages, command_area] = Layout::vertical([
                Constraint::Length(4),
                Constraint::Min(6),
                Constraint::Length(9),
                Constraint::Length(3),
            ])
            .areas(area);
            render_header(frame, header, snapshot);
            render_tasks(frame, tasks, snapshot, tick);
            render_messages(frame, messages, snapshot);
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
                "  running={} pending={} completed={} failed={} cancelled={} skipped={} · elapsed {elapsed}",
                counts.running,
                counts.pending,
                counts.completed,
                counts.failed,
                counts.cancelled,
                counts.skipped,
            )),
        ]),
        Line::from(format!(
            "replicates={} · phase tasks={} · output={output}",
            snapshot.replicate_count,
            snapshot.tasks.len()
        )),
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
        .take(available)
        .map(|task| task_row(task, tick))
        .collect::<Vec<_>>();
    if snapshot.tasks.len() > available && available > 0 {
        rows.truncate(available.saturating_sub(1));
        rows.push(Row::new([
            Cell::from(format!(
                "… {} more tasks",
                snapshot.tasks.len() - rows.len()
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
    match (snapshot.phase_replicate, snapshot.phase_name.as_deref()) {
        (Some(replicate), Some(phase)) => format!(
            " Tasks · replicate {replicate} · phase {phase} ({}) ",
            snapshot.tasks.len()
        ),
        _ => " Tasks · waiting for phase ".to_owned(),
    }
}

fn task_row(task: &TaskSnapshot, tick: usize) -> Row<'static> {
    let status_style = match task.status {
        TaskStatus::Pending | TaskStatus::Skipped => Style::default().fg(Color::DarkGray),
        TaskStatus::Running => Style::default().fg(Color::Cyan),
        TaskStatus::Completed => Style::default().fg(Color::Green),
        TaskStatus::Failed => Style::default().fg(Color::Red),
        TaskStatus::Cancelled => Style::default().fg(Color::Yellow),
    };
    let progress = progress_text(task, tick);
    let timing = timing_text(task);
    let task_label = if task.detail.is_empty() {
        format!("{} · {}", task.label, display_kind(&task.kind))
    } else {
        format!("{} · {}", task.label, task.detail)
    };
    Row::new([
        Cell::from(task_label).style(Style::default().add_modifier(Modifier::BOLD)),
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

fn timing_text(task: &TaskSnapshot) -> String {
    let Some(started) = task.started else {
        return String::new();
    };
    let elapsed = task
        .finished
        .unwrap_or_else(Instant::now)
        .duration_since(started);
    timing_text_for(elapsed, task.iteration, task.target)
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

fn render_messages(frame: &mut ratatui::Frame<'_>, area: Rect, snapshot: &DashboardSnapshot) {
    let visible = usize::from(area.height.saturating_sub(2));
    let start = snapshot.messages.len().saturating_sub(visible);
    let text = snapshot.messages[start..]
        .iter()
        .map(|message| Line::from(message.clone()))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Messages "))
            .wrap(Wrap { trim: true }),
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
        " Command · cancelling · type exit to close after cleanup "
    } else {
        " Command · type exit then Enter · Ctrl+C cancels "
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
    let mut counts = Counts::default();
    for task in &snapshot.tasks {
        match task.status {
            TaskStatus::Pending => counts.pending += 1,
            TaskStatus::Running => counts.running += 1,
            TaskStatus::Completed => counts.completed += 1,
            TaskStatus::Failed => counts.failed += 1,
            TaskStatus::Cancelled => counts.cancelled += 1,
            TaskStatus::Skipped => counts.skipped += 1,
        }
    }
    counts
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
