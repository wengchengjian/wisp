# FetchClient owns fetch dispatch and shared cookie state

Status: accepted

FetchClient is the single seam for Http/Dynamic/Stealth fetches: it constructs and caches strategies, owns a composite CookieJar, and rejects Auto. Auto stays in crawl Engine because it needs the rule engine, CF domain locks, and upgrade middleware. This concentrates mode and cookie behavior in one module instead of scattering strategy construction across Fetcher, Engine, and MCP.
