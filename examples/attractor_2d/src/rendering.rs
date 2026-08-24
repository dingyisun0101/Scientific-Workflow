use std::path::Path;
use std::process::Command;

use scientific_workflow::prelude::study::TaskContext;

use crate::AppResult;

pub(crate) fn render_trajectories(
    script: &Path,
    recording_directory: &Path,
    output_directory: &Path,
    context: &TaskContext,
) -> AppResult<()> {
    context.set_detail("starting Python trajectory renderer");
    let output = Command::new("mamba")
        .args(["run", "-n", "DSES", "python"])
        .arg(script)
        .arg("--recording-directory")
        .arg(recording_directory)
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
