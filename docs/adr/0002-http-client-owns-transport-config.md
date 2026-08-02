# http::Client owns transport config construction

Status: accepted

http::Client now has a config-driven constructor, and FetchClient delegates all HttpConfig mapping to it. The previous manual mapping silently dropped dns_over_https; centralizing config mapping prevents future transport fields from leaking again.
