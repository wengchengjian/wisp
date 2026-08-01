//! wreq 响应转统一 Response。

use std::collections::HashMap;

use futures::StreamExt;

use super::Client;
use wisp_core::error::{NetworkError, Result, WispError};
use wisp_core::{Request as FetchRequest, Response as FetchResponse};

pub(super) async fn build_fetch_response(
    client: &Client,
    resp: wreq::Response,
    request: FetchRequest,
) -> Result<FetchResponse> {
    let status = resp.status().as_u16();
    let url = resp.uri().to_string();
    let content_type = resp
        .headers()
        .get(wreq::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // 预分配 header 容量，避免 HashMap 渐进扩容；非法 UTF-8 值跳过。
    let mut headers = HashMap::with_capacity(resp.headers().len());
    for (k, v) in resp.headers().iter() {
        if let Ok(v) = v.to_str() {
            headers.insert(k.to_string(), v.to_string());
        }
    }

    // 流式读取 body 并检查大小限制，防止超大响应导致 OOM
    let max_body_size = client.config.max_body_size;
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| WispError::Network(NetworkError::Http(format!("read body chunk: {e}"))))?;
        if body.len() + chunk.len() > max_body_size {
            return Err(WispError::Network(NetworkError::ResponseBodyTooLarge {
                url: url.clone(),
                actual: body.len() + chunk.len(),
                limit: max_body_size,
            }));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(FetchResponse::from_http(
        status,
        url,
        headers,
        body,
        content_type,
        request,
    ))
}
