//! Deterministic phase-barrier scheduler with bounded preparation.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use super::error::RuntimeError;
use super::phase::{Phase, PhaseFailurePolicy, Task, TaskDisplayKind};
use super::reporting::RuntimeReporter;
use super::task::TaskContext;

pub(crate) fn execute_phase(phase: Phase, reporter: &RuntimeReporter) -> Result<(), RuntimeError> {
    let worker_count = phase.max_concurrent_workloads();
    let queue_capacity = phase.queue_capacity();
    let failure_policy = phase.failure_policy();
    let tasks = phase.into_tasks();
    let (work_sender, work_receiver) = mpsc::sync_channel::<Task>(queue_capacity);
    let work_receiver = Arc::new(Mutex::new(work_receiver));
    let (result_sender, result_receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));

    for task in tasks.iter().filter(|task| task.is_reused()) {
        reporter.mark_reused(task.key())?;
    }

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let receiver = Arc::clone(&work_receiver);
            let results = result_sender.clone();
            let stop = Arc::clone(&stop);
            scope.spawn(move || {
                loop {
                    let task = {
                        let receiver = receiver
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        receiver.recv()
                    };
                    let Ok(task) = task else {
                        break;
                    };
                    let mut task = task;
                    let key = task.key().clone();
                    if stop.load(Ordering::Acquire) {
                        let _ = reporter.mark_skipped(&key);
                        let _ = results.send(Ok(key));
                        continue;
                    }
                    if reporter.is_cancelled() {
                        let _ = reporter.mark_cancelled(&key);
                        let _ = results.send(Err(RuntimeError::Cancelled));
                        continue;
                    }
                    let Some(workload) = task.take_workload() else {
                        let _ = results.send(Err(RuntimeError::MissingTaskWorkload {
                            task: key.to_string(),
                        }));
                        stop.store(true, Ordering::Release);
                        if failure_policy == PhaseFailurePolicy::FailFast {
                            reporter.request_cancellation();
                        }
                        continue;
                    };
                    let context = match task.display_kind() {
                        TaskDisplayKind::Progress => reporter
                            .start_progress(&key, 0, None)
                            .map(|progress| TaskContext::progress(task, progress)),
                        TaskDisplayKind::Activity => reporter
                            .start_activity(&key)
                            .map(|activity| TaskContext::activity(task, activity)),
                    };
                    let context = match context {
                        Ok(context) => context,
                        Err(error) => {
                            let _ = results.send(Err(error));
                            continue;
                        }
                    };
                    if context.is_cancelled() {
                        context.cancel("cancelled before task execution");
                        let _ = results.send(Err(RuntimeError::Cancelled));
                        continue;
                    }
                    match catch_unwind(AssertUnwindSafe(|| workload(&context))) {
                        Ok(Ok(())) => {
                            let result = if context.is_cancelled() {
                                context.cancel("cancelled");
                                Err(RuntimeError::Cancelled)
                            } else {
                                context.complete().map(|()| key)
                            };
                            let _ = results.send(result);
                        }
                        Ok(Err(source)) => {
                            let cancelled = context.is_cancelled();
                            let error = if cancelled {
                                context.cancel(source.to_string());
                                RuntimeError::Cancelled
                            } else {
                                context.fail(source.to_string());
                                stop.store(true, Ordering::Release);
                                if failure_policy == PhaseFailurePolicy::FailFast {
                                    reporter.request_cancellation();
                                }
                                RuntimeError::TaskWorkload {
                                    task: key.to_string(),
                                    source,
                                }
                            };
                            let _ = results.send(Err(error));
                        }
                        Err(_) => {
                            context.fail("task workload panicked");
                            stop.store(true, Ordering::Release);
                            if failure_policy == PhaseFailurePolicy::FailFast {
                                reporter.request_cancellation();
                            }
                            let _ = results.send(Err(RuntimeError::SchedulerPanicked));
                        }
                    }
                }
            });
        }
        drop(result_sender);

        for task in tasks.into_iter().filter(|task| !task.is_reused()) {
            if stop.load(Ordering::Acquire) {
                let _ = reporter.mark_skipped(task.key());
            } else if reporter.is_cancelled() {
                let _ = reporter.mark_cancelled(task.key());
            } else if let Err(error) = work_sender.send(task) {
                let _ = reporter.mark_skipped(error.0.key());
            }
        }
        drop(work_sender);
    });

    let mut first_error = None;
    for result in result_receiver {
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else if reporter.is_cancelled() {
        Err(RuntimeError::Cancelled)
    } else {
        Ok(())
    }
}
