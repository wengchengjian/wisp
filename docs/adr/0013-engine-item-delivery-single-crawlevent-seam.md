# Engine item delivery uses one CrawlEvent seam

Status: accepted

Items produced by a Spider flow through ItemPipeline and are then emitted once as `CrawlEvent::Item` on the EventBus. `run` and `run_many` are convenience adapters: they consume the CrawlEvent stream, collect Item values, and return them alongside stats. Typed errors are preserved through an internal `oneshot` outcome channel instead of degrading to the stream's string error.

The engine no longer owns an items collector. `EngineState.items` is removed, `run_inner_many` does not take an items Vec, and streaming runs no longer accumulate a discarded item list. `CrawlStream`'s inner stream is `Send` so `run_many` futures can cross threads.

Future architecture reviews should not re-suggest adding an engine-internal items collector. Whether `run/run_many` should stop returning items entirely, or whether Item should become a dedicated type instead of `serde_json::Value`, is a separate open decision.
