# MCP Tool uses one typed handler seam

Status: accepted

`Tool::from_handler::<Args, Output>` is the only Tool registration path. It
owns argument parsing and result serialization; each tool provides a typed
`TypedRun<Args, Output>` function that wraps the existing async handler.

`Tool::new` is removed. All five built-in tools use `from_handler`, and
`TOOLS` remains the single registry.

Future architecture reviews should not re-suggest per-tool `parse_args` /
`to_value` closures or a second closure-based Tool constructor.
