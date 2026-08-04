# Item output is one Result-capable module

Status: accepted

`ItemOutput` is the single Item output module. It owns JSON, JSONL, Markdown,
and WARC serialization behind `open` / `write` / `close`, and every write error
propagates instead of being swallowed.

`OutputWriterPipeline` is an adapter over `ItemOutput`; `Items` is an
in-memory view that keeps `Item` provenance; MCP `crawl_site` reuses the same
JSONL serialization. `runtime/items.rs`, `JsonlWriter`, and
`JsonlWriterPipeline` are removed. `OutputWriterPipeline::new` overwrites on
open and `new_append` appends JSONL.

`ItemPipeline` is the public extension seam for custom pipelines.
`open` / `process_item` / `close` return `Result`; `Ok(None)` means discard and
`Err` means failure. When a pipeline fails, Engine emits one
`CrawlEvent::Error`, stops scheduling new work, and returns the typed error
from `run` / `run_many`.

`BatchItemPipeline.flush_fn` returns `Result<()>` so database batch closures
(including SQLite through `wisp_storage::Store`) can report failures. No
built-in SQLite Item pipeline is added; user closures remain the adapter.

Future architecture reviews should not re-suggest raw `Value`-only output,
duplicate JSONL writers, or an `Option`-only `ItemPipeline` that cannot
distinguish discard from failure. Whether `run` / `run_many` should stop
returning items entirely remains the open decision recorded in ADR-0013.
