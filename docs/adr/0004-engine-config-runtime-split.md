# Engine config and runtime resources are separate

Status: accepted

EngineConfig is the single immutable user config produced by the builder and held by Engine. FetchClient, EngineControl, stores, autoscale, event bus, middleware, and pipelines live in EngineRuntime. EngineContext consumes the same EngineConfig and EngineRuntime instead of maintaining a second subset.
