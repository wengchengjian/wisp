# One-shot Fetcher stays as a convenience layer

Status: accepted

Fetcher remains a public one-shot convenience layer over the FetchClient
fetch seam. It carries a mode and delegates to `FetchClient::fetch`; it is
not a competing transport implementation.

Removing it would push mode handling back into every one-shot caller
(examples, integration tests, docs) without shrinking any real interface.
Future architecture reviews should not re-suggest collapsing Fetcher into
FetchClient.
