//! Finding and running ffmpeg / ffprobe: no console window on Windows, cancellable, with the
//! `-progress` stream parsed into a fraction.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A shared stop flag for a running job.
pub type Cancel = Arc<AtomicBool>;

pub fn new_cancel() -> Cancel {
    Arc::new(AtomicBool::new(false))
}

fn exe_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// A `Command` that opens no console window on Windows.
pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

fn works(path: &Path, version_flag: &str) -> bool {
    command(path)
        .arg(version_flag)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Looks for `tool` ("ffmpeg" or "ffprobe"): the configured path (or its folder), next to the
/// program, in an `ffmpeg` / `ffmpeg\bin` folder beside it, on PATH, then in usual install folders.
pub fn find_tool(tool: &str, configured: &str) -> Option<PathBuf> {
    let name = exe_name(tool);
    let mut candidates: Vec<PathBuf> = Vec::new();
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        if p.is_dir() {
            candidates.push(p.join(&name));
            candidates.push(p.join("bin").join(&name));
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(&exe_name("ffmpeg")))
        {
            // a path to ffmpeg also serves for ffprobe, which sits beside it
            if let Some(dir) = p.parent() {
                candidates.push(dir.join(&name));
            }
        } else {
            candidates.push(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for sub in ["", "ffmpeg", "ffmpeg/bin", "bin"] {
                candidates.push(dir.join(sub).join(&name));
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(&name));
        }
    }
    if cfg!(windows) {
        for base in [
            "C:/ffmpeg/bin",
            "C:/Program Files/ffmpeg/bin",
            "C:/Program Files (x86)/ffmpeg/bin",
        ] {
            candidates.push(Path::new(base).join(&name));
        }
    } else {
        for base in ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"] {
            candidates.push(Path::new(base).join(&name));
        }
    }
    let flag = if tool == "ffprobe" || tool == "ffmpeg" {
        "-version"
    } else {
        "--version"
    };
    candidates
        .into_iter()
        .find(|p| p.is_file() && works(p, flag))
}

/// Result of a finished process.
#[derive(Debug)]
pub struct Outcome {
    pub success: bool,
    pub cancelled: bool,
    /// The last lines ffmpeg wrote to stderr (the reason of a failure).
    pub stderr_tail: String,
}

/// Parses ffmpeg `-progress` lines (`out_time_us=…`, `out_time_ms=…`) into seconds.
pub fn parse_progress_seconds(line: &str) -> Option<f64> {
    let (key, value) = line.trim().split_once('=')?;
    let micros: f64 = match key {
        "out_time_us" | "out_time_ms" => value.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    // ffmpeg's `out_time_ms` is really microseconds too.
    (micros >= 0.0).then_some(micros / 1_000_000.0)
}

/// Runs `program args…`, calling `on_progress(seconds)` as `-progress pipe:1` reports time.
/// Stops (kills the process) as soon as `cancel` is set.
pub fn run(
    program: &Path,
    args: &[String],
    cancel: &Cancel,
    mut on_progress: impl FnMut(f64),
) -> Result<Outcome, String> {
    let mut child = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let stderr = child.stderr.take();
    let tail = std::thread::spawn(move || {
        let mut lines: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        if let Some(e) = stderr {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                if lines.len() >= 12 {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
        }
        lines.into_iter().collect::<Vec<_>>().join("\n")
    });
    let (tx, rx) = std::sync::mpsc::channel::<f64>();
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        if let Some(o) = stdout {
            for line in BufReader::new(o).lines().map_while(Result::ok) {
                if let Some(s) = parse_progress_seconds(&line) {
                    if tx.send(s).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut cancelled = false;
    let status = loop {
        while let Ok(s) = rx.try_recv() {
            on_progress(s);
        }
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            let _ = child.kill();
            break child.wait().map_err(|e| e.to_string())?;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = reader.join();
    while let Ok(s) = rx.try_recv() {
        on_progress(s);
    }
    let stderr_tail = tail.join().unwrap_or_default();
    Ok(Outcome {
        success: status.success() && !cancelled,
        cancelled,
        stderr_tail,
    })
}

/// Runs a tool to completion and returns (stdout, stderr), giving up after `timeout`.
pub fn capture(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(String, String), String> {
    let mut child = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut b);
        }
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(o) = err.as_mut() {
            let _ = o.read_to_end(&mut b);
        }
        b
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timeout".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let o = t_out.join().unwrap_or_default();
    let e = t_err.join().unwrap_or_default();
    Ok((
        String::from_utf8_lossy(&o).into_owned(),
        String::from_utf8_lossy(&e).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_become_seconds() {
        assert_eq!(parse_progress_seconds("out_time_us=2500000"), Some(2.5));
        assert_eq!(parse_progress_seconds("out_time_ms=1000000"), Some(1.0));
        assert_eq!(parse_progress_seconds("frame=12"), None);
        assert_eq!(parse_progress_seconds("out_time_us=N/A"), None);
        assert_eq!(parse_progress_seconds("out_time_us=-5"), None);
        assert_eq!(parse_progress_seconds("garbage"), None);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let r = run(
            Path::new("/nonexistent/ffmpeg-xyz"),
            &[],
            &new_cancel(),
            |_| {},
        );
        assert!(r.is_err());
    }

    #[test]
    fn cancel_stops_a_running_process() {
        let Some(ffmpeg) = find_tool("ffmpeg", "") else {
            return;
        };
        let cancel = new_cancel();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.store(true, Ordering::Relaxed);
        });
        let args: Vec<String> = [
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=600",
            "-f",
            "null",
            "-",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let start = Instant::now();
        let out = run(&ffmpeg, &args, &cancel, |_| {}).unwrap();
        assert!(out.cancelled && !out.success);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
