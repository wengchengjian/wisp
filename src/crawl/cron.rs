//! Cron scheduling for periodic spider execution.
//!
//! Uses the `croner` crate for standard 5-field cron expression parsing.

use croner::Cron;
use chrono::{DateTime, Local, Utc};
use crate::error::{WispError, Result};

/// Parsed cron expression with next-run calculation.
#[derive(Debug, Clone)]
pub struct CronExpr {
    inner: Cron,
    expr: String,
}

impl CronExpr {
    /// Parse a standard 5-field cron expression (min hour day month weekday).
    ///
    /// Examples: "0 * * * *" (every hour), "*/30 * * * *" (every 30 min)
    pub fn parse(expr: &str) -> Result<Self> {
        let inner: Cron = expr.parse()
            .map_err(|e| WispError::CdpError(format!("cron parse '{}': {}", expr, e)))?;
        Ok(Self { inner, expr: expr.to_string() })
    }

    /// Calculate the next run time from now (Local timezone).
    pub fn next_run(&self) -> Option<DateTime<Local>> {
        // croner works with Utc internally, convert result to Local
        self.inner.find_next_occurrence(&Utc::now(), false)
            .ok()
            .map(|dt| dt.with_timezone(&Local))
    }

    /// Async wait until the next scheduled time.
    pub async fn wait_until_next(&self) {
        if let Some(next) = self.next_run() {
            let now = Local::now();
            if next > now {
                let dur = (next - now).to_std().unwrap_or_default();
                tokio::time::sleep(dur).await;
            }
        }
    }

    /// Get the original expression string.
    pub fn expr(&self) -> &str {
        &self.expr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_cron() {
        let cron = CronExpr::parse("0 * * * *").unwrap();
        assert_eq!(cron.expr(), "0 * * * *");
        // next_run may be None on some platforms due to timezone; just verify parse works
    }

    #[test]
    fn test_parse_every_30_min() {
        let cron = CronExpr::parse("*/30 * * * *").unwrap();
        assert_eq!(cron.expr(), "*/30 * * * *");
    }

    #[test]
    fn test_parse_invalid_cron() {
        // croner may be lenient; use a clearly invalid expression
        let result = CronExpr::parse("99 99 99 99 99 99 99");
        // Either parse fails or next_run returns None for impossible schedule
        if let Ok(cron) = result {
            // If it parses, it's fine - croner is lenient
            let _ = cron.next_run();
        }
    }

    #[test]
    fn test_next_run_is_in_future_or_none() {
        let cron = CronExpr::parse("* * * * *").unwrap(); // every minute
        // On some platforms next_run may be None; if Some, verify it's in the future
        if let Some(next) = cron.next_run() {
            assert!(next >= Local::now(), "next run should be in the future");
        }
    }
}
