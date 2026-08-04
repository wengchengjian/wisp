# Page owns the navigation seam

Status: accepted

`Page::goto` is the deep navigation seam. It enables the Network CDP domain,
subscribes before navigating, waits for page load, captures the document
status code, and refreshes the frame id. It returns `Result<u16>`; `reload`
remains `Result<()>`.

`recv_navigation_status` and its event parsing moved from wisp-fetcher into
wisp-browser's `page::navigation`. Dynamic and Stealth strategies call
`Page::goto` and keep only their differentiated steps: CF cookies,
ChallengeSolver, human behavior, wait-for-selector, and response extraction.

Response extraction stays in wisp-fetcher but reuses Page's high-level
`url`, `title`, `content`, and `cookie_strings` methods instead of executing
document JS directly. `Page::cookie_strings(url)` queries CDP scoped to one
URL and includes httpOnly cookies; `Response.cookies` therefore now includes
httpOnly cookie strings.

Raw `Page::session` / `session_id` / `cmd` access remains public because
stealth cookie jars and turnstile solving still use it. Narrowing that access
behind dedicated cookie APIs is a separate future candidate.

Future architecture reviews should not re-suggest duplicate navigation status
capture in fetcher strategies, raw CDP event loops in Dynamic/Stealth, or
document-JS response extraction that bypasses Page.
