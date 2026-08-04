# Request splits transport from Crawl state

Status: accepted

`Request` is transport-only: `url`, `method`, `headers`, `body`, and
`proxy`. `proxy` stays on the transport Request because middleware injects it
before FetchClient reads it; ADR-0005 is not reopened.

`CrawlRequest` lives in wisp-core and wraps a transport `Request` plus
`callback`, `spider`, `priority`, `depth`, `meta`, `retry_count`, and
`fetch_mode_override`. It implements `Deref` and `DerefMut<Target=Request>` so
middleware can still mutate headers, body, and proxy. Crawl builders moved to
`CrawlRequest`.

`Response.request` is a `CrawlRequest`; `from_http` / `from_browser` still
accept a transport `Request` and wrap it with default Crawl state. Engine
reattaches the original `CrawlRequest` after every fetch, so callback, spider,
depth, meta, and mode override survive the transport round trip.

The FetchClient seam still takes `&Request` transport-only. Spider,
Page, Middleware, scheduler, and checkpoint all carry `CrawlRequest`.

Checkpoint serialization persists `callback`, `spider`, `priority`, `depth`,
and `meta`; `retry_count`, `fetch_mode_override`, and `proxy` remain skipped,
matching the previous behavior.

Future architecture reviews should not re-suggest a single merged Request
type, or moving proxy to FetchOptions without a design for middleware-side
per-request injection.
