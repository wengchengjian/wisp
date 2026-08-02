# wisp

Rust 爬虫框架。提供 Spider trait（含 callback 路由的 `handle` 方法）、SpiderBuilder 多 handler 构建、Engine 纯基础设施（HTTP/缓存/代理池共享）。

## 构建

```bash
cargo build            # lib + bins
cargo build --release
```

## Agent skills

### Issue tracker

Issues for this repo live in GitHub Issues and are operated through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage labels are `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, and `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: read root `CONTEXT.md` and `docs/adr/` before working in an area. See `docs/agents/domain.md`.
