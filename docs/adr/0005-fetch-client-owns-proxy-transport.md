# FetchClient owns per-request proxy transport

Status: accepted

FetchClient reads Request.proxy and builds cached per-proxy HTTP clients from the full HttpConfig, so UA, DoH, emulation, body limits and certificate settings do not leak when a proxy is selected. Browser modes reject a per-request proxy that differs from the configured browser proxy instead of silently ignoring it.
