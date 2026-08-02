# MCP crawl uses SpiderBuilder and Page

Status: accepted

MCP crawl_site is built as Engine plus SpiderBuilder instead of a custom SimpleSpider. SpiderBuilder declares max_depth, Page provides follow_links_filtered for follow_pattern regex filtering, and Engine enforces allowed domains and crawl depth. This removes duplicated parsing, link following and filtering from MCP.
