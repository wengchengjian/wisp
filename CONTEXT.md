# wisp

Rust web scraping framework that coordinates Fetch, Crawl, Spider, Tool, and Cookie state.

## Language

**Fetch**:
A single retrieval operation that returns a Response, regardless of transport.
_Avoid_: request (the input), page load

**Fetch mode**:
The named transport strategy for a Fetch: Http, Dynamic, Stealth, or Auto.
_Avoid_: browser mode

**Proxy**:
Transport route selected for a Fetch; HTTP honors per-request proxies, browser supports only the client-level configured proxy.
_Avoid_: proxy server

**Crawl**:
A repeated Fetch execution driven by scheduling, middleware, and stop conditions.
_Avoid_: scraping run

**Crawl depth**:
Maximum follow hops a Spider allows; declared by the Spider and enforced by Engine.
_Avoid_: depth limit

**Engine**:
The Crawl module that schedules Fetches, runs middleware, and enforces stop conditions.
_Avoid_: runner

**Spider**:
User-defined crawl behavior: start URLs, response handling, follow requests, and stopping.
_Avoid_: scraper

**Cookie state**:
Cookies attached to a site across Fetch modes through one shared seam.
_Avoid_: cookie jar (implementation)

**Tool**:
A typed MCP operation exposing Fetch, Crawl, or adaptive behavior to clients.
_Avoid_: MCP endpoint
