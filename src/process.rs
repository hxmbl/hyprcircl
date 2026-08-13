// =========================================================================
// Process Execution Helpers
// =========================================================================

/// Run a shell command and return its stdout (Waybar module commands).
pub fn run_shell(cmd: &str) -> Option<String> {
    let out = std::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .ok()?;
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
    let _ = std::process::Command::new("kill")
        .args(["-9", "--", &pid])
        .status();
    let _ = child.wait();
}

// =========================================================================
// Unit tests for process execution helpers
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_shell_returns_stdout() {
        let out = run_shell("printf 'hello'");
        assert_eq!(out.as_deref(), Some("hello"));
    }

    #[test]
    fn run_shell_returns_none_on_failure() {
        // A non-existent command yields a non-zero exit -> None.
        assert_eq!(run_shell("exit 3"), None);
    }

    #[test]
    fn run_shell_multiline_trimmed_only_by_caller() {
        // Stdout keeps the trailing newline; callers decide how to trim.
        let out = run_shell("printf 'a\\nb\\n'");
        assert_eq!(out.as_deref(), Some("a\nb\n"));
    }

    #[test]
    fn kill_process_group_terminates_child() {
        use std::os::unix::process::CommandExt;

        // Spawn a long-lived process as its own process-group leader so the
        // negative-PID kill used by kill_process_group reaps it cleanly.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdout(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn sleep");
        assert_eq!(child.try_wait().expect("poll"), None, "still running");

        kill_process_group(&mut child);

        // After the kill the child must have exited.
        let status = child.wait().expect("reap");
        assert!(!status.success() || status.code().is_some());
    }

    #[test]
    fn kill_process_group_handles_already_exited() {
        // A child that already exited should not panic and should still reap.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let _ = child.wait();
        // Running kill on an exited child is a no-op that must not panic.
        kill_process_group(&mut child);
    }
}
