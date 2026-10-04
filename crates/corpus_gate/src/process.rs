//! Execute one observed phase, draining both pipes while the child is running.
use std::{
    io::{self, Read},
    path::Path,
    process::{Command, Output, Stdio},
    thread::{self, JoinHandle},
};

fn reader(pipe: impl Read + Send + 'static) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut pipe = pipe;
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn finish(reader: JoinHandle<io::Result<Vec<u8>>>, phase: &str) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| format!("{phase} output reader panicked"))?
        .map_err(|error| format!("{phase} output read failed: {error}"))
}

pub(super) fn execute(
    deka: &Path,
    phase: &str,
    entry: &str,
    project: &Path,
    slug: &str,
) -> Result<Output, String> {
    let mut child = Command::new(deka)
        .args([phase, entry])
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn {} {phase}: {error}", deka.display()))?;
    // Both handles exist because this command explicitly requests piped output.
    let stdout = reader(child.stdout.take().expect("piped child stdout"));
    let stderr = reader(child.stderr.take().expect("piped child stderr"));
    let deadline = std::time::Instant::now() + super::RUN_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(std::time::Duration::from_millis(25));
            }
            Ok(None) => {
                break Err(format!(
                    "{slug} {phase} timed out after {:?}",
                    super::RUN_TIMEOUT
                ));
            }
            Err(error) => break Err(format!("failed to wait for {phase}: {error}")),
        }
    };
    if status.is_err() {
        // This is exactly the child we started, never a name/pattern kill.
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = finish(stdout, phase);
    let stderr = finish(stderr, phase);
    Ok(Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
}
