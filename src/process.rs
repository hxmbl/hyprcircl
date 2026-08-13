// =========================================================================
// Process Execution Helpers
// =========================================================================

/// Run a shell command and return its stdout (Waybar module commands).
pub fn run_shell(cmd: &str) -> Option<String> {
    let out = std::process::Command::new("sh").args(["-c", cmd]).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

/// Terminate a stream child and its whole process group. Stream commands are
/// spawned under `setsid`, so the child PID is its process-group leader;
/// killing the negative PID reaps grandchildren too (e.g. `nc`, `pactl`).
/// The `--` is required so `-<pid>` is parsed as a group PID, not a signal.
pub fn kill_process_group(child: &mut std::process::Child) {
    let pid = format!("-{}", child.id());
    let _ = std::process::Command::new("kill").args(["-9", "--", &pid]).status();
    let _ = child.wait();
}
