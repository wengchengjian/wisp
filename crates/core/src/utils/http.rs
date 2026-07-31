//! HTTP 相关工具函数。

/// 将 HTTP 状态码映射为标准 reason phrase。
///
/// 覆盖常见状态码，未知状态码返回 `"Unknown"`。
pub fn status_text(code: u16) -> &'static str {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        429 => "Too Many Requests",
        444 => "No Response",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_status_codes() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(301), "Moved Permanently");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(500), "Internal Server Error");
        assert_eq!(status_text(429), "Too Many Requests");
    }

    #[test]
    fn unknown_status_code() {
        assert_eq!(status_text(999), "Unknown");
    }
}
