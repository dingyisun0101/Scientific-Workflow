//! Best-effort host resource sampling for the interactive dashboard.

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct UsageSnapshot {
    pub(super) cpu_percent: Option<f64>,
    pub(super) ram_percent: Option<f64>,
    pub(super) disk_percent: Option<f64>,
}

#[derive(Default)]
pub(super) struct UsageMonitor {
    cpu_history: VecDeque<(Instant, CpuCounters)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

impl UsageMonitor {
    pub(super) fn sample(&mut self, output: Option<&Path>) -> UsageSnapshot {
        let cpu_percent = self.sample_cpu(Instant::now(), read_cpu_counters());

        UsageSnapshot {
            cpu_percent,
            ram_percent: read_ram_usage(),
            disk_percent: read_disk_usage(output.unwrap_or_else(|| Path::new("."))),
        }
    }

    fn sample_cpu(&mut self, now: Instant, current: Option<CpuCounters>) -> Option<f64> {
        let Some(current) = current else {
            self.cpu_history.clear();
            return None;
        };
        if self.cpu_history.back().is_some_and(|(_, previous)| {
            current.total < previous.total || current.idle < previous.idle
        }) {
            self.cpu_history.clear();
        }
        self.cpu_history.push_back((now, current));
        let cutoff = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        // Keep the sample bracketing the cutoff so irregular refreshes still
        // cover a full second, and interpolate the counters at the boundary.
        while self.cpu_history.len() > 2 && self.cpu_history[1].0 <= cutoff {
            self.cpu_history.pop_front();
        }
        let &(first_time, first) = self.cpu_history.front()?;
        let (mut total, mut idle) = (first.total as f64, first.idle as f64);
        if first_time < cutoff
            && let Some(&(second_time, second)) = self.cpu_history.get(1)
        {
            let interval = second_time.duration_since(first_time).as_secs_f64();
            if interval > 0.0 {
                let weight = cutoff.duration_since(first_time).as_secs_f64() / interval;
                total += (second.total as f64 - total) * weight;
                idle += (second.idle as f64 - idle) * weight;
            }
        }
        let elapsed = current.total as f64 - total;
        (elapsed > 0.0)
            .then(|| ((elapsed - (current.idle as f64 - idle)) * 100.0 / elapsed).clamp(0.0, 100.0))
    }
}

fn read_cpu_counters() -> Option<CpuCounters> {
    let document = fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_counters(&document)
}

fn parse_cpu_counters(document: &str) -> Option<CpuCounters> {
    let values = document
        .lines()
        .next()?
        .strip_prefix("cpu ")?
        .split_whitespace()
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    let total = values
        .iter()
        .try_fold(0_u64, |sum, value| sum.checked_add(*value))?;
    let idle = values[3].checked_add(values.get(4).copied().unwrap_or(0))?;
    Some(CpuCounters { total, idle })
}

fn read_ram_usage() -> Option<f64> {
    let document = fs::read_to_string("/proc/meminfo").ok()?;
    parse_ram_usage(&document)
}

fn parse_ram_usage(document: &str) -> Option<f64> {
    let mut total = None;
    let mut available = None;
    for line in document.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("MemTotal:") => {
                total = fields.next().and_then(|value| value.parse::<u64>().ok());
            }
            Some("MemAvailable:") => {
                available = fields.next().and_then(|value| value.parse::<u64>().ok());
            }
            _ => {}
        }
    }
    let total = total?;
    percentage(total.saturating_sub(available?), total)
}

fn read_disk_usage(path: &Path) -> Option<f64> {
    crate::runtime::disk_usage(path).ok()
}

fn percentage(occupied: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (occupied as f64 * 100.0 / total as f64).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::{CpuCounters, parse_cpu_counters, parse_ram_usage, percentage};

    #[test]
    fn proc_cpu_fields_exclude_guest_double_counting_and_include_iowait_as_idle() {
        assert_eq!(
            parse_cpu_counters("cpu  10 2 3 40 5 6 7 8 9 10\ncpu0 0"),
            Some(CpuCounters {
                total: 81,
                idle: 45
            })
        );
    }

    #[test]
    fn memory_and_percentage_parsing_report_bounded_occupation() {
        assert_eq!(
            parse_ram_usage("MemTotal: 1000 kB\nMemAvailable: 250 kB\n"),
            Some(75.0)
        );
        assert_eq!(percentage(3, 4), Some(75.0));
        assert_eq!(percentage(1, 0), None);
    }
}

#[cfg(test)]
mod averaging_tests {
    use super::*;

    #[test]
    fn cpu_window_interpolates_cutoff_and_discards_old_samples() {
        let mut monitor = UsageMonitor::default();
        let start = Instant::now();
        assert_eq!(
            monitor.sample_cpu(start, Some(CpuCounters { total: 0, idle: 0 })),
            None
        );
        assert_eq!(
            monitor.sample_cpu(
                start + Duration::from_millis(400),
                Some(CpuCounters {
                    total: 40,
                    idle: 40
                })
            ),
            Some(0.0)
        );
        let average = monitor
            .sample_cpu(
                start + Duration::from_millis(1200),
                Some(CpuCounters {
                    total: 120,
                    idle: 40,
                }),
            )
            .unwrap();
        assert!((average - 80.0).abs() < 1e-10);
        assert_eq!(
            monitor.sample_cpu(
                start + Duration::from_millis(1400),
                Some(CpuCounters {
                    total: 140,
                    idle: 60
                })
            ),
            Some(80.0)
        );
        assert_eq!(monitor.cpu_history.len(), 3);
        assert_eq!(
            monitor.sample_cpu(start + Duration::from_secs(2), None),
            None
        );
        assert!(monitor.cpu_history.is_empty());
        assert_eq!(
            monitor.sample_cpu(
                start + Duration::from_secs(3),
                Some(CpuCounters { total: 5, idle: 2 })
            ),
            None
        );
        assert_eq!(
            monitor.sample_cpu(
                start + Duration::from_secs(4),
                Some(CpuCounters { total: 1, idle: 0 })
            ),
            None
        );
    }
}
