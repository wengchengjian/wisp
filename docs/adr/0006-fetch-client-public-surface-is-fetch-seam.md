# FetchClient public surface is the fetch seam

Status: superseded by 0008

FetchClient exposes new, fetch and config as the public interface. Low-level transport accessors such as fetch_http, fetch_browser, http, http_arc, browser_pool, browser_strategy and cookie_jar are no longer public; internal crates use the fetch seam or narrow hidden methods. This keeps transport invariants local and makes tests cross one interface.
