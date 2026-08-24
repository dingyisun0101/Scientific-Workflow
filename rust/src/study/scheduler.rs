//! Deterministic phase-barrier scheduler with bounded preparation.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use super::error::StudyError;
use super::phase::{Phase, PhaseFailurePolicy, Task, TaskKey, TaskMode};
use super::record::StudyRecorder;
use super::renderer::StudyRenderer;
use super::task::TaskContext;
use super::timing::{ExpirationWatch, StartGate, TimingFailures};

struct ScheduledTask {
    rank: usize,
    task: Task,
}

pub(crate) fn execute_phase(
    phase: Phase,
    renderer: &StudyRenderer,
    execution: &StudyRecorder,
) -> Result<(), StudyError> {
    let worker_count = phase.max_active_tasks();
    let prepared_task_queue_capacity = phase.prepared_task_queue_capacity();
    let failure_policy = phase.failure_policy();
    let delay_per_task = phase.delay_per_task();
    let task_timeout = phase.task_timeout();
    let deadline_after = phase.deadline_after();
    let phase_id = phase.id().get();
    let tasks = phase.into_tasks();
    let (work_sender, work_receiver) =
        mpsc::sync_channel::<ScheduledTask>(prepared_task_queue_capacity);
    let work_receiver = Arc::new(Mutex::new(work_receiver));
    let (result_sender, result_receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let cancellation = renderer.cancellation_flag();
    let start_gate = Arc::new(StartGate::new(delay_per_task));
    let timing_failures = Arc::new(TimingFailures::new());
    let deadline_watch = deadline_after.map(|deadline_after| {
        ExpirationWatch::phase(
            deadline_after,
            phase_id,
            Arc::clone(&cancellation),
            Arc::clone(&timing_failures),
        )
    });
    for task in tasks.iter().filter(|task| task.is_completed()) {
        renderer.mark_completed(task.key())?;
    }
    if delay_per_task.is_some() {
        for (rank, task) in tasks
            .iter()
            .filter(|task| !task.is_completed())
            .enumerate()
            .skip(1)
        {
            renderer.mark_delayed(task.key(), rank)?;
        }
    }
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let receiver = Arc::clone(&work_receiver);
            let results = result_sender.clone();
            let stop = Arc::clone(&stop);
            let cancellation = Arc::clone(&cancellation);
            let start_gate = Arc::clone(&start_gate);
            let timing_failures = Arc::clone(&timing_failures);
            scope.spawn(move || {
                loop {
                    let scheduled = {
                        let receiver = receiver
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        receiver.recv()
                    };
                    let Ok(scheduled) = scheduled else {
                        break;
                    };
                    let ScheduledTask { rank, mut task } = scheduled;
                    let key = task.key().clone();
                    if stop.load(Ordering::Acquire) {
                        mark_task_skipped(renderer, &results, &key);
                        continue;
                    }
                    if renderer.is_cancelled() {
                        mark_task_cancelled(renderer, &results, &key);
                        continue;
                    }
                    let Some(start_permit) = start_gate.wait_for_turn(rank, &cancellation, &stop)
                    else {
                        if renderer.is_cancelled() {
                            mark_task_cancelled(renderer, &results, &key);
                        } else {
                            mark_task_skipped(renderer, &results, &key);
                        }
                        continue;
                    };
                    if stop.load(Ordering::Acquire) {
                        mark_task_skipped(renderer, &results, &key);
                        continue;
                    }
                    if renderer.is_cancelled() {
                        mark_task_cancelled(renderer, &results, &key);
                        continue;
                    }
                    let Some(workload) = task.take_workload() else {
                        signal_task_failure(
                            renderer,
                            &results,
                            &stop,
                            failure_policy,
                            StudyError::MissingTaskWorkload {
                                task: key.to_string(),
                            },
                        );
                        continue;
                    };
                    let context = match task.mode() {
                        TaskMode::Progress => renderer
                            .start_progress(&key, 0, None)
                            .map(|progress| TaskContext::progress(task, progress)),
                        TaskMode::OneShot => renderer
                            .start_one_shot(&key)
                            .map(|one_shot| TaskContext::one_shot(task, one_shot)),
                    };
                    let context = match context {
                        Ok(context) => context,
                        Err(error) => {
                            signal_task_failure(renderer, &results, &stop, failure_policy, error);
                            continue;
                        }
                    };
                    start_permit.started();
                    let _execution_timer = match execution.task_started(&key) {
                        Ok(timer) => timer,
                        Err(error) => {
                            context.fail(error.to_string());
                            signal_task_failure(renderer, &results, &stop, failure_policy, error);
                            continue;
                        }
                    };
                    if context.is_cancelled() {
                        context.cancel("cancelled before task execution");
                        let _ = report_task_outcome(&results, Err(StudyError::Cancelled));
                        continue;
                    }
                    let timeout_watch = task_timeout.map(|timeout| {
                        ExpirationWatch::task(
                            timeout,
                            key.to_string(),
                            Arc::clone(&cancellation),
                            Arc::clone(&timing_failures),
                        )
                    });
                    let outcome = catch_unwind(AssertUnwindSafe(|| workload(&context)));
                    let timed_out = timeout_watch.is_some_and(ExpirationWatch::finish);
                    if timed_out {
                        context.cancel("task timeout exceeded");
                        signal_task_failure(
                            renderer,
                            &results,
                            &stop,
                            failure_policy,
                            StudyError::Cancelled,
                        );
                        continue;
                    }
                    match outcome {
                        Ok(Ok(())) => {
                            let result = if context.is_cancelled() {
                                context.cancel("cancelled");
                                Err(StudyError::Cancelled)
                            } else {
                                context.complete().map(|()| key)
                            };
                            let _ = match result {
                                Ok(key) => report_task_outcome(&results, Ok(key)),
                                Err(error) => report_task_outcome(&results, Err(error)),
                            };
                        }
                        Ok(Err(source)) => {
                            if context.is_cancelled() {
                                context.cancel(source.to_string());
                                let _ = report_task_outcome(&results, Err(StudyError::Cancelled));
                            } else {
                                context.fail(source.to_string());
                                signal_task_failure(
                                    renderer,
                                    &results,
                                    &stop,
                                    failure_policy,
                                    StudyError::TaskWorkload {
                                        task: key.to_string(),
                                        source,
                                    },
                                );
                            }
                        }
                        Err(_) => {
                            context.fail("task workload panicked");
                            signal_task_failure(
                                renderer,
                                &results,
                                &stop,
                                failure_policy,
                                StudyError::SchedulerPanicked,
                            );
                        }
                    }
                }
            });
        }
        drop(result_sender);

        for (rank, task) in tasks
            .into_iter()
            .filter(|task| !task.is_completed())
            .enumerate()
        {
            if stop.load(Ordering::Acquire) {
                let _ = renderer.mark_skipped(task.key());
            } else if renderer.is_cancelled() {
                let _ = renderer.mark_cancelled(task.key());
            } else if let Err(error) = work_sender.send(ScheduledTask { rank, task }) {
                let _ = renderer.mark_skipped(error.0.task.key());
            }
        }
        drop(work_sender);
    });
    if let Some(watch) = deadline_watch {
        watch.finish();
    }

    let mut first_error = None;
    for result in result_receiver {
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = timing_failures.take() {
        Err(error)
    } else if let Some(error) = first_error {
        Err(error)
    } else if renderer.is_cancelled() {
        Err(StudyError::Cancelled)
    } else {
        Ok(())
    }
}

fn mark_task_skipped(
    renderer: &StudyRenderer,
    results: &mpsc::Sender<Result<TaskKey, StudyError>>,
    key: &TaskKey,
) {
    let _ = renderer.mark_skipped(key);
    let _ = report_task_outcome(results, Ok(key.clone()));
}

fn mark_task_cancelled(
    renderer: &StudyRenderer,
    results: &mpsc::Sender<Result<TaskKey, StudyError>>,
    key: &TaskKey,
) {
    let _ = renderer.mark_cancelled(key);
    let _ = report_task_outcome(results, Err(StudyError::Cancelled));
}

fn signal_task_failure(
    renderer: &StudyRenderer,
    results: &mpsc::Sender<Result<TaskKey, StudyError>>,
    stop: &AtomicBool,
    failure_policy: PhaseFailurePolicy,
    error: StudyError,
) {
    stop.store(true, Ordering::Release);
    if failure_policy == PhaseFailurePolicy::FailFast {
        renderer.request_cancellation();
    }
    let _ = report_task_outcome(results, Err(error));
}

fn report_task_outcome(
    results: &mpsc::Sender<Result<TaskKey, StudyError>>,
    outcome: Result<TaskKey, StudyError>,
) -> Result<(), mpsc::SendError<Result<TaskKey, StudyError>>> {
    results.send(outcome)
}
