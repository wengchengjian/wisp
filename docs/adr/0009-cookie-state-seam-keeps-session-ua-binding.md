# Cookie state seam keeps session UA binding

Status: accepted

The shared Cookie state seam intentionally carries session UA binding
(`ua` / `set_session_ua` with default no-ops). Stealth-acquired cookies and the
UA that earned them are one session concept shared to the HTTP fast path;
narrowing the seam would split one concept across two references and add cfg
noise for no behavioral gain. Rejected alternative: removing the UA methods
from `CookieJar` and passing `CfCookieJar` separately to Stealth.
