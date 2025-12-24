// src/cron/cron_parser.rs

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;

pub struct CronParser;

impl CronParser {
    /// Compute next execution time for a cron expression.
    ///
    /// - `cron_expr`: standard 5/6-field cron expression supported by `cron` crate
    /// - `from_utc`: "now" moment as UTC
    /// - `timezone`: tz name like "UTC", "Asia/Kolkata"
    ///
    /// Returns next run time in UTC.
    pub fn next_execution(cron_expr: &str, from_utc: DateTime<Utc>, timezone: &str) -> Result<DateTime<Utc>> {
        let schedule = Schedule::from_str(cron_expr)
            .map_err(|e| anyhow!("invalid cron expression '{}': {:?}", cron_expr, e))?;

        let tz: Tz = timezone
            .parse()
            .map_err(|_| anyhow!("invalid timezone '{}'", timezone))?;

        let from_tz = from_utc.with_timezone(&tz);

        // IMPORTANT:
        // `after()` is on Schedule, not on ScheduleIterator.
        let next_tz = schedule
            .after(&from_tz)
            .next()
            .ok_or_else(|| anyhow!("no upcoming cron occurrence for expression '{}'", cron_expr))?;

        Ok(next_tz.with_timezone(&Utc))
    }

    /// Convenience: validate cron expr
    pub fn validate(cron_expr: &str) -> Result<()> {
        let _ = Schedule::from_str(cron_expr)
            .map_err(|e| anyhow!("invalid cron expression '{}': {:?}", cron_expr, e))?;
        Ok(())
    }

    /// Parse a tz string into chrono_tz::Tz
    pub fn parse_tz(timezone: &str) -> Result<Tz> {
        timezone
            .parse::<Tz>()
            .map_err(|_| anyhow!("invalid timezone '{}'", timezone))
    }

    /// Helper: convert UTC timestamp to tz DateTime
    pub fn to_tz(ts_utc: DateTime<Utc>, tz: Tz) -> DateTime<Tz> {
        tz.from_utc_datetime(&ts_utc.naive_utc())
    }
}
