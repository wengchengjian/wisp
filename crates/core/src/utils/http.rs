//! HTTP 相关工具函数。

/// 将 HTTP 状态码映射为标准 reason phrase。
///
/// 覆盖常见状态码，未知状态码返回 `"Unknown"`。
fn status_text_1xx(code: u16) -> Option<&'static str> {
    match code {
        100 => Some("Continue"),
        101 => Some("Switching Protocols"),
        _ => None,
    }
}

fn status_text_2xx(code: u16) -> Option<&'static str> {
    match code {
        200 => Some("OK"),
        201 => Some("Created"),
        202 => Some("Accepted"),
        204 => Some("No Content"),
        206 => Some("Partial Content"),
        _ => None,
    }
}

fn status_text_3xx(code: u16) -> Option<&'static str> {
    match code {
        301 => Some("Moved Permanently"),
        302 => Some("Found"),
        303 => Some("See Other"),
        304 => Some("Not Modified"),
        307 => Some("Temporary Redirect"),
        308 => Some("Permanent Redirect"),
        _ => None,
    }
}

fn status_text_40x(code: u16) -> Option<&'static str> {
    match code {
        400 => Some("Bad Request"),
        401 => Some("Unauthorized"),
        403 => Some("Forbidden"),
        404 => Some("Not Found"),
        405 => Some("Method Not Allowed"),
        407 => Some("Proxy Authentication Required"),
        408 => Some("Request Timeout"),
        409 => Some("Conflict"),
        410 => Some("Gone"),
        _ => None,
    }
}

fn status_text_42x(code: u16) -> Option<&'static str> {
    match code {
        429 => Some("Too Many Requests"),
        _ => None,
    }
}

fn status_text_44x(code: u16) -> Option<&'static str> {
    match code {
        444 => Some("No Response"),
        _ => None,
    }
}

fn status_text_4xx(code: u16) -> Option<&'static str> {
    status_text_40x(code)
        .or_else(|| status_text_42x(code))
        .or_else(|| status_text_44x(code))
}

fn status_text_5xx(code: u16) -> Option<&'static str> {
    match code {
        500 => Some("Internal Server Error"),
        502 => Some("Bad Gateway"),
        503 => Some("Service Unavailable"),
        504 => Some("Gateway Timeout"),
        _ => None,
    }
}

pub fn status_text(code: u16) -> &'static str {
    status_text_1xx(code)
        .or_else(|| status_text_2xx(code))
        .or_else(|| status_text_3xx(code))
        .or_else(|| status_text_4xx(code))
        .or_else(|| status_text_5xx(code))
        .unwrap_or("Unknown")
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
