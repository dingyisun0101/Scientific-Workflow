//! Linux-owned process trees and concurrent, durable program output draining.
//!
//! Future macOS/Windows support must qualify process groups/Job Objects and
//! directory durability; compiling a fallback is not a support promise.
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::presentation::TaskPresentation;
use crate::persistence::ProgramPersistenceSession;
use crate::task::TaskResult;

const FRAME_LIMIT: usize = 16 * 1024;
const PREFIX: &[u8] = b"@workflow ";
static PROCESS_GROUPS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

// Keep the registry locked across launch and force-exit to prevent late children.
fn spawn(command: &mut Command) -> std::io::Result<ProcessTree> {
    let mut groups = PROCESS_GROUPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let child = command.spawn()?;
    groups.push(child.id());
    Ok(ProcessTree(child, false))
}

#[cfg(feature = "terminal-ui")]
pub(crate) fn force_exit() -> ! {
    let groups = PROCESS_GROUPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(unix)]
    for &pid in groups.iter() {
        // SAFETY: only groups created and retained by this runtime are registered.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(unix)]
    for &pid in groups.iter() {
        // SAFETY: these are our direct children; another supervisor may have reaped them.
        unsafe {
            libc::waitpid(pid as i32, std::ptr::null_mut(), 0);
        }
    }
    std::process::exit(130)
}

pub(super) fn execute(
    mut command: Command,
    mut persistence: ProgramPersistenceSession,
    cancellation: &AtomicBool,
    presentation: &TaskPresentation,
    cooperative: bool,
) -> TaskResult {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let control = presentation.control();
    let control_path = persistence
        .dependencies_path()
        .with_file_name("workflow-control.json");
    if cooperative {
        write_control(&control_path, control.paused(), false)?;
        command.env("WORKFLOW_CONTROL_PATH", &control_path);
        for variable in [
            "OMP_NUM_THREADS",
            "OPENBLAS_NUM_THREADS",
            "MKL_NUM_THREADS",
            "NUMEXPR_NUM_THREADS",
            "VECLIB_MAXIMUM_THREADS",
        ] {
            command.env(variable, "1");
        }
    } else {
        command.env_remove("WORKFLOW_CONTROL_PATH");
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match spawn(&mut command) {
        Ok(child) => child,
        Err(error) => {
            persistence.fail(None, &error.to_string());
            return Err(error.into());
        }
    };
    let failure = Arc::new(Mutex::new(None));
    let stdout = drain(
        child.0.stdout.take().expect("piped stdout"),
        persistence.take_stdout(),
        false,
        presentation.clone(),
        Arc::clone(&failure),
    );
    let stderr = drain(
        child.0.stderr.take().expect("piped stderr"),
        persistence.take_stderr(),
        true,
        presentation.clone(),
        Arc::clone(&failure),
    );
    let outcome = (|| -> TaskResult<Option<std::process::ExitStatus>> {
        let mut last_pause = control.paused();
        let mut parked = None;
        loop {
            if cancellation.load(Ordering::Acquire) || control.cancelled() {
                if cooperative {
                    let _ = write_control(&control_path, false, true);
                }
                child.stop();
                return Ok(None);
            }
            if let Some(reason) = failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                return Err(std::io::Error::other(reason).into());
            }
            if cooperative {
                let paused = control.paused();
                if paused != last_pause {
                    write_control(&control_path, paused, false)?;
                    last_pause = paused;
                }
                let acknowledged = control_path.with_extension("json.parent.paused").is_file();
                if paused && acknowledged {
                    parked.get_or_insert_with(|| control.parked());
                } else {
                    parked = None;
                }
            }
            if let Some(status) = child.0.try_wait()? {
                return Ok(Some(status));
            }
            thread::sleep(Duration::from_millis(5));
        }
    })();
    // Stop remaining descendants before joining pipe readers and releasing permits.
    child.stop();
    let stdout_result = stdout.join();
    let stderr_result = stderr.join();
    if stdout_result.is_err() || stderr_result.is_err() {
        persistence.fail(None, "program pipe reader panicked");
        return Err(std::io::Error::other("program pipe reader panicked").into());
    }
    if let Some(reason) = failure
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        persistence.fail(None, &reason);
        return Err(std::io::Error::other(reason).into());
    }
    match outcome {
        Ok(Some(status)) if status.success() => {
            persistence.complete(status.code()).map_err(Into::into)
        }
        Ok(Some(status)) => {
            let reason = format!(
                "program exited with {status}; logs beside {}",
                persistence.dependencies_path().display()
            );
            persistence.fail(status.code(), &reason);
            Err(std::io::Error::other(reason).into())
        }
        Ok(None) => {
            persistence.fail(None, "runtime cancellation requested");
            Ok(())
        }
        Err(error) => {
            persistence.fail(None, &error.to_string());
            Err(error)
        }
    }
}

struct ProcessTree(Child, bool);
impl ProcessTree {
    fn stop(&mut self) {
        if self.1 {
            return;
        }
        self.1 = true;
        #[cfg(unix)]
        // SAFETY: process_group(0) creates an exclusively owned group led by this child.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_millis(150);
        while self.0.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        #[cfg(unix)]
        // SAFETY: cleanup targets only the group created for this task.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
        PROCESS_GROUPS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|id| *id != self.0.id());
    }
}
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_control(path: &Path, paused: bool, cancelled: bool) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(
        &temporary,
        format!("{{\"paused\":{paused},\"cancelled\":{cancelled}}}"),
    )?;
    std::fs::rename(temporary, path)
}

fn drain(
    mut pipe: impl Read + Send + 'static,
    mut log: File,
    telemetry: bool,
    presentation: TaskPresentation,
    failure: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut line = Vec::new();
        let mut oversized = false;
        let mut warnings = 0;
        let mut writable = true;
        let mut last_progress = None;
        let mut last_publish = Instant::now() - Duration::from_secs(1);
        loop {
            let count = match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    *failure
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(format!("required output pipe read failed: {error}"));
                    break;
                }
            };
            if writable && let Err(error) = log.write_all(&buffer[..count]) {
                *failure
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(format!("required program log write failed: {error}"));
                writable = false; // Continue draining until the supervisor stops the group.
            }
            if !telemetry {
                continue;
            }
            for &byte in &buffer[..count] {
                if byte == b'\n' {
                    if line.starts_with(PREFIX) {
                        if oversized {
                            warn(&presentation, &mut warnings);
                        } else {
                            match parse(&line[PREFIX.len()..]) {
                                Some(Event::Log { level, message }) => {
                                    presentation.log(&level, &message)
                                }
                                Some(event @ Event::Progress { .. }) => {
                                    last_progress = Some(event);
                                    if last_publish.elapsed() >= Duration::from_millis(50) {
                                        publish_progress(
                                            &presentation,
                                            last_progress.take().unwrap(),
                                        );
                                        last_publish = Instant::now();
                                    }
                                }
                                None => warn(&presentation, &mut warnings),
                            }
                        }
                    }
                    line.clear();
                    oversized = false;
                } else if line.len() < FRAME_LIMIT {
                    line.push(byte);
                } else {
                    oversized = true;
                }
            }
        }
        if line.starts_with(PREFIX) {
            warn(&presentation, &mut warnings);
        }
        if let Some(event) = last_progress {
            publish_progress(&presentation, event);
        }
        if writable && let Err(error) = log.flush().and_then(|()| log.sync_all()) {
            *failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(format!("required program log flush failed: {error}"));
        }
    })
}
fn warn(presentation: &TaskPresentation, warnings: &mut usize) {
    if *warnings < 3 {
        presentation.log("warning", "malformed, oversized, or unsupported Workflow event; original bytes retained in stderr.log");
        *warnings += 1;
    }
}
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(2048)
        .collect()
}
enum Event {
    Log {
        level: String,
        message: String,
    },
    Progress {
        stage: String,
        completed: u64,
        total: Option<u64>,
        unit: String,
    },
}
fn parse(bytes: &[u8]) -> Option<Event> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value.get("version")?.as_u64()? != 1 {
        return None;
    }
    match value.get("kind")?.as_str()? {
        "log" => {
            let level = value.get("level")?.as_str()?;
            if !["debug", "info", "warning", "error", "success"].contains(&level) {
                return None;
            }
            Some(Event::Log {
                level: level.into(),
                message: clean(value.get("message")?.as_str()?),
            })
        }
        "progress" => {
            let completed = value.get("completed")?.as_u64()?;
            let total = match value.get("total") {
                Some(serde_json::Value::Null) | None => None,
                Some(v) => Some(v.as_u64()?),
            };
            if total.is_some_and(|n| completed > n) {
                return None;
            }
            let stage = clean(value.get("stage")?.as_str()?);
            let unit = clean(value.get("unit")?.as_str()?);
            if stage.trim().is_empty() || unit.trim().is_empty() {
                return None;
            }
            Some(Event::Progress {
                stage,
                completed,
                total,
                unit,
            })
        }
        _ => None,
    }
}
fn publish_progress(p: &TaskPresentation, event: Event) {
    if let Event::Progress {
        stage,
        completed,
        total,
        unit,
    } = event
    {
        p.program_progress(&stage, completed, total, &unit);
    }
}

pub(super) fn check_prerequisites(study: &crate::study::Study) -> Result<(), super::RuntimeError> {
    let interpreters = study
        .phases()
        .iter()
        .flat_map(|p| p.tasks())
        .filter(|t| t.is_npy())
        .filter_map(|t| t.program_path())
        .collect::<std::collections::BTreeSet<_>>();
    for interpreter in interpreters {
        let probe = "import sys; assert sys.version_info >= (3,14), 'Python 3.14+ required'; import scientific_workflow, numpy, threadpoolctl; assert scientific_workflow.__version__ == '0.4.3', 'install scientific-workflow[npy] 0.4.3'; from scientific_workflow.npy import convert_workflow_dependencies";
        let output = Command::new(interpreter)
            .args(["-c", probe])
            .output()
            .map_err(|source| super::RuntimeError::PythonPrerequisite {
                interpreter: interpreter.to_path_buf(),
                reason: source.to_string(),
            })?;
        if !output.status.success() {
            return Err(super::RuntimeError::PythonPrerequisite {
                interpreter: interpreter.to_path_buf(),
                reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_reject_bad_versions_bounds_and_control_characters() {
        assert!(parse(br#"{"version":2,"kind":"log","level":"info","message":"x"}"#).is_none());
        assert!(parse(br#"{"version":1,"kind":"progress","stage":"x","completed":2,"total":1,"unit":"records"}"#).is_none());
        assert!(
            parse(br#"{"version":1,"kind":"progress","stage":"","completed":0,"unit":"records"}"#)
                .is_none()
        );
        match parse(br#"{"version":1,"kind":"log","level":"error","message":"\u001b[31mx"}"#)
            .unwrap()
        {
            Event::Log { message, .. } => assert!(!message.contains('\u{1b}')),
            _ => panic!("expected log"),
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn required_log_failure_is_reported_while_the_pipe_is_drained() {
        struct Observer;
        impl super::super::RuntimeObserver for Observer {
            fn publish(
                &self,
                _: super::super::RuntimeEvent<'_>,
            ) -> Result<(), super::super::PresentationFailure> {
                Ok(())
            }
            fn cancellation_requested(&self) -> Result<bool, super::super::PresentationFailure> {
                Ok(false)
            }
            fn finish(&self) -> Result<(), super::super::PresentationFailure> {
                Ok(())
            }
        }
        let presentation = super::super::presentation::RuntimePresentation::new(Observer);
        let failure = Arc::new(Mutex::new(None));
        let pipe = std::io::Cursor::new(vec![b'x'; 100_000]);
        let log = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        drain(
            pipe,
            log,
            false,
            presentation.task(0, "test"),
            failure.clone(),
        )
        .join()
        .unwrap();
        assert!(
            failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("log write failed")
        );
    }
}
