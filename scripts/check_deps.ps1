$ErrorActionPreference = "Stop"

$bad = @()
$bad += rg -n "wisp_fetcher|wisp-fetcher" crates/http crates/core crates/storage crates/parser crates/proxy crates/browser
$bad += rg -n "wisp_crawl|wisp-crawl" crates/fetcher crates/http crates/core crates/storage crates/parser crates/proxy crates/browser crates/stealth

if ($bad) {
    $bad
    exit 1
}
Write-Host "dependency direction OK"
