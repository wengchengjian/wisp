# Fetch seam carries per-call transport options

Status: accepted

Supersedes: 0006

FetchClient's public surface is `new`, `fetch`, `fetch_with` and `config`.
`fetch_with` accepts `FetchOptions`, currently `emulation`; `fetch` delegates
with defaults so existing callers keep one call. `try_http_with_session_cookie`
is the narrow doc(hidden) method crawl Auto uses for the Cookie state fast path:
it returns `Some` only when shared session cookies produced an HTTP 200, and
blocked detection stays in crawl. Browser strategies live in one `strategy`
module: the `BrowserFetchStrategy` trait plus the Dynamic and Stealth adapters.
