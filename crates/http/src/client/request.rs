//! 统一请求入口与便捷方法。

use serde_json::Value;

use super::Client;
use super::error::classify_request_error;
use super::response::build_fetch_response;
use wisp_core::error::{ParseError, Result, WispError};
use wisp_core::{Method as FetchMethod, Request as FetchRequest, Response as FetchResponse};

impl Client {
    /// 统一请求入口：接受 `fetcher::Request`，直接返回 `fetcher::Response`。
    ///
    /// 消除 http::Response 中间类型，避免字段克隆转换。
    #[tracing::instrument(level = "trace", skip(self, req), fields(url = %req.url))]
    pub async fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse> {
        let extra_headers = req.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()));

        let wreq_resp = match req.method {
            FetchMethod::Get => {
                self.http
                    .get(&req.url)
                    .headers(self.build_headers_with(extra_headers))
                    .send()
                    .await
            }
            FetchMethod::Post => {
                let mut builder = self
                    .http
                    .post(&req.url)
                    .headers(self.build_headers_with(extra_headers));
                if let Some(ref b) = req.body {
                    builder = builder.body(b.clone());
                }
                builder.send().await
            }
            FetchMethod::Put => {
                let mut builder = self
                    .http
                    .put(&req.url)
                    .headers(self.build_headers_with(extra_headers));
                if let Some(ref b) = req.body {
                    builder = builder.body(b.clone());
                }
                builder.send().await
            }
            FetchMethod::Delete => {
                self.http
                    .delete(&req.url)
                    .headers(self.build_headers_with(extra_headers))
                    .send()
                    .await
            }
        };

        let wreq_resp =
            wreq_resp.map_err(|e| classify_request_error(&e, &req.url, self.config.timeout))?;
        build_fetch_response(self, wreq_resp, req.clone()).await
    }

    /// GET request（便捷方法，内部构造 Request）。
    pub async fn get(
        &self,
        url: &str,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest::get(url);
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        self.fetch(&req).await
    }

    /// POST request with optional body/json（便捷方法）。
    pub async fn post(
        &self,
        url: &str,
        body: Option<&str>,
        json: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest::post(url, body.map(|b| b.to_string()));
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON serialize: {e}"))))?;
            req.body = Some(json_str);
            req.headers
                .insert("content-type".to_string(), "application/json".to_string());
        }
        self.fetch(&req).await
    }

    /// PUT request（便捷方法）。
    pub async fn put(
        &self,
        url: &str,
        body: Option<&str>,
        json: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest {
            url: url.to_string(),
            method: FetchMethod::Put,
            body: body.map(|b| b.to_string()),
            ..Default::default()
        };
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON serialize: {e}"))))?;
            req.body = Some(json_str);
            req.headers
                .insert("content-type".to_string(), "application/json".to_string());
        }
        self.fetch(&req).await
    }

    /// DELETE request（便捷方法）。
    pub async fn delete(
        &self,
        url: &str,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest {
            url: url.to_string(),
            method: FetchMethod::Delete,
            ..Default::default()
        };
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        self.fetch(&req).await
    }
}
