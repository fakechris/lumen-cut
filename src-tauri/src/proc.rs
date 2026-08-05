//! Subprocess plumbing. Centralised so the sidecar pattern is uniform across
//! ffmpeg / yt-dlp / faster-whisper / pyannote.

use std::future::Future;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command as TokioCommand;

use crate::error::{AppError, AppResult};

const STDERR_TAIL_BYTES: usize = 256 * 1024;
static ACTIVE_PROCESS_GROUPS: OnceLock<Mutex<std::collections::HashMap<u32, ProcessGroup>>> =
    OnceLock::new();

/// Windows `CREATE_NO_WINDOW`. Console-subsystem tools (ffmpeg, yt-dlp,
/// python) otherwise flash a console window for every managed job.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Windows counterpart of a Unix process group: a job object that owns the
/// child and every descendant it spawns, so one call tears down ffmpeg's
/// helper processes too. `KILL_ON_JOB_CLOSE` makes the teardown survive a
/// forgotten explicit terminate.
#[cfg(windows)]
mod job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job object handle is a kernel handle with no thread affinity.
    // Every use below is a single Win32 call that the kernel serialises.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// Create an anonymous kill-on-close job and put `process` in it.
        /// Returns `None` when either step fails, in which case the caller
        /// falls back to killing the direct child only.
        pub fn containing(process: HANDLE) -> Option<Self> {
            // SAFETY: null attributes and name request the documented
            // anonymous-job default.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
                // SAFETY: the struct is a plain-old-data limit descriptor.
                unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` outlives the call and its size is exact.
            let configured = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                return None;
            }
            // SAFETY: both handles are owned and open for the duration.
            if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
                return None;
            }
            Some(job)
        }

        pub fn terminate(&self) {
            // SAFETY: the handle is owned and still open.
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // Closing the last handle kills whatever is still in the job.
            // SAFETY: the handle is owned and closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// The teardown handle for one managed child. Unix identifies a process group
/// by pid; Windows owns a job object instead.
#[derive(Default)]
struct ProcessGroup {
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl ProcessGroup {
    /// Ask the child and everything it spawned to stop. Best effort: a
    /// process that already exited is not an error here.
    fn terminate(&self, pid: u32) {
        #[cfg(unix)]
        // Every managed child starts a fresh process group whose id equals
        // its pid. A negative pid targets that complete group.
        self.signal(pid, libc::SIGTERM);
        #[cfg(windows)]
        {
            let _ = pid;
            // Job termination is unconditional on Windows; there is no
            // graceful-then-forceful pair to escalate through.
            if let Some(job) = &self.job {
                job.terminate();
            }
        }
        #[cfg(not(any(unix, windows)))]
        let _ = pid;
    }

    /// Escalation for a group that ignored [`ProcessGroup::terminate`].
    fn kill(&self, pid: u32) {
        #[cfg(unix)]
        self.signal(pid, libc::SIGKILL);
        #[cfg(not(unix))]
        self.terminate(pid);
    }

    #[cfg(unix)]
    fn signal(&self, pid: u32, signal: i32) {
        // SAFETY: `kill` is async-signal-safe and a stale pid only returns
        // ESRCH, which this teardown path deliberately ignores.
        unsafe {
            libc::kill(-(pid as i32), signal);
        }
    }
}

tokio::task_local! {
    static CANCEL_FLAG: Arc<AtomicBool>;
}

/// Run a future so every subprocess it spawns observes the same cancellation
/// flag. This keeps ffmpeg, yt-dlp and Python ASR cancellation consistent
/// without a process-global kill switch.
pub async fn with_cancellation<F>(flag: Arc<AtomicBool>, future: F) -> F::Output
where
    F: Future,
{
    CANCEL_FLAG.scope(flag, future).await
}

pub fn cancellation_requested() -> bool {
    CANCEL_FLAG
        .try_with(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn active_process_groups() -> &'static Mutex<std::collections::HashMap<u32, ProcessGroup>> {
    ACTIVE_PROCESS_GROUPS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

struct ProcessGroupRegistration(Option<u32>);

impl ProcessGroupRegistration {
    fn new(child: &tokio::process::Child) -> Self {
        let Some(pid) = child.id() else {
            return Self(None);
        };
        #[cfg(windows)]
        let group = ProcessGroup {
            // `process_group` has no Windows equivalent, so containment is
            // established after the spawn. The window where the child could
            // start a descendant first is a few microseconds wide.
            job: child
                .raw_handle()
                .and_then(|handle| job::Job::containing(handle as _)),
        };
        #[cfg(not(windows))]
        let group = ProcessGroup::default();
        active_process_groups()
            .lock()
            .expect("active subprocess state poisoned")
            .insert(pid, group);
        Self(Some(pid))
    }
}

impl Drop for ProcessGroupRegistration {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            active_process_groups()
                .lock()
                .expect("active subprocess state poisoned")
                .remove(&pid);
        }
    }
}

/// Stop every managed subprocess group while the desktop event loop is still
/// alive. `kill_on_drop` covers normal future cancellation; this registry also
/// covers application quit, when Tauri exits the process without unwinding all
/// in-flight async tasks.
pub fn terminate_all_processes() {
    let groups = active_process_groups()
        .lock()
        .expect("active subprocess state poisoned")
        .drain()
        .collect::<Vec<_>>();
    for (pid, group) in groups {
        group.terminate(pid);
    }
}

/// Async variant — runs the subprocess to completion, capturing stdout, and
/// annotates failures with `sidecar` and the stderr tail. Used by code paths
/// that already live inside a tokio runtime.
pub async fn run(bin: &str, args: &[&str]) -> AppResult<String> {
    run_with_env(bin, args, &[]).await
}

pub type ProgressLineCallback = Arc<dyn Fn(String) + Send + Sync>;

/// Run a cancellable subprocess while streaming complete stderr lines to the
/// caller. Stdout remains reserved for the sidecar's machine-readable result.
pub async fn run_with_progress(
    bin: &str,
    args: &[&str],
    on_stderr_line: ProgressLineCallback,
) -> AppResult<String> {
    run_with_env_and_progress(bin, args, &[], Some(on_stderr_line)).await
}

/// Run a cancellable subprocess with a small, explicit environment overlay.
/// This avoids process-global mutation when a local project stores credentials
/// needed by one sidecar.
pub async fn run_with_env(
    bin: &str,
    args: &[&str],
    environment: &[(&str, &str)],
) -> AppResult<String> {
    run_with_env_and_progress(bin, args, environment, None).await
}

/// Run a cancellable subprocess with an explicit environment overlay while
/// streaming complete stderr lines to the caller.
pub async fn run_with_env_progress(
    bin: &str,
    args: &[&str],
    environment: &[(&str, &str)],
    on_stderr_line: ProgressLineCallback,
) -> AppResult<String> {
    run_with_env_and_progress(bin, args, environment, Some(on_stderr_line)).await
}

async fn run_with_env_and_progress(
    bin: &str,
    args: &[&str],
    environment: &[(&str, &str)],
    on_stderr_line: Option<ProgressLineCallback>,
) -> AppResult<String> {
    let process = label(bin);
    let started = std::time::Instant::now();
    tracing::info!(process, "subprocess started");
    let mut command = TokioCommand::new(bin);
    command
        .args(args)
        .envs(environment.iter().copied())
        // Managed jobs are non-interactive. Inheriting a terminal lets tools
        // such as ffmpeg stop themselves with SIGTTIN when they probe stdin
        // from their isolated process group, leaving the UI waiting forever.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    // Console-subsystem sidecars would otherwise flash a window per job.
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|e| AppError::Sidecar {
        sidecar: label(bin),
        message: format!("spawn: {e}"),
    })?;
    let _process_group = ProcessGroupRegistration::new(&child);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Schema("subprocess stdout was not captured".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Schema("subprocess stderr was not captured".into()))?;
    let stdout_task = tokio::spawn(async move {
        let mut reader = stdout;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut bytes = Vec::new();
        loop {
            let mut line = Vec::new();
            let count = reader.read_until(b'\n', &mut line).await?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&line);
            if bytes.len() > STDERR_TAIL_BYTES * 2 {
                let excess = bytes.len() - STDERR_TAIL_BYTES;
                bytes.drain(..excess);
            }
            if let Some(callback) = &on_stderr_line {
                callback(
                    String::from_utf8_lossy(&line)
                        .trim_end_matches(['\r', '\n'])
                        .to_string(),
                );
            }
        }
        if bytes.len() > STDERR_TAIL_BYTES {
            let excess = bytes.len() - STDERR_TAIL_BYTES;
            bytes.drain(..excess);
        }
        Ok::<Vec<u8>, std::io::Error>(bytes)
    });

    let mut was_cancelled = false;
    let status = loop {
        if cancellation_requested() {
            was_cancelled = true;
            break terminate_process_tree(&mut child).await?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    };
    let out = stdout_task
        .await
        .map_err(|error| AppError::Schema(format!("stdout reader failed: {error}")))??;
    let err = stderr_task
        .await
        .map_err(|error| AppError::Schema(format!("stderr reader failed: {error}")))??;

    if was_cancelled {
        tracing::info!(
            process,
            elapsed_ms = started.elapsed().as_millis(),
            "subprocess cancelled"
        );
        return Err(AppError::Cancelled);
    }
    if !status.success() {
        let tail = String::from_utf8_lossy(&err);
        tracing::warn!(
            process,
            exit_code = status.code().unwrap_or(-1),
            elapsed_ms = started.elapsed().as_millis(),
            "subprocess failed"
        );
        return Err(AppError::Sidecar {
            sidecar: label(bin),
            message: format!("exit {}: {}", status.code().unwrap_or(-1), tail.trim()),
        });
    }

    tracing::info!(
        process,
        elapsed_ms = started.elapsed().as_millis(),
        "subprocess completed"
    );
    Ok(String::from_utf8(out).unwrap_or_default())
}

async fn terminate_process_tree(
    child: &mut tokio::process::Child,
) -> std::io::Result<std::process::ExitStatus> {
    if let Some(pid) = child.id() {
        // Ask the whole tree to stop first: ffmpeg and yt-dlp both spawn
        // helpers that keep the output file locked if only the direct child
        // dies. On Unix that is a SIGTERM to the process group; on Windows it
        // is `TerminateJobObject` on the job the child was placed in.
        let signal = |escalate: bool| {
            if let Some(group) = active_process_groups()
                .lock()
                .expect("active subprocess state poisoned")
                .get(&pid)
            {
                if escalate {
                    group.kill(pid);
                } else {
                    group.terminate(pid);
                }
            }
        };
        signal(false);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                signal(true);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    }
    let _ = child.kill().await;
    child.wait().await
}

/// Convert a binary name to the canonical sidecar label we surface to users.
fn label(bin: &str) -> &'static str {
    match bin {
        "ffmpeg" => "ffmpeg",
        "yt-dlp" => "yt-dlp",
        b if b.ends_with("lumen_cut_asr") => "lumen_cut_asr",
        b if b.ends_with("lumen_cut_diarize") => "lumen_cut_diarize",
        _ => "sidecar",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interpreter for the small scripts these tests run. `sh` and PowerShell
    /// are the two shells that ship with their platform and behave
    /// predictably under redirected stdio.
    fn shell() -> (&'static str, &'static [&'static str]) {
        if cfg!(windows) {
            (
                "powershell.exe",
                &["-NoProfile", "-NonInteractive", "-Command"],
            )
        } else {
            ("/bin/sh", &["-c"])
        }
    }

    /// Run `script` in the platform shell. `unix` and `windows` express the
    /// same behaviour, so each test still asserts one contract.
    async fn run_script(unix: &str, windows: &str) -> AppResult<String> {
        run_script_with_env(unix, windows, &[]).await
    }

    async fn run_script_with_env(
        unix: &str,
        windows: &str,
        environment: &[(&str, &str)],
    ) -> AppResult<String> {
        let (bin, prefix) = shell();
        let mut args = prefix.to_vec();
        args.push(if cfg!(windows) { windows } else { unix });
        run_with_env(bin, &args, environment).await
    }

    #[test]
    fn process_termination_does_not_spawn_synchronous_commands() {
        let source = include_str!("proc.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        let forbidden = ["std::process::", "Command"].concat();
        assert!(
            !source.contains(&forbidden),
            "cancellation must not block the async subprocess runner"
        );
        assert!(
            production.contains("kill_on_drop(true)"),
            "child processes must not survive an app shutdown"
        );
        assert!(
            production.contains("terminate_all_processes"),
            "the desktop shutdown path must be able to terminate every active process group"
        );
    }

    #[tokio::test]
    async fn run_captures_stdout() {
        let out = run_script("printf hello", "[Console]::Out.Write('hello')")
            .await
            .unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[tokio::test]
    async fn managed_processes_receive_eof_instead_of_terminal_input() {
        let out = run_script(
            "if read -r _value; then printf input; else printf eof; fi",
            "if ($null -eq [Console]::In.ReadLine()) { [Console]::Out.Write('eof') } \
             else { [Console]::Out.Write('input') }",
        )
        .await
        .unwrap();
        assert_eq!(out.trim(), "eof");
    }

    #[tokio::test]
    async fn run_surfaces_failure() {
        let err = run_script("exit 1", "exit 1").await.unwrap_err();
        match err {
            AppError::Sidecar { message, .. } => assert!(message.contains("exit")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn failure_output_keeps_a_bounded_tail() {
        let err = run_script(
            "yes x | head -c 1048576 >&2; printf 'TAIL_MARKER' >&2; exit 1",
            "[Console]::Error.Write('x' * 1048576); \
             [Console]::Error.Write('TAIL_MARKER'); exit 1",
        )
        .await
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.len() < 300_000,
            "stderr was not bounded: {}",
            message.len()
        );
        assert!(message.ends_with("TAIL_MARKER"));
    }

    #[tokio::test]
    async fn run_with_env_passes_an_explicit_value() {
        let out = run_script_with_env(
            "printf '%s' \"$LUMEN_CUT_PROC_TEST\"",
            "[Console]::Out.Write($env:LUMEN_CUT_PROC_TEST)",
            &[("LUMEN_CUT_PROC_TEST", "scoped")],
        )
        .await
        .unwrap();
        assert_eq!(out.trim(), "scoped");
    }

    #[tokio::test]
    async fn run_with_progress_streams_stderr_lines() {
        let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = lines.clone();
        let (bin, prefix) = shell();
        let mut args = prefix.to_vec();
        args.push(if cfg!(windows) {
            "[Console]::Error.WriteLine('LUMEN_CUT_PROGRESS {\"progress\":64}'); \
             [Console]::Out.Write('done')"
        } else {
            "printf 'LUMEN_CUT_PROGRESS {\"progress\":64}\n' >&2; printf done"
        });
        let out = run_with_progress(
            bin,
            &args,
            Arc::new(move |line| captured.lock().unwrap().push(line)),
        )
        .await
        .unwrap();
        assert_eq!(out.trim(), "done");
        assert_eq!(
            lines.lock().unwrap().as_slice(),
            ["LUMEN_CUT_PROGRESS {\"progress\":64}"]
        );
    }

    #[tokio::test]
    async fn scoped_cancellation_stops_a_running_process() {
        let flag = Arc::new(AtomicBool::new(false));
        let trigger = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            trigger.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let error = with_cancellation(flag, run_script("sleep 10", "Start-Sleep -Seconds 10"))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Cancelled));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
