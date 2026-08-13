//! Deterministic phase-barrier scheduler with bounded preparation.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use super::error::RuntimeError;
use super::phase::{Phase, Task, TaskDisplayKind};
use super::reporting::RuntimeReporter;
use super::task::TaskContext;

pub(crate) fn execute_phase(phase: &Phase, reporter: &RuntimeReporter) -> Result<(), RuntimeError> {
    let (work_sender, work_receiver) = mpsc::sync_channel::<Task>(phase.queue_capacity());
    let work_receiver = Arc::new(Mutex::new(work_receiver));
    let (result_sender, result_receiver) = mpsc::channel();
    let mut submitted = 0_usize;

    for task in phase.tasks().iter().filter(|task| task.is_reused()) {
        reporter.mark_reused(task.key())?;
    }

    thread::scope(|scope| {
        for _ in 0..phase.max_concurrent_workloads() {
            let receiver = Arc::clone(&work_receiver);
            let results = result_sender.clone();
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
                    let key = task.key().clone();
                    let context = match task.display_kind() {
                        TaskDisplayKind::Progress => reporter
                            .start_progress(&key, 0, None)
                            .map(|progress| TaskContext::progress(task.clone(), progress)),
                        TaskDisplayKind::Activity => reporter
                            .start_activity(&key)
                            .map(|activity| TaskContext::activity(task.clone(), activity)),
                    };
                    let context = match context {
                        Ok(context) => context,
                        Err(error) => {
                            let _ = results.send(Err(error));
                            continue;
                        }
                    };
                    let Some(workload) = task.executable().cloned() else {
                        context.fail("missing workload");
                        let _ = results.send(Err(RuntimeError::MissingTaskWorkload {
                            task: key.to_string(),
                        }));
                        continue;
                    };
                    match catch_unwind(AssertUnwindSafe(|| workload(&context))) {
                        Ok(Ok(())) => {
                            let result = context.complete().map(|()| key);
                            let _ = results.send(result);
                        }
                        Ok(Err(source)) => {
                            context.fail(source.to_string());
                            let _ = results.send(Err(RuntimeError::TaskWorkload {
                                task: key.to_string(),
                                source,
                            }));
                        }
                        Err(_) => {
                            context.fail("task workload panicked");
                            let _ = results.send(Err(RuntimeError::SchedulerPanicked));
                        }
                    }
                }
            });
        }
        drop(result_sender);

        for task in phase.tasks().iter().filter(|task| !task.is_reused()) {
            submitted += 1;
            if work_sender.send(task.clone()).is_err() {
                break;
            }
        }
        drop(work_sender);
    });

    let mut first_error = None;
    for result in result_receiver.into_iter().take(submitted) {
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}
