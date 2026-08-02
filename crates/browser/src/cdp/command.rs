//! CDP 命令发送与响应等待。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures::SinkExt;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tungstenite::Message;

use super::CdpSession;
use wisp_core::error::{BrowserError, Result, WispError};

impl CdpSession {
    /// Send a CDP command and wait for response.
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        self.execute_with_session(method, params, None).await
    }

    /// Send a CDP command with optional sessionId.
    pub async fn execute_with_session(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            msg["sessionId"] = json!(sid);
        }

        let text = serde_json::to_string(&msg).unwrap();
        self.writer
            .lock()
            .await
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!("ws send: {e}")))
            })?;

        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| WispError::Timeout(format!("CDP: {method}")))?;
        let response = response.map_err(|_| {
            WispError::Browser(BrowserError::CdpConnection("channel closed".into()))
        })?;

        if let Some(error) = response.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("CDP error");
            return Err(WispError::Browser(BrowserError::CdpConnection(
                msg.to_string(),
            )));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }
}
