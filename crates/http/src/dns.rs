//! DNS-over-HTTPS resolver，适配 wreq 自定义 DNS。

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;

use hickory_resolver::TokioResolver;
use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use wisp_core::error::{NetworkError, Result, WispError};
use wreq::dns::{Addrs, Name, Resolve, Resolving};

/// 使用 DoH 端点解析域名的 resolver。
#[derive(Clone)]
pub struct DoHResolver {
    inner: Arc<TokioResolver>,
}

impl DoHResolver {
    /// 从 `https://host[:port]/path` 构造；host 会在启动时解析一次为 IP。
    pub fn new(endpoint: &str) -> Result<Self> {
        let parsed = url::Url::parse(endpoint)
            .map_err(|e| WispError::Network(NetworkError::Http(format!("invalid DoH URL: {e}"))))?;
        if parsed.scheme() != "https" {
            return Err(WispError::Network(NetworkError::Http(
                "DoH endpoint must use https".into(),
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| {
                WispError::Network(NetworkError::Http("DoH endpoint missing host".into()))
            })?
            .to_string();
        let port = parsed.port().unwrap_or(443);
        let ip = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| {
                WispError::Network(NetworkError::Http(format!(
                    "resolve DoH endpoint {host}: {e}"
                )))
            })?
            .next()
            .ok_or_else(|| {
                WispError::Network(NetworkError::Http(format!(
                    "DoH endpoint {host} resolved to no address"
                )))
            })?
            .ip();
        let mut connection = ConnectionConfig::https(
            Arc::from(host.as_str()),
            Some(Arc::from(parsed.path().to_string())),
        );
        connection.port = port;
        let name_server = NameServerConfig::new(ip, true, vec![connection]);
        let config = ResolverConfig::from_parts(None, Vec::new(), vec![name_server]);
        let inner = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
            .build()
            .map_err(|e| {
                WispError::Network(NetworkError::Http(format!("build DoH resolver: {e}")))
            })?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }
}

impl Resolve for DoHResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let lookup = inner.lookup_ip(name.as_str()).await?;
            let addrs: Addrs = Box::new(
                lookup
                    .iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_endpoint() {
        assert!(DoHResolver::new("http://1.1.1.1/dns-query").is_err());
    }

    #[test]
    fn accepts_https_ip_endpoint() {
        assert!(DoHResolver::new("https://1.1.1.1/dns-query").is_ok());
    }
}
