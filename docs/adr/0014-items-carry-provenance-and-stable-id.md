# Items carry provenance and a stable id

Status: accepted

`Item<T = serde_json::Value>` is the first-class item type. The engine attaches `source_url`, `spider`, `callback`, and a stable sha256-based `id` when items cross the delivery seam; Spider and Page still produce `serde_json::Value` and are unchanged.

Item pipelines, `CrawlEvent::Item`, and `run/run_many` all carry `Item<Value>`. Built-in output pipelines serialize the whole Item, so persisted JSON and MCP `crawl_site` output include provenance. Users can get a typed view through `Item::try_typed` without consuming or cloning the payload.

Future architecture reviews should not re-suggest raw `Value`-only item delivery or an engine-internal items collector. Whether `run/run_many` should stop returning items entirely remains an open decision.
