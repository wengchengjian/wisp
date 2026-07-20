use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use serde_json::Value;
use crate::error::{PatchrightError, Result};

/// Pipe-based CDP transport using Chrome's stdin/stdout.
/// Messages are JSON objects delimited by null bytes (\0).
pub struct PipeTransport {
    writer: ChildStdin,
    reader: BufReader<ChildStdout>,
}

impl PipeTransport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            writer: stdin,
            reader: BufReader::new(stdout),
        }
    }

    /// Send a CDP message (JSON + null byte)
    pub async fn send(&mut self, msg: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(msg)
            .map_err(|e| PatchrightError::CdpError(format!("serialize: {e}")))?;
        bytes.push(0);
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Receive next CDP message (read until null byte)
    pub async fn recv(&mut self) -> Result<Value> {
        let mut buf = Vec::new();
        let n = self.reader.read_until(0, &mut buf).await?;
        if n == 0 {
            return Err(PatchrightError::CdpError("pipe closed".into()));
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            return Err(PatchrightError::CdpError("empty message".into()));
        }
        serde_json::from_slice(&buf)
            .map_err(|e| PatchrightError::CdpError(format!("deserialize: {e}")))
    }
}
