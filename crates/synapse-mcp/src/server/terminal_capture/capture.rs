//! Owned-PTY capture loop (#902).
//!
//! Launches a command attached to an owned pseudoconsole (ConPTY on Windows,
//! via `portable-pty`), reads its raw VT byte stream, and drives both the
//! [`AsciicastWriter`](super::asciicast::AsciicastWriter) recording and the
//! [`ShadowScreen`](super::shadow_screen::ShadowScreen) live snapshot from it.
//!
//! The pseudoconsole is *owned* by Synapse: the child writes to a PTY we
//! created, so we see exactly what a terminal would, byte for byte — the
//! authoritative source for replay (#920) and live streaming (#914).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use super::asciicast::{AsciicastHeader, AsciicastWriter};
use super::shadow_screen::ShadowScreen;

/// Outcome of a completed capture session, with the physical artifacts as the
/// source of truth.
#[derive(Clone, Debug)]
pub(crate) struct CaptureSummary {
    /// Path of the asciicast v3 recording written.
    pub asciicast_path: PathBuf,
    /// Child process exit code (0 on clean exit).
    pub exit_code: i64,
    /// Total raw bytes read from the PTY.
    pub bytes_captured: u64,
    /// Number of `o` output events recorded.
    pub output_events: u64,
    /// The shadow screen rendered to text at the moment the child exited.
    pub final_screen_text: String,
}

/// Files owned by a live capture session.
#[derive(Clone, Debug)]
pub(crate) struct CaptureArtifacts {
    pub asciicast_path: PathBuf,
    pub status_path: PathBuf,
    pub final_screen_path: PathBuf,
}

/// Readback returned after spawning a command into an owned PTY.
#[derive(Clone, Debug)]
pub(crate) struct SpawnedCapture {
    pub process_id: u32,
    pub artifacts: CaptureArtifacts,
}

/// Specification for a capture session.
#[derive(Clone, Debug)]
pub(crate) struct CaptureSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    /// Unix-seconds timestamp stamped into the asciicast header.
    pub started_unix_secs: u64,
    pub title: Option<String>,
}

/// Starts `spec` in an owned pseudoconsole and returns immediately after the
/// child PID is known. A background waiter finalizes the physical artifacts
/// when the child exits.
pub(crate) fn spawn_capture_to_asciicast(
    spec: CaptureSpec,
    artifacts: CaptureArtifacts,
) -> anyhow::Result<SpawnedCapture> {
    let pty_system = native_pty_system();
    let size = PtySize {
        rows: spec.rows,
        cols: spec.cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = pty_system
        .openpty(size)
        .map_err(|error| anyhow::anyhow!("PTY_OPEN_FAILED: {error}"))?;

    let command = command_from_spec(&spec);
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| anyhow::anyhow!("PTY_SPAWN_FAILED: {error}"))?;
    let Some(process_id) = child.process_id() else {
        kill_spawned_child(child.as_mut(), None, "process_id_unavailable");
        return Err(anyhow::anyhow!("PTY_PROCESS_ID_UNAVAILABLE"));
    };
    drop(pair.slave);

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            kill_spawned_child(child.as_mut(), Some(process_id), "reader_clone_failed");
            return Err(anyhow::anyhow!("PTY_READER_CLONE_FAILED: {error}"));
        }
    };
    let pty_input = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            kill_spawned_child(child.as_mut(), Some(process_id), "writer_take_failed");
            return Err(anyhow::anyhow!("PTY_WRITER_TAKE_FAILED: {error}"));
        }
    };
    let file = match File::create(&artifacts.asciicast_path) {
        Ok(file) => file,
        Err(error) => {
            kill_spawned_child(child.as_mut(), Some(process_id), "asciicast_create_failed");
            return Err(anyhow::anyhow!(
                "ASCIICAST_CREATE_FAILED: {}: {error}",
                artifacts.asciicast_path.display()
            ));
        }
    };
    if let Err(error) = write_capture_status_running(&artifacts, &spec, process_id) {
        kill_spawned_child(child.as_mut(), Some(process_id), "status_write_failed");
        return Err(error);
    }

    let header = AsciicastHeader {
        cols: spec.cols,
        rows: spec.rows,
        term_type: String::new(),
        timestamp: spec.started_unix_secs,
        title: spec.title.clone(),
    };
    let cols = spec.cols;
    let rows = spec.rows;
    let reader_thread = spawn_reader_thread(reader, pty_input, file, header, cols, rows);
    let waiter_artifacts = artifacts.clone();
    let mut spawn_failure_killer = child.clone_killer();
    std::thread::Builder::new()
        .name(format!("synapse-pty-wait-{process_id}"))
        .spawn(move || {
            let status = child.wait();
            drop(pair.master);
            let outcome = reader_thread.join();
            if let Err(error) =
                finalize_capture_artifacts(&waiter_artifacts, process_id, status, outcome)
            {
                tracing::warn!(
                    code = "PTY_CAPTURE_FINALIZE_FAILED",
                    process_id,
                    error = %error,
                    "failed to finalize PTY capture artifacts"
                );
            }
        })
        .map_err(move |error| {
            if let Err(kill_error) = spawn_failure_killer.kill() {
                tracing::warn!(
                    code = "PTY_CAPTURE_THREAD_SPAWN_CLEANUP_FAILED",
                    process_id,
                    error = %kill_error,
                    "failed to kill PTY child after waiter thread spawn failed"
                );
            }
            anyhow::anyhow!("PTY_WAITER_THREAD_SPAWN_FAILED: {error}")
        })?;

    Ok(SpawnedCapture {
        process_id,
        artifacts,
    })
}

/// Runs `spec` to completion attached to an owned pseudoconsole, writing the
/// asciicast v3 recording to `asciicast_path` and returning a summary. Blocking:
/// the daemon runs one of these on a dedicated thread per spawned agent.
pub(crate) fn capture_to_asciicast(
    spec: &CaptureSpec,
    asciicast_path: &Path,
) -> anyhow::Result<CaptureSummary> {
    let pty_system = native_pty_system();
    let size = PtySize {
        rows: spec.rows,
        cols: spec.cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = pty_system
        .openpty(size)
        .map_err(|error| anyhow::anyhow!("PTY_OPEN_FAILED: {error}"))?;

    let command = command_from_spec(spec);

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| anyhow::anyhow!("PTY_SPAWN_FAILED: {error}"))?;
    // The slave handle must be dropped after spawn so the child is the only
    // writer to the PTY.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow::anyhow!("PTY_READER_CLONE_FAILED: {error}"))?;
    let pty_input = pair
        .master
        .take_writer()
        .map_err(|error| anyhow::anyhow!("PTY_WRITER_TAKE_FAILED: {error}"))?;

    let file = File::create(asciicast_path).map_err(|error| {
        anyhow::anyhow!(
            "ASCIICAST_CREATE_FAILED: {}: {error}",
            asciicast_path.display()
        )
    })?;
    let header = AsciicastHeader {
        cols: spec.cols,
        rows: spec.rows,
        term_type: String::new(),
        timestamp: spec.started_unix_secs,
        title: spec.title.clone(),
    };
    let cols = spec.cols;
    let rows = spec.rows;

    // Read on a dedicated thread. On Windows ConPTY the master reader does NOT
    // return EOF until the pseudoconsole (master) is closed — which we must not
    // do until the child has exited. A single-threaded read-then-wait therefore
    // deadlocks. So: drain on a worker thread (owning the recorder + shadow
    // screen), wait for the child on this thread, THEN drop the master to force
    // the reader to EOF, then join.
    let reader_thread = spawn_reader_thread(reader, pty_input, file, header, cols, rows);

    capture_trace("waiting on child");
    let status = child
        .wait()
        .map_err(|error| anyhow::anyhow!("PTY_WAIT_FAILED: {error}"))?;
    let exit_code = i64::from(status.exit_code());
    capture_trace(&format!("child exited code={exit_code}; dropping master"));
    // Close the pseudoconsole so the reader thread observes EOF and finishes.
    drop(pair.master);
    capture_trace("master dropped; joining reader thread");
    let mut outcome = reader_thread
        .join()
        .map_err(|_| anyhow::anyhow!("PTY_READER_THREAD_PANICKED"))??;
    capture_trace("reader thread joined");

    outcome
        .writer
        .record_exit(outcome.elapsed, exit_code)
        .map_err(|error| anyhow::anyhow!("ASCIICAST_EXIT_WRITE_FAILED: {error}"))?;

    Ok(CaptureSummary {
        asciicast_path: asciicast_path.to_path_buf(),
        exit_code,
        bytes_captured: outcome.bytes_captured,
        output_events: outcome.output_events,
        final_screen_text: outcome.screen.render_text(),
    })
}

fn command_from_spec(spec: &CaptureSpec) -> CommandBuilder {
    let mut command = CommandBuilder::new(&spec.program);
    for arg in &spec.args {
        command.arg(arg);
    }
    if let Some(cwd) = spec.cwd.as_ref() {
        command.cwd(cwd);
    }
    // portable-pty's CommandBuilder launches with an EMPTY environment. On
    // Windows a process started without SystemRoot/PATH fails at DLL init with
    // STATUS_DLL_INIT_FAILED (0xC0000142) before main runs.
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    command
}

fn kill_spawned_child(
    child: &mut dyn portable_pty::Child,
    process_id: Option<u32>,
    reason: &'static str,
) {
    if let Err(error) = child.kill() {
        tracing::warn!(
            code = "PTY_CAPTURE_STARTUP_CLEANUP_FAILED",
            ?process_id,
            reason,
            error = %error,
            "failed to kill PTY child after capture startup failed"
        );
    }
}

fn spawn_reader_thread(
    reader: Box<dyn Read + Send>,
    pty_input: Box<dyn Write + Send>,
    file: File,
    header: AsciicastHeader,
    cols: u16,
    rows: u16,
) -> std::thread::JoinHandle<anyhow::Result<ReaderOutcome>> {
    std::thread::spawn(move || -> anyhow::Result<ReaderOutcome> {
        capture_trace("reader thread started");
        let mut reader = reader;
        let mut pty_input = pty_input;
        let mut terminal_responder = TerminalQueryResponder::default();
        let mut writer = AsciicastWriter::start(BufWriter::new(file), &header)
            .map_err(|error| anyhow::anyhow!("ASCIICAST_HEADER_WRITE_FAILED: {error}"))?;
        let mut screen = ShadowScreen::new(cols, rows);
        let start = Instant::now();
        let mut buffer = [0u8; 8192];
        let mut bytes_captured = 0u64;
        let mut output_events = 0u64;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    capture_trace("reader EOF (read returned 0)");
                    break;
                }
                Ok(n) => {
                    let chunk = &buffer[..n];
                    writer
                        .record_output(start.elapsed(), chunk)
                        .map_err(|error| anyhow::anyhow!("ASCIICAST_WRITE_FAILED: {error}"))?;
                    screen.feed(chunk);
                    if let Err(error) = terminal_responder.respond(chunk, pty_input.as_mut()) {
                        tracing::warn!(
                            code = "PTY_TERMINAL_QUERY_RESPONSE_FAILED",
                            error = %error,
                            "failed to answer PTY terminal query"
                        );
                    }
                    bytes_captured += n as u64;
                    output_events += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(anyhow::anyhow!("PTY_READ_FAILED: {error}")),
            }
        }
        Ok(ReaderOutcome {
            writer,
            screen,
            bytes_captured,
            output_events,
            elapsed: start.elapsed(),
        })
    })
}

#[derive(Default)]
struct TerminalQueryResponder {
    pending: Vec<u8>,
}

impl TerminalQueryResponder {
    fn respond(&mut self, chunk: &[u8], pty_input: &mut dyn Write) -> std::io::Result<()> {
        self.pending.extend_from_slice(chunk);
        let mut processed_until = 0;
        while let Some((offset, sequence_len, response)) =
            next_supported_terminal_query(&self.pending[processed_until..])
        {
            let sequence_start = processed_until + offset;
            pty_input.write_all(response)?;
            pty_input.flush()?;
            processed_until = sequence_start + sequence_len;
        }

        let retain_len = supported_terminal_query_max_len().saturating_sub(1);
        let retain_from = self
            .pending
            .len()
            .saturating_sub(retain_len)
            .max(processed_until);
        if retain_from > 0 {
            self.pending.drain(..retain_from);
        }
        Ok(())
    }
}

fn next_supported_terminal_query(haystack: &[u8]) -> Option<(usize, usize, &'static [u8])> {
    const CPR_RESPONSE: &[u8] = b"\x1b[1;1R";
    const QUERIES: [&[u8]; 2] = [b"\x1b[6n", b"\x1b[?6n"];
    QUERIES
        .iter()
        .filter_map(|query| find_subslice(haystack, query).map(|offset| (offset, query.len())))
        .min_by_key(|(offset, _len)| *offset)
        .map(|(offset, len)| (offset, len, CPR_RESPONSE))
}

fn supported_terminal_query_max_len() -> usize {
    b"\x1b[?6n".len()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn finalize_capture_artifacts(
    artifacts: &CaptureArtifacts,
    process_id: u32,
    status: std::io::Result<portable_pty::ExitStatus>,
    outcome: std::thread::Result<anyhow::Result<ReaderOutcome>>,
) -> anyhow::Result<()> {
    let mut outcome = outcome
        .map_err(|_| anyhow::anyhow!("PTY_READER_THREAD_PANICKED"))?
        .map_err(|error| anyhow::anyhow!("PTY_READER_THREAD_FAILED: {error}"))?;
    let exit_code = status
        .map_err(|error| anyhow::anyhow!("PTY_WAIT_FAILED: {error}"))
        .map(|status| i64::from(status.exit_code()))?;
    outcome
        .writer
        .record_exit(outcome.elapsed, exit_code)
        .map_err(|error| anyhow::anyhow!("ASCIICAST_EXIT_WRITE_FAILED: {error}"))?;
    let final_screen_text = outcome.screen.render_text();
    std::fs::write(&artifacts.final_screen_path, &final_screen_text).map_err(|error| {
        anyhow::anyhow!(
            "FINAL_SCREEN_WRITE_FAILED: {}: {error}",
            artifacts.final_screen_path.display()
        )
    })?;
    write_capture_status_finished(
        artifacts,
        process_id,
        exit_code,
        outcome.bytes_captured,
        outcome.output_events,
        final_screen_text.len(),
    )
}

fn write_capture_status_running(
    artifacts: &CaptureArtifacts,
    spec: &CaptureSpec,
    process_id: u32,
) -> anyhow::Result<()> {
    write_capture_status_json(
        artifacts,
        serde_json::json!({
            "schema_version": 1,
            "status": "running",
            "process_id": process_id,
            "program": spec.program,
            "args": spec.args,
            "cwd": spec.cwd.as_ref().map(|path| path.display().to_string()),
            "cols": spec.cols,
            "rows": spec.rows,
            "started_unix_secs": spec.started_unix_secs,
            "title": spec.title,
            "asciicast_path": artifacts.asciicast_path.display().to_string(),
            "final_screen_path": artifacts.final_screen_path.display().to_string(),
        }),
    )
}

fn write_capture_status_finished(
    artifacts: &CaptureArtifacts,
    process_id: u32,
    exit_code: i64,
    bytes_captured: u64,
    output_events: u64,
    final_screen_bytes: usize,
) -> anyhow::Result<()> {
    write_capture_status_json(
        artifacts,
        serde_json::json!({
            "schema_version": 1,
            "status": "finished",
            "process_id": process_id,
            "exit_code": exit_code,
            "bytes_captured": bytes_captured,
            "output_events": output_events,
            "asciicast_path": artifacts.asciicast_path.display().to_string(),
            "final_screen_path": artifacts.final_screen_path.display().to_string(),
            "final_screen_bytes": final_screen_bytes,
        }),
    )
}

fn write_capture_status_json(
    artifacts: &CaptureArtifacts,
    status: serde_json::Value,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(&status)
        .map_err(|error| anyhow::anyhow!("CAPTURE_STATUS_ENCODE_FAILED: {error}"))?;
    std::fs::write(&artifacts.status_path, bytes).map_err(|error| {
        anyhow::anyhow!(
            "CAPTURE_STATUS_WRITE_FAILED: {}: {error}",
            artifacts.status_path.display()
        )
    })
}

/// Phase tracing for the capture loop, gated to the `synapse_pty_trace` cfg-less
/// env var so it is silent in production but visible when debugging a hang.
fn capture_trace(message: &str) {
    if std::env::var_os("SYNAPSE_PTY_TRACE").is_some() {
        eprintln!("[pty-capture] {message}");
    }
}

/// What the reader thread hands back so the caller can finalize the recording.
struct ReaderOutcome {
    writer: AsciicastWriter<BufWriter<File>>,
    screen: ShadowScreen,
    bytes_captured: u64,
    output_events: u64,
    elapsed: std::time::Duration,
}

/// A live, shareable handle to a capture session's shadow screen, so the #914
/// streaming endpoint can dump the current screen on attach while the capture
/// loop keeps feeding it. (Wired into the spawn path as #914 lands.)
pub(crate) type SharedShadowScreen = Arc<Mutex<ShadowScreen>>;

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    fn read_events(path: &Path) -> (Value, Vec<Value>) {
        let text = std::fs::read_to_string(path).expect("read asciicast");
        let mut lines = text.lines();
        let header: Value = serde_json::from_str(lines.next().expect("header line")).expect("hdr");
        let events = lines
            .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
            .map(|line| serde_json::from_str::<Value>(line).expect("event json"))
            .collect();
        (header, events)
    }

    #[test]
    fn terminal_query_responder_answers_cursor_position_request() {
        let mut responder = TerminalQueryResponder::default();
        let mut pty_input = Vec::new();

        responder
            .respond(b"\x1b[6n", &mut pty_input)
            .expect("respond succeeds");

        assert_eq!(pty_input, b"\x1b[1;1R");
    }

    #[test]
    fn terminal_query_responder_handles_split_cursor_position_request() {
        let mut responder = TerminalQueryResponder::default();
        let mut pty_input = Vec::new();

        responder
            .respond(b"prefix\x1b[", &mut pty_input)
            .expect("partial respond succeeds");
        assert!(pty_input.is_empty());

        responder
            .respond(b"?6nsuffix\x1b[6n", &mut pty_input)
            .expect("final respond succeeds");

        assert_eq!(pty_input, b"\x1b[1;1R\x1b[1;1R");
    }

    #[test]
    #[ignore = "real-process FSV: opens a real owned ConPTY and captures a child's output. Run from an INTERACTIVE console session (`cargo test -p synapse-mcp -- --ignored`). The conpty-hosted child fails DLL init (0xC0000142) or hangs under restricted automation window-stations, which is an environment limitation, not a capture-code defect — the byte->asciicast and byte->screen transforms are fully covered by the default-gate unit tests."]
    fn captures_real_process_output_to_valid_asciicast_v3() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let asciicast_path = temp.path().join("session.cast");
        let marker = "HELLO_CONPTY_7F3A9";

        // A real child process writing to the owned pseudoconsole.
        #[cfg(windows)]
        let spec = CaptureSpec {
            program: "cmd.exe".to_owned(),
            args: vec!["/c".to_owned(), format!("echo {marker}")],
            cwd: Some(temp.path().to_path_buf()),
            env: std::env::vars().collect(),
            cols: 80,
            rows: 24,
            started_unix_secs: 1_700_000_000,
            title: Some("conpty-fsv".to_owned()),
        };
        #[cfg(not(windows))]
        let spec = CaptureSpec {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), format!("echo {marker}")],
            cwd: Some(temp.path().to_path_buf()),
            env: std::env::vars().collect(),
            cols: 80,
            rows: 24,
            started_unix_secs: 1_700_000_000,
            title: Some("pty-fsv".to_owned()),
        };

        let summary = capture_to_asciicast(&spec, &asciicast_path).expect("capture succeeds");

        // Source of truth 1: the recording exists on disk and is valid v3.
        assert!(asciicast_path.exists(), "asciicast file must be written");
        let (header, events) = read_events(&asciicast_path);
        assert_eq!(header["version"], 3, "must be asciicast v3");
        assert_eq!(header["term"]["cols"], 80);
        assert_eq!(header["term"]["rows"], 24);

        // Source of truth 2: an output event carries the child's real stdout.
        let captured_output: String = events
            .iter()
            .filter(|event| event[1] == "o")
            .map(|event| event[2].as_str().unwrap_or_default().to_owned())
            .collect();
        assert!(
            captured_output.contains(marker),
            "captured output must contain the echoed marker; got: {captured_output:?}"
        );

        // Source of truth 3: a terminating exit event with the child's code.
        let exit_event = events.iter().rev().find(|event| event[1] == "x");
        let exit_event = exit_event.expect("an exit event must terminate the recording");
        assert_eq!(exit_event[2], "0", "echo exits 0");
        assert_eq!(summary.exit_code, 0);

        // Source of truth 4: the shadow screen rendered the same text.
        assert!(
            summary.final_screen_text.contains(marker),
            "shadow screen must render the marker; got: {:?}",
            summary.final_screen_text
        );
        assert!(summary.bytes_captured > 0 && summary.output_events > 0);
    }
}
