use std::path::{Path, PathBuf};
use std::process::Command;

use scientific_workflow::prelude::study::TaskContext;

use crate::AppResult;

pub(crate) fn render_trajectories(
    script: &Path,
    recording_directories: &[PathBuf],
    output_directory: &Path,
    context: &TaskContext,
) -> AppResult<()> {
    context.set_detail("starting Python trajectory renderer");
    let mut command = Command::new("mamba");
    command.args(["run", "-n", "DSES", "python"]).arg(script);
    for recording in recording_directories {
        command.arg("--recording").arg(recording);
    }
    let output = command
        .arg("--output-directory")
        .arg(output_directory)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("trajectory renderer failed: {stderr}").into());
    }

    let message = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !message.is_empty() {
        context.report(message)?;
    }
    context.set_detail("trajectory images rendered");
    Ok(())
}
