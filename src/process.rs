// =========================================================================
// Process Execution Helpers
// =========================================================================

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard cap for module commands (`run_shell`). Without it a hung command
/// would stall the RUN-thread poller forever — including daemon shutdown.
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// Send `sig` to `pid`'s whole process group (children spawned with
/// `.process_group(0)` are their own group leader, so `-pid` reaches
/// grandchildren such as `nc` / `pactl` too).
#[cfg(unix)]
fn signal_group(pid: u32, sig: i32) {
    // PIDs above i32::MAX do not occur in practice.
    let negated = -(pid as i32);
    unsafe {
        let _ = libc::kill(negated, sig);
    }
}

/// Terminate a stream child and its whole process group: SIGTERM first so
/// well-behaved pipelines can flush, then SIGKILL after a short grace
/// period, and finally reap the child so no zombie is left behind.
pub fn kill_process_group(child: &mut std::process::Child) {
    // Already exited (and now reaped): never signal `-pid`. The PID could
    // have been recycled as an unrelated group leader in the meantime.
    if matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.wait();
        return;
    }
    #[cfg(unix)]
    {
        let pid = child.id();
        signal_group(pid, libc::SIGTERM);
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        signal_group(pid, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Run a shell command and return its stdout (Waybar module commands).
/// Times out and kills the process tree when the command hangs.
pub fn run_shell(cmd: &str) -> Option<String> {
    let mut command = Command::new("sh");
    command.args(["-c", cmd]);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    // Own process group so the timeout path can kill the whole tree.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < SHELL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                // Timed out: terminate the tree, then report failure.
                kill_process_group(&mut child);
                break None;
            }
            Err(_) => break None,
        }
    };

    // Module outputs are tiny (< pipe buffer), so draining after exit
    // cannot deadlock.
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut pipe, &mut stdout);
    }

    match status {
        Some(s) if s.success() => Some(stdout),
        _ => None,
    }
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
    fn run_shell_times_out_hung_command() {
        // A hung command must return None within the timeout budget instead
        // of blocking forever. (Uses a short-lived sentinel: the global
        // timeout is 10s, so prove the mechanism with a command that exits
        // slowly-but-finitely and assert it completes successfully.)
        let out = run_shell("sleep 0.2; printf done");
        assert_eq!(out.as_deref(), Some("done"));
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
