//! Private full-screen terminal presentation for study reporting.
//!
//! This module deliberately knows nothing about phases, task workloads, or
//! scheduling. It consumes immutable display snapshots and emits user actions.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::command::{CommandInput, CommandSubmission, EditAction, StudyCommand};
use super::{ProgressSummary, TaskMode, TaskStatus};

const MESSAGE_HISTORY: usize = 100;
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub(super) struct TaskView {
    pub(super) label: String,
    pub(super) mode: TaskMode,
    pub(super) current: u64,
    pub(super) target: Option<u64>,
    pub(super) started: bool,
    pub(super) status: TaskStatus,
    pub(super) detail: String,
}

/// Immutable state prepared by the renderer for one terminal refresh.
pub(super) struct RenderSnapshot {
    pub(super) heading: String,
    pub(super) summary: ProgressSummary,
    pub(super) tasks: Vec<TaskView>,
}

pub(super) struct TerminalUi {
    terminal: Terminal<CrosstermBackend<io::Stderr>>,
    command: CommandInput,
    messages: VecDeque<String>,
    timing: Vec<TaskTiming>,
    phase_started: Instant,
    tick: usize,
    exit_requested: bool,
}

#[derive(Clone, Copy, Default)]
struct TaskTiming {
    started: Option<(Instant, u64)>,
}

impl TerminalUi {
    pub(super) fn enter(task_count: usize) -> Result<Self, TerminalSetupFailure> {
        enable_raw_mode().map_err(|source| TerminalSetupFailure {
            operation: "enable raw input mode",
            source,
        })?;
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
            return Err(TerminalSetupFailure {
                operation: "enter the isolated terminal screen",
                source,
            });
        }
        let backend = CrosstermBackend::new(io::stderr());
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(source) => {
                restore_terminal();
                return Err(TerminalSetupFailure {
                    operation: "initialize terminal drawing",
                    source,
                });
            }
        };
        Ok(Self {
            terminal,
            command: CommandInput::default(),
            messages: VecDeque::with_capacity(MESSAGE_HISTORY),
            timing: vec![TaskTiming::default(); task_count],
            phase_started: Instant::now(),
            tick: 0,
            exit_requested: false,
        })
    }

    pub(super) fn push_message(&mut self, message: impl Into<String>) {
        if self.messages.len() == MESSAGE_HISTORY {
            self.messages.pop_front();
        }
        self.messages.push_back(message.into());
    }

    pub(super) fn poll_command(&mut self) -> io::Result<Option<StudyCommand>> {
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(Some(StudyCommand::Exit));
                    }
                    if self.exit_requested {
                        continue;
                    }
                    let Some(action) = edit_action(key.code, key.modifiers) else {
                        continue;
                    };
                    match self.command.edit(action) {
                        Some(CommandSubmission::Parsed(command)) => return Ok(Some(command)),
                        Some(CommandSubmission::Unknown(command)) => {
                            self.push_message(format!("unknown command: {command}"));
                        }
                        Some(CommandSubmission::Empty) | None => {}
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn mark_exit_requested(&mut self) {
        if !self.exit_requested {
            self.exit_requested = true;
            self.command.clear();
            self.push_message("study: exit requested; waiting for active tasks");
        }
    }

    pub(super) fn draw(&mut self, snapshot: &RenderSnapshot) -> io::Result<()> {
        if self.timing.len() != snapshot.tasks.len() {
            self.timing
                .resize(snapshot.tasks.len(), TaskTiming::default());
        }
        let now = Instant::now();
        let rows = snapshot
            .tasks
            .iter()
            .zip(&mut self.timing)
            .map(|(task, timing)| task_row(task, timing, now, self.tick))
            .collect::<Vec<_>>();
        self.tick = self.tick.wrapping_add(1);
        let view = UiView {
            heading: snapshot.heading.clone(),
            summary: format_summary(&snapshot.summary, self.phase_started.elapsed()),
            rows,
            messages: self.messages.iter().cloned().collect(),
            command: self.command.text(),
            command_cursor: self.command.cursor(),
            exit_requested: self.exit_requested,
        };
        self.terminal.draw(|frame| render(frame, &view))?;
        Ok(())
    }
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

impl Drop for TerminalUi {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        restore_terminal();
    }
}

pub(super) struct TerminalSetupFailure {
    pub(super) operation: &'static str,
    pub(super) source: io::Error,
}

fn restore_terminal() {
    let mut stderr = io::stderr();
    let _ = execute!(stderr, DisableMouseCapture, Show, LeaveAlternateScreen);
    let _ = stderr.flush();
    let _ = disable_raw_mode();
}

struct UiView {
    heading: String,
    summary: String,
    rows: Vec<TaskRow>,
    messages: Vec<String>,
    command: String,
    command_cursor: usize,
    exit_requested: bool,
}

struct TaskRow {
    label: String,
    status: TaskStatus,
    detail: String,
    progress: TaskProgressView,
    elapsed: String,
    eta: String,
}

enum TaskProgressView {
    Idle,
    OneShot,
    Known {
        current: u64,
        target: u64,
        ratio: f64,
    },
    Unknown {
        spinner: char,
        current: u64,
    },
}

fn task_row(task: &TaskView, timing: &mut TaskTiming, now: Instant, tick: usize) -> TaskRow {
    if task.started && timing.started.is_none() {
        timing.started = Some((now, task.current));
    }
    let (elapsed, initial) = timing
        .started
        .map_or((Duration::ZERO, task.current), |(started, initial)| {
            (now.saturating_duration_since(started), initial)
        });
    let progress = match (task.started, task.mode, task.target) {
        (false, _, _) => TaskProgressView::Idle,
        (true, TaskMode::OneShot, _) => TaskProgressView::OneShot,
        (true, TaskMode::Progress, Some(target)) => TaskProgressView::Known {
            current: task.current,
            target,
            ratio: if target == 0 {
                1.0
            } else {
                (task.current as f64 / target as f64).clamp(0.0, 1.0)
            },
        },
        (true, TaskMode::Progress, None) => TaskProgressView::Unknown {
            spinner: SPINNER[tick % SPINNER.len()],
            current: task.current,
        },
    };
    let eta = match task.target {
        Some(target) if task.status == TaskStatus::Running && task.current > initial => {
            let completed = task.current - initial;
            let remaining = target.saturating_sub(task.current);
            let seconds = elapsed.as_secs_f64() * remaining as f64 / completed as f64;
            format_duration(Duration::from_secs_f64(seconds))
        }
        Some(_) if task.status == TaskStatus::Completed => format_duration(Duration::ZERO),
        _ => "unknown".to_owned(),
    };
    TaskRow {
        label: task.label.clone(),
        status: task.status,
        detail: if task.detail == task.status.as_str() {
            String::new()
        } else {
            task.detail.clone()
        },
        progress,
        elapsed: format_duration(elapsed),
        eta,
    }
}

fn render(frame: &mut Frame<'_>, view: &UiView) {
    let [header_area, tasks_area, messages_area, command_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    let header = Block::default().borders(Borders::ALL).title(" Study ");
    let header_inner = header.inner(header_area);
    frame.render_widget(header, header_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(view.heading.as_str()),
            Line::from(Span::styled(
                &view.summary,
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ])
        .wrap(Wrap { trim: true }),
        header_inner,
    );

    render_tasks(frame, tasks_area, &view.rows);

    let messages = Block::default().borders(Borders::ALL).title(" Messages ");
    let messages_inner = messages.inner(messages_area);
    frame.render_widget(messages, messages_area);
    let visible_messages = usize::from(messages_inner.height);
    let first = view.messages.len().saturating_sub(visible_messages);
    frame.render_widget(
        Paragraph::new(
            view.messages[first..]
                .iter()
                .map(|message| Line::from(message.as_str()))
                .collect::<Vec<_>>(),
        ),
        messages_inner,
    );

    let command_title = if view.exit_requested {
        " Command — exiting "
    } else {
        " Command — type exit to stop "
    };
    let command_block = Block::default().borders(Borders::ALL).title(command_title);
    let command_inner = command_block.inner(command_area);
    frame.render_widget(command_block, command_area);
    let prompt = if view.exit_requested {
        Line::from(Span::styled(
            "> exit requested; waiting for active tasks",
            Style::default().fg(Color::Yellow),
        ))
    } else {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(&view.command),
        ])
    };
    frame.render_widget(Paragraph::new(prompt), command_inner);
    if !view.exit_requested && command_inner.width > 0 && command_inner.height > 0 {
        let x = command_inner
            .x
            .saturating_add(2)
            .saturating_add(u16::try_from(view.command_cursor).unwrap_or(u16::MAX))
            .min(command_inner.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(x, command_inner.y));
    }
}

fn render_tasks(frame: &mut Frame<'_>, area: Rect, rows: &[TaskRow]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Tasks ({}) ", rows.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let visible = usize::from(inner.height);
    let show_overflow = rows.len() > visible;
    let row_limit = if show_overflow {
        visible.saturating_sub(1)
    } else {
        visible
    };
    for (offset, row) in rows.iter().take(row_limit).enumerate() {
        let row_area = Rect::new(
            inner.x,
            inner.y + u16::try_from(offset).unwrap_or(0),
            inner.width,
            1,
        );
        render_task_row(frame, row_area, row);
    }
    if show_overflow && visible > 0 {
        let hidden = rows.len() - row_limit;
        let overflow_area = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(format!("… {hidden} more tasks not visible"))
                .style(Style::default().fg(Color::DarkGray)),
            overflow_area,
        );
    }
}

fn render_task_row(frame: &mut Frame<'_>, area: Rect, row: &TaskRow) {
    let [
        label_area,
        status_area,
        progress_area,
        count_area,
        timing_area,
    ] = Layout::horizontal([
        Constraint::Percentage(28),
        Constraint::Percentage(18),
        Constraint::Fill(1),
        Constraint::Length(15),
        Constraint::Length(27),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(row.label.as_str()).style(Style::default().add_modifier(Modifier::BOLD)),
        label_area,
    );
    let status = if row.detail.is_empty() {
        row.status.as_str().to_owned()
    } else {
        format!("{}: {}", row.status.as_str(), row.detail)
    };
    frame.render_widget(
        Paragraph::new(status).style(status_style(row.status)),
        status_area,
    );
    match row.progress {
        TaskProgressView::Idle => {}
        TaskProgressView::OneShot => {}
        TaskProgressView::Known {
            current,
            target,
            ratio,
        } => {
            frame.render_widget(
                LineGauge::default()
                    .ratio(ratio)
                    .label("")
                    .filled_symbol("█")
                    .unfilled_symbol("░")
                    .filled_style(Style::default().fg(Color::Cyan))
                    .unfilled_style(Style::default().fg(Color::Blue)),
                progress_area,
            );
            frame.render_widget(Paragraph::new(format!(" {current}/{target}")), count_area);
        }
        TaskProgressView::Unknown { spinner, current } => {
            frame.render_widget(
                Paragraph::new(format!("{spinner} iteration {current}"))
                    .style(Style::default().fg(Color::Cyan)),
                progress_area,
            );
        }
    }
    let timing = match row.progress {
        TaskProgressView::Idle => String::new(),
        TaskProgressView::OneShot => format!("elapsed {}", row.elapsed),
        _ => format!("elapsed {} ETA {}", row.elapsed, row.eta),
    };
    frame.render_widget(Paragraph::new(timing), timing_area);
}

fn status_style(status: TaskStatus) -> Style {
    match status {
        TaskStatus::Pending | TaskStatus::Skipped => Style::default().fg(Color::DarkGray),
        TaskStatus::Running => Style::default().fg(Color::Cyan),
        TaskStatus::Completed => Style::default().fg(Color::Green),
        TaskStatus::Failed => Style::default().fg(Color::Red),
        TaskStatus::Cancelled => Style::default().fg(Color::Yellow),
    }
}

fn format_summary(summary: &ProgressSummary, elapsed: Duration) -> String {
    format!(
        "running={} pending={} completed={} failed={} cancelled={} skipped={} · elapsed {}",
        summary.running(),
        summary.pending(),
        summary.completed(),
        summary.failed(),
        summary.cancelled(),
        summary.skipped(),
        format_duration(elapsed),
    )
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn task_progress_keeps_counts_elapsed_and_eta_fields() {
        let now = Instant::now();
        let mut timing = TaskTiming {
            started: Some((now - Duration::from_secs(10), 20)),
        };
        let row = task_row(
            &TaskView {
                label: "simulation".to_owned(),
                mode: TaskMode::Progress,
                current: 60,
                target: Some(100),
                started: true,
                status: TaskStatus::Running,
                detail: "evolving".to_owned(),
            },
            &mut timing,
            now,
            0,
        );
        assert!(matches!(
            row.progress,
            TaskProgressView::Known { current: 60, target: 100, ratio } if ratio == 0.6
        ));
        assert_eq!(row.elapsed, "00:00:10");
        assert_eq!(row.eta, "00:00:10");
    }

    #[test]
    fn renderer_keeps_sections_command_and_progress_bar_at_fixed_sizes() {
        let view = UiView {
            heading: "Phase 1 of 1 — [2] simulation".to_owned(),
            summary: "running=1 pending=0 completed=0 completed=0 failed=0 cancelled=0 skipped=0 · elapsed 00:00:10".to_owned(),
            rows: vec![TaskRow {
                label: "simulation {seed=7}".to_owned(),
                status: TaskStatus::Running,
                detail: "evolving".to_owned(),
                progress: TaskProgressView::Known {
                    current: 60,
                    target: 100,
                    ratio: 0.6,
                },
                elapsed: "00:00:10".to_owned(),
                eta: "00:00:10".to_owned(),
            }],
            messages: vec!["simulation: checkpoint committed".to_owned()],
            command: "ex".to_owned(),
            command_cursor: 2,
            exit_requested: false,
        };
        for (width, height) in [(120, 20), (50, 10)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| render(frame, &view)).unwrap();
            let cells = terminal.backend().buffer().content();
            let text = cells.iter().map(|cell| cell.symbol()).collect::<String>();
            assert!(text.contains("Study"));
            assert!(text.contains("Tasks"));
            assert!(text.contains("Messages"));
            assert!(text.contains("Command"));
            assert!(text.contains("60/100"));
            if width >= 100 {
                assert!(text.contains('█'));
                assert!(text.contains('░'));
            }
        }
    }
}
