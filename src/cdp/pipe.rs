use std::io::{BufRead, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::Arc;
use tokio::sync::Mutex;
use serde_json::Value;
use crate::error::{PatchrightError, Result};

/// Pipe-based CDP transport using std ChildStdin/ChildStdout.
/// Messages are JSON objects delimited by null bytes (\0).
/// Uses a background thread for reading to avoid tokio conversion issues on Windows.
pub struct PipeTransport {
    writer: Arc<Mutex<ChildStdin>>,
    #[allow(dead_code)]
    reader_handle: std::thread::JoinHandle<()>,
}

impl PipeTransport {
    /// Create a new pipe transport. Returns the transport and a receiver for incoming messages.
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> (Self, tokio::sync::mpsc::UnboundedReceiver<Value>) {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();

        let reader_handle = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            eprintln!("[pipe-reader] thread started");
            loop {
                let mut buf = Vec::new();
                match reader.read_until(0, &mut buf) {
                    Ok(0) => { eprintln!("[pipe-reader] EOF"); break; }
                    Ok(n) => {
                        eprintln!("[pipe-reader] read {n} bytes");
                        if buf.last() == Some(&0) { buf.pop(); }
                        if buf.is_empty() { continue; }
                        if let Ok(msg) = serde_json::from_slice::<Value>(&buf) {
                            if msg_tx.send(msg).is_err() { eprintln!("[pipe-reader] channel closed"); break; }
                        } else {
                            eprintln!("[pipe-reader] malformed JSON");
                        }
                    }
                    Err(e) => { eprintln!("[pipe-reader] read error: {e}"); break; }
                }
            }
        });

        (Self { writer: Arc::new(Mutex::new(stdin)), reader_handle }, msg_rx)
    }

    /// Send a CDP message (JSON + null byte)
    pub async fn send(&self, msg: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(msg)
            .map_err(|e| PatchrightError::CdpError(format!("serialize: {e}")))?;
        bytes.push(0);
        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            let mut w = writer.blocking_lock();
            w.write_all(&bytes)?;
            w.flush()?;
            Ok::<_, std::io::Error>(())
        })
        .await
        .map_err(|e| PatchrightError::CdpError(format!("write join: {e}")))?
        .map_err(|e| PatchrightError::CdpError(format!("write: {e}")))?;
        Ok(())
    }
}
