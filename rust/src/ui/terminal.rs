//! Required interactive Ratatui dashboard.

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
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use unicode_width::UnicodeWidthChar;

use super::command::{CommandInput, CommandSubmission, EditAction, UiCommand};
use super::state::{DashboardSnapshot, TaskSnapshot, TaskStatus};
use super::usage::{UsageMonitor, UsageSnapshot};

static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const TIMING_WIDTH: u16 = 19;

pub(super) fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub(super) struct DashboardTerminal {
    terminal: Terminal<CrosstermBackend<io::Stderr>>,
    command: CommandInput,
    tick: usize,
    lease: TerminalLease,
    task_offset: usize,
    task_anchor: Option<(u64, String)>,
    usage: UsageMonitor,
    redraw_requested: bool,
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
            usage: UsageMonitor::default(),
            redraw_requested: true,
        })
    }

    pub(super) fn poll_command(&mut self) -> io::Result<Option<CommandSubmission>> {
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if key.code == KeyCode::PageDown {
                        self.redraw_requested = true;
                        let size = self.terminal.size()?;
                        let capacity = page_capacity(
                            dashboard_areas(Rect::new(0, 0, size.width, size.height))[1],
                        );
                        self.task_offset = self.task_offset.saturating_add(capacity);
                        self.task_anchor = None;
                        continue;
                    }
                    if key.code == KeyCode::PageUp {
                        self.redraw_requested = true;
                        let size = self.terminal.size()?;
                        let capacity = page_capacity(
                            dashboard_areas(Rect::new(0, 0, size.width, size.height))[1],
                        );
                        self.task_offset = self.task_offset.saturating_sub(capacity);
                        self.task_anchor = None;
                        continue;
                    }
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        self.redraw_requested = true;
                        return Ok(Some(CommandSubmission::Parsed(UiCommand::Interrupt)));
                    }
                    if let Some(action) = edit_action(key.code, key.modifiers) {
                        self.redraw_requested = true;
                        if let Some(submission) = self.command.edit(action) {
                            return Ok(Some(submission));
                        }
                    }
                }
                Event::Resize(_, _) => self.redraw_requested = true,
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn clear_command(&mut self) {
        self.command.clear();
        self.redraw_requested = true;
    }

    pub(super) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw_requested)
    }

    pub(super) fn draw(&mut self, snapshot: &DashboardSnapshot) -> io::Result<()> {
        if let Some(anchor) = &self.task_anchor {
            self.task_offset = snapshot
                .tasks
                .iter()
                .position(|task| (task.replicate, &task.identity) == (anchor.0, &anchor.1))
                .unwrap_or(0);
        }
        let size = self.terminal.size()?;
        let capacity = page_capacity(dashboard_areas(Rect::new(0, 0, size.width, size.height))[1]);
        self.task_offset = page_offset(self.task_offset, snapshot.tasks.len(), capacity);
        self.task_anchor = snapshot
            .tasks
            .get(self.task_offset)
            .map(|t| (t.replicate, t.identity.clone()));
        let task_offset = self.task_offset;
        let usage = self.usage.sample(snapshot.output.as_deref());
        let tick = self.tick;
        let command = self.command.text();
        let command_cursor = self.command.cursor();
        self.tick = self.tick.wrapping_add(1);
        self.terminal.draw(|frame| {
            let area = frame.area();
            let [header, tasks, messages, usage_area, command_area] = dashboard_areas(area);
            render_header(frame, header, snapshot);
            render_tasks(frame, tasks, snapshot, tick, task_offset);
            render_messages(frame, messages, snapshot);
            render_usage(
                frame,
                usage_area,
                usage,
                snapshot.active_threads,
                snapshot.thread_budget,
            );
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

fn dashboard_areas(area: Rect) -> [Rect; 5] {
    Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(4),
        Constraint::Length(if area.height >= 24 { 7 } else { 5 }),
        Constraint::Length(3),
        Constraint::Length(3),
    ])
    .areas(area)
}

fn page_capacity(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(3)).max(1)
}
fn page_offset(offset: usize, count: usize, capacity: usize) -> usize {
    (offset / capacity).min(count.saturating_sub(1) / capacity) * capacity
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
    let [title_area, study_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new("SCIENTIFIC WORKFLOW")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        title_area,
    );
    let statuses = [
        ("running", counts.running, Color::Blue),
        ("pending", counts.pending, Color::Yellow),
        ("completed", counts.completed, Color::Green),
        ("failed", counts.failed, Color::Red),
        ("cancelled", counts.cancelled, Color::Magenta),
        ("skipped", counts.skipped, Color::Gray),
    ];
    let text = vec![
        Line::from(format!(
            "Total time {elapsed} · {}",
            if snapshot.execution_finished {
                "finished"
            } else {
                snapshot.control_status
            }
        )),
        Line::from(
            statuses
                .into_iter()
                .map(|(name, count, color)| {
                    Span::styled(format!("{name}={count} "), Style::default().fg(color))
                })
                .collect::<Vec<_>>(),
        ),
        Line::from(format!(
            "replicates={}/{} · visible active tasks={}",
            snapshot.current_replicate,
            snapshot.replicate_count,
            snapshot.tasks.len()
        )),
        Line::from(format!("output={output}")),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Study ")),
        study_area,
    );
}

fn render_tasks(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    tick: usize,
    offset: usize,
) {
    let header = Row::new(["task", "threads", "status", "progress", "elapsed / ETA"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let available = usize::from(area.height.saturating_sub(3));
    let widths = task_columns(area.width, &snapshot.tasks);
    let rows = snapshot
        .tasks
        .iter()
        .skip(offset)
        .take(available)
        .map(|task| task_row(task, tick, snapshot.now, usize::from(widths[3])))
        .collect::<Vec<_>>();
    let table = Table::new(rows, widths.map(Constraint::Length))
        .header(header)
        .column_spacing(1)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " Phase={}/{}: {} Tasks · PgUp/PgDn ",
            snapshot.current_phase,
            snapshot.phase_count,
            snapshot.tasks.len()
        )));
    frame.render_widget(table, area);
}

fn task_columns(width: u16, tasks: &[TaskSnapshot]) -> [u16; 5] {
    // Reserve borders and four separators, then protect the complete counter.
    let available = width.saturating_sub(6);
    let required = tasks
        .iter()
        .filter(|task| task.kind == "execution_unit")
        .map(|task| progress_count(task).len())
        .max()
        .unwrap_or(0);
    let progress = u16::try_from(
        (usize::from(available) * 32 / 100)
            .max(required)
            .min(usize::from(available)),
    )
    .expect("progress width is bounded by the terminal area");
    let remaining = available - progress;
    let threads = remaining.min(7);
    let remaining = remaining - threads;
    let status = remaining.min(12);
    let remaining = remaining - status;
    let timing = if remaining >= TIMING_WIDTH + 8 {
        TIMING_WIDTH
    } else {
        0
    };
    [remaining - timing, threads, status, progress, timing]
}

fn progress_count(task: &TaskSnapshot) -> String {
    match task.target {
        Some(target) => format!("{}/{target}", task.iteration),
        None => task.iteration.to_string(),
    }
}

fn task_row(task: &TaskSnapshot, tick: usize, now: Instant, progress_width: usize) -> Row<'static> {
    let status_style = match task.status {
        TaskStatus::Pending | TaskStatus::Skipped => Style::default().fg(Color::DarkGray),
        TaskStatus::Running => Style::default().fg(Color::Blue),
        TaskStatus::Completed => Style::default().fg(Color::Green),
        TaskStatus::Failed => Style::default().fg(Color::Red),
        TaskStatus::Cancelled => Style::default().fg(Color::Yellow),
    };
    let progress = progress_text(task, tick, progress_width);
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
        Cell::from(if task.status == TaskStatus::Running {
            task.threads.to_string()
        } else {
            "0".into()
        }),
        Cell::from(task.status.label()).style(status_style),
        Cell::from(progress).style(Style::default().fg(Color::Blue)),
        Cell::from(timing),
    ])
}

fn display_kind(kind: &str) -> &str {
    match kind {
        "execution_unit" => "unit",
        other => other,
    }
}

fn progress_text(task: &TaskSnapshot, tick: usize, width: usize) -> String {
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
    let count = progress_count(task);
    if count.len() > width {
        // A visibly omitted value is safer than a clipped, incorrect integer.
        return ".".repeat(width.min(3));
    }
    let bar_width = width.saturating_sub(count.len() + 1).min(16);
    match task.target {
        Some(target) if bar_width > 0 => {
            let ratio = if target == 0 {
                1.0
            } else {
                (task.iteration as f64 / target as f64).clamp(0.0, 1.0)
            };
            let filled = (ratio * bar_width as f64).round() as usize;
            format!(
                "{count} {}{}",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(bar_width - filled),
            )
        }
        None if bar_width > 0 => format!("{count} {}", SPINNER[tick % SPINNER.len()]),
        _ => count,
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

fn render_messages(frame: &mut ratatui::Frame<'_>, area: Rect, snapshot: &DashboardSnapshot) {
    let width = usize::from(area.width.saturating_sub(2)).max(1);
    let visible = usize::from(area.height.saturating_sub(2));
    let mut lines = Vec::new();
    for message in &snapshot.messages {
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
                .title(" Messages · recent · full history in log.txt "),
        ),
        area,
    );
}

fn render_usage(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    usage: UsageSnapshot,
    threads: usize,
    budget: usize,
) {
    let value = |percent: Option<f64>| {
        percent.map_or_else(|| "--".to_owned(), |percent| format!("{percent:5.1}%"))
    };
    let line = Line::from(vec![
        Span::styled("THREADS ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{threads}/{budget}   ")),
        Span::styled("CPU ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(value(usage.cpu_percent)),
        Span::raw("   "),
        Span::styled("RAM ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(value(usage.ram_percent)),
        Span::raw("   "),
        Span::styled("DISK ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(value(usage.disk_percent)),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(" Usage ")),
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

    #[test]
    fn usage_panel_labels_all_three_percentages() {
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(60, 3)).unwrap();
        terminal
            .draw(|frame| {
                render_usage(
                    frame,
                    frame.area(),
                    UsageSnapshot {
                        cpu_percent: Some(12.5),
                        ram_percent: Some(50.0),
                        disk_percent: Some(75.25),
                    },
                    4,
                    8,
                );
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Usage"));
        assert!(text.contains("THREADS 4/8"));
        assert!(text.contains("CPU  12.5%"));
        assert!(text.contains("RAM  50.0%"));
        assert!(text.contains("DISK  75.2%"));
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
            .draw(|frame| render_messages(frame, frame.area(), &snapshot))
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

#[cfg(test)]
mod progress_tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn pages_use_the_rendered_capacity_and_clamp_after_resize() {
        let area = Rect::new(0, 0, 120, 8);
        assert_eq!(page_capacity(area), 5);
        assert_eq!(page_offset(usize::MAX, 12, 5), 10);
        assert_eq!(page_offset(10, 12, 8), 8);
        assert_eq!(page_offset(10, 0, 5), 0);
        let mut snapshot = super::super::state::DashboardState::new().snapshot();
        snapshot.tasks = (0..12)
            .map(|index| {
                let mut value = task(1, 2);
                value.label = format!("item-{index:02}");
                value
            })
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        for offset in [0, 5, 10] {
            terminal
                .draw(|frame| render_tasks(frame, frame.area(), &snapshot, 0, offset))
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            for index in 0..12 {
                assert_eq!(
                    rendered.contains(&format!("item-{index:02}")),
                    (offset..(offset + 5).min(12)).contains(&index)
                );
            }
        }
    }

    fn task(iteration: u64, target: u64) -> TaskSnapshot {
        TaskSnapshot {
            threads: 2,
            identity: "ensemble".into(),
            program_progress: None,
            replicate: 0,
            phase: "evolve".into(),
            label: "ensemble".into(),
            kind: "execution_unit".into(),
            status: TaskStatus::Running,
            iteration,
            target: Some(target),
            started: Some(Instant::now()),
            finished: None,
            detail: String::new(),
        }
    }

    #[test]
    fn counters_survive_narrow_tables_without_losing_digits() {
        for width in [35, 80, 99, 100, 173] {
            for (iteration, target) in [(100, 36_000), (1104, 432_000)] {
                let mut snapshot = super::super::state::DashboardState::new().snapshot();
                snapshot.tasks = vec![task(iteration, target)];
                let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
                terminal
                    .draw(|frame| render_tasks(frame, frame.area(), &snapshot, 0, 0))
                    .unwrap();
                let row: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .skip(usize::from(width) * 2)
                    .take(usize::from(width))
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(
                    row.contains(&format!("{iteration}/{target}")),
                    "width={width}: {row}"
                );
            }
        }
    }

    #[test]
    fn very_narrow_cells_omit_instead_of_misrepresenting_counters() {
        let task = task(u64::MAX, u64::MAX);
        assert_eq!(progress_text(&task, 0, 8), "...");
        assert_eq!(progress_text(&task, 0, 2), "..");
        assert_eq!(progress_text(&task, 0, 0), "");
        assert!(progress_text(&task, 0, 60).starts_with(&progress_count(&task)));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    #[test]
    fn page_title_study_rows_and_task_title_have_agreed_colors_and_positions() {
        let mut snapshot = super::super::state::DashboardState::new().snapshot();
        snapshot.current_replicate = 2;
        snapshot.replicate_count = 5;
        snapshot.current_phase = 1;
        snapshot.phase_count = 3;
        let mut terminal = Terminal::new(TestBackend::new(140, 12)).unwrap();
        terminal
            .draw(|frame| {
                render_header(frame, Rect::new(0, 0, 140, 7), &snapshot);
                render_tasks(frame, Rect::new(0, 7, 140, 5), &snapshot, 0, 0);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("SCIENTIFIC WORKFLOW"));
        assert!(text.contains("Total time"));
        assert!(text.contains("replicates=2/5"));
        assert!(text.contains("Phase=1/3: 0 Tasks · PgUp/PgDn"));
        let header: String = buffer
            .content
            .iter()
            .take(140)
            .map(|cell| cell.symbol())
            .collect();
        let title_start = header.find("SCIENTIFIC WORKFLOW").unwrap();
        let right_padding = 140 - title_start - "SCIENTIFIC WORKFLOW".len();
        assert!(title_start.abs_diff(right_padding) <= 1);
        assert_eq!(
            buffer[(u16::try_from(title_start).unwrap(), 0)].symbol(),
            "S"
        );
        assert_eq!(
            buffer[(u16::try_from(title_start).unwrap(), 0)].fg,
            Color::Cyan
        );
        assert!(
            buffer[(u16::try_from(title_start).unwrap(), 0)]
                .modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(buffer[(1, 3)].fg, Color::Blue);
        assert!(
            buffer
                .content
                .iter()
                .enumerate()
                .all(|(index, cell)| cell.fg != Color::Cyan || index < 140)
        );
    }
}
