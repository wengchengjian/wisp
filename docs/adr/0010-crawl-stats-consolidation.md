# Crawl stats consolidation changes checkpoint format

Status: accepted

CrawlStats is the single serializable Crawl stats model and the only public
stats surface. SpiderStats is a crate-private counter adapter; CrawlState
embeds a CrawlStats value for checkpoint persistence instead of duplicating
stat fields. Duration serializes as u64 milliseconds.

Checkpoint blobs written before this change do not deserialize. Loading an
old blob logs a warning and skips it; the engine continues without crashing.
No version envelope is added because the project is pre-1.0 and the failure
path is already safe.
