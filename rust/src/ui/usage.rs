//! Best-effort host resource sampling for the interactive dashboard.

use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct UsageSnapshot {
    pub(super) cpu_percent: Option<f64>,
    pub(super) ram_percent: Option<f64>,
    pub(super) disk_percent: Option<f64>,
}

#[derive(Default)]
pub(super) struct UsageMonitor {
    previous_cpu: Option<CpuCounters>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

impl UsageMonitor {
    pub(super) fn sample(&mut self, output: Option<&Path>) -> UsageSnapshot {
        let current_cpu = read_cpu_counters();
        let cpu_percent = current_cpu
            .zip(self.previous_cpu)
            .and_then(|(current, previous)| {
                percentage(
                    current
                        .total
                        .saturating_sub(previous.total)
                        .saturating_sub(current.idle.saturating_sub(previous.idle)),
                    current.total.saturating_sub(previous.total),
                )
            });
        self.previous_cpu = current_cpu;

        UsageSnapshot {
            cpu_percent,
            ram_percent: read_ram_usage(),
            disk_percent: read_disk_usage(output.unwrap_or_else(|| Path::new("."))),
        }
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
