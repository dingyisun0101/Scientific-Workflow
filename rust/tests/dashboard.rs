//! Exercise the public facade in a real terminal, including command input.
#[cfg(unix)]
#[test]
fn dashboard_child() {
    let Some(project) = std::env::var_os("WORKFLOW_TERMINAL_PROBE") else {
        return;
    };
    let result = scientific_workflow::run(std::path::Path::new(&project));
    if std::env::var_os("WORKFLOW_EXPECT_CANCEL").is_some() {
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    } else {
        result.unwrap();
    }
}

#[cfg(unix)]
#[test]
fn dashboard_commands_disk_reminders_and_terminal_restoration() {
    let result = std::process::Command::new("python3")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/terminal_probe.py"))
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
