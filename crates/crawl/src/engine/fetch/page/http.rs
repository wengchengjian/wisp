//! HTTP 抓取路径（代理客户端缓存）。

use std::sync::Arc;

use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_http::Client;

pub(super) async fn fetch_http_response(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
) -> Result<Response> {
    let base_client = fetch_client.http();
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        let timeout = base_client.config_ref().timeout;
        let proxy_owned = proxy.to_string();
        let client: Arc<Client> = proxy_clients
            .try_get_with(proxy_owned.clone(), move || {
                let new_client = Client::builder()
                    .timeout(timeout)
                    .proxy(&proxy_owned)
                    .build()?;
                Ok::<Arc<Client>, wisp_core::error::WispError>(Arc::new(new_client))
            })
            .map_err(|e| {
                wisp_core::error::WispError::Network(wisp_core::error::NetworkError::Http(format!(
                    "proxy client build failed: {}",
                    e
                )))
            })?;
        Some(client)
    } else {
        None
    };
    let use_client: &Client = match &proxy_client {
        Some(c) => c.as_ref(),
        None => base_client,
    };
    use_client.fetch(req).await
}
