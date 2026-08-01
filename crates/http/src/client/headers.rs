//! 请求 header 合并。

use wreq::header::HeaderName;

use super::Client;

impl Client {
    /// 合并 config headers 与 per-request extra headers（extra 覆盖同名 config header）。
    pub(super) fn build_headers_with<'a>(
        &self,
        extra_headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> wreq::header::HeaderMap {
        let mut map = self.build_headers();
        for (k, v) in extra_headers {
            match (
                HeaderName::from_bytes(k.as_bytes()),
                wreq::header::HeaderValue::from_str(v),
            ) {
                (Ok(name), Ok(val)) => {
                    map.insert(name, val);
                }
                (Err(e), _) => tracing::warn!("跳过无效 header 名 '{}': {}", k, e),
                (_, Err(e)) => tracing::warn!("跳过无效 header 值 '{}': {}", k, e),
            }
        }
        map
    }

    fn build_headers(&self) -> wreq::header::HeaderMap {
        let mut map = wreq::header::HeaderMap::new();
        for (k, v) in &self.config.headers {
            match (
                HeaderName::from_bytes(k.as_bytes()),
                wreq::header::HeaderValue::from_str(v),
            ) {
                (Ok(name), Ok(val)) => {
                    map.insert(name, val);
                }
                (Err(e), _) => tracing::warn!("跳过无效 config header 名 '{}': {}", k, e),
                (_, Err(e)) => tracing::warn!("跳过无效 config header 值 '{}': {}", k, e),
            }
        }
        map
    }
}
