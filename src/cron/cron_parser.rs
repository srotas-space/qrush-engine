// /src/cron/cron_parser.rs
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use std::collections::HashSet;
use std::str::FromStr;

/// Full-featured cron parser (no external cron crate).
/// Supports:
/// - 6-field:  `sec  min  hour  dom  mon  dow`
/// - 5-field:  `min  hour  dom  mon  dow`   (auto-seconds = 0)
///
/// Tokens per field:
/// ```text
/// *         -> any
/// a         -> exact
/// a,b,c     -> list
/// a-b       -> range inclusive
/// */n       -> step over full range
/// a-b/n     -> stepped range
/// Names:
///   Months:  JAN..DEC
///   Weekdays: SUN,MON,TUE,WED,THU,FRI,SAT  (0/7 = SUN)
/// ```
pub struct CronParser;

#[derive(Debug, Clone)]
struct CronSpec {
    sec:  Field,
    min:  Field,
    hour: Field,
    dom:  FieldDomDow, // day-of-month (1..31) with "any" info
    mon:  Field,
    dow:  FieldDomDow, // day-of-week (0..6, 0/7 = Sun) with "any" info
}

#[derive(Debug, Clone)]
struct Field {
    allowed: HashSet<u32>, // empty => Any
    min: u32,
    max: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FieldDomDow {
    allowed: HashSet<u32>, // empty => Any
    min: u32,
    max: u32,
    any: bool,
}

impl CronParser {
    /// `cron_expr`: 5 or 6 fields (see header).
    /// `from_utc` : baseline (exclusive) instant in UTC; next > from returned.
    /// `tz_str`   : IANA timezone string, e.g. "UTC", "Asia/Kolkata".
    pub fn next_execution(cron_expr: &str, from_utc: DateTime<Utc>, tz_str: &str) -> Result<DateTime<Utc>> {
        let tz: Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);

        // Normalize to 6 fields if 5 are provided (insert seconds=0 at start)
        let expr = normalize_to_six(cron_expr);
        let spec = CronSpec::parse(&expr)
            .with_context(|| format!("Invalid cron expression: {}", cron_expr))?;

        // Work in local tz for correct wall-clock semantics
        // Start strictly AFTER 'from'
        let mut dt = from_utc.with_timezone(&tz) + chrono::Duration::seconds(1);

        // Hard upper bound to avoid infinite loops: advance up to 5 years
        let end_limit = dt + chrono::Duration::days(366 * 5);

        // Outer loop: year-month-day alignment
        loop {
            if dt > end_limit {
                return Err(anyhow!("Could not find next occurrence within 5 years"));
            }

            // 1) MONTH
            if !spec.mon.matches(dt.month()) {
                if let Some(next_m) = spec.mon.next_ge(dt.month()) {
                    // same year
                    if next_m != dt.month() {
                        // bump month, reset lower units
                        dt = set_ymd_hms(&tz, dt.year(), next_m, 1, 0, 0, 0)?;
                    }
                } else {
                    // move to earliest allowed month next year
                    let first_m = spec.mon.first().unwrap_or(1);
                    dt = set_ymd_hms(&tz, dt.year() + 1, first_m, 1, 0, 0, 0)?;
                }
            }

            // 2) DAY (DOM/DOW with OR)
            if !spec.matches_day(&dt) {
                // advance day by 1 until matching day (or month/year rolls)
                dt = dt + chrono::Duration::days(1);
                dt = set_hms(&tz, dt, 0, 0, 0)?;
                continue; // re-check month/day constraints on new date
            }

            // 3) HOUR
            if !spec.hour.matches(dt.hour()) {
                if let Some(next_h) = spec.hour.next_ge(dt.hour()) {
                    dt = set_hms(&tz, dt, next_h, 0, 0)?;
                } else {
                    // Next allowed hour is in next day
                    dt = dt + chrono::Duration::days(1);
                    dt = set_hms(&tz, dt, spec.hour.first().unwrap_or(0), 0, 0)?;
                    continue; // day may change -> re-run month/day checks
                }
            }

            // 4) MINUTE
            if !spec.min.matches(dt.minute()) {
                if let Some(next_min) = spec.min.next_ge(dt.minute()) {
                    dt = set_hms(&tz, dt, dt.hour(), next_min, 0)?;
                } else {
                    // bump hour
                    if let Some(next_h) = spec.hour.next_gt(dt.hour()) {
                        dt = set_hms(&tz, dt, next_h, spec.min.first().unwrap_or(0), 0)?;
                    } else {
                        // next day at first hour/min
                        dt = dt + chrono::Duration::days(1);
                        dt = set_hms(
                            &tz,
                            dt,
                            spec.hour.first().unwrap_or(0),
                            spec.min.first().unwrap_or(0),
                            0,
                        )?;
                    }
                    continue; // hour/day may change -> re-check
                }
            }

            // 5) SECOND
            if !spec.sec.matches(dt.second()) {
                if let Some(next_s) = spec.sec.next_ge(dt.second()) {
                    dt = set_hms(&tz, dt, dt.hour(), dt.minute(), next_s)?;
                } else {
                    // bump minute
                    if let Some(next_min) = spec.min.next_gt(dt.minute()) {
                        dt = set_hms(&tz, dt, dt.hour(), next_min, spec.sec.first().unwrap_or(0))?;
                    } else if let Some(next_h) = spec.hour.next_gt(dt.hour()) {
                        dt = set_hms(&tz, dt, next_h, spec.min.first().unwrap_or(0), spec.sec.first().unwrap_or(0))?;
                    } else {
                        // next day at first hour/min/sec
                        dt = dt + chrono::Duration::days(1);
                        dt = set_hms(
                            &tz,
                            dt,
                            spec.hour.first().unwrap_or(0),
                            spec.min.first().unwrap_or(0),
                            spec.sec.first().unwrap_or(0),
                        )?;
                    }
                    continue; // minute/hour/day may change -> re-check
                }
            }

            // All constraints satisfied
            return Ok(dt.with_timezone(&Utc));
        }
    }
}

// --------- CronSpec parsing & matching ----------

impl CronSpec {
    fn parse(expr6: &str) -> Result<Self> {
        let parts: Vec<&str> = expr6.split_whitespace().collect();
        if parts.len() != 6 {
            return Err(anyhow!("Expected 6 fields: sec min hour dom mon dow"));
        }

        let sec  = Field::parse(parts[0], 0, 59, None, false)?;
        let min  = Field::parse(parts[1], 0, 59, None, false)?;
        let hour = Field::parse(parts[2], 0, 23, None, false)?;
        let dom  = FieldDomDow::parse_dom(parts[3])?;
        let mon  = Field::parse(parts[4], 1, 12, Some(&month_name_map()), false)?;
        let dow  = FieldDomDow::parse_dow(parts[5])?;

        Ok(Self { sec, min, hour, dom, mon, dow })
    }

    /// DOM/DOW OR logic:
    /// - If both are Any => accept any day
    /// - Else day is valid if (DOM matches) OR (DOW matches)
    fn matches_day(&self, dt: &DateTime<Tz>) -> bool {
        let dom_any = self.dom.any;
        let dow_any = self.dow.any;

        let dom_match = self.dom.matches_dom(dt.day());
        let dow_match = self.dow.matches_dow(dt.weekday().num_days_from_sunday()); // 0=Sun..6=Sat

        match (dom_any, dow_any) {
            (true,  true)  => true,
            (false, true)  => dom_match,
            (true,  false) => dow_match,
            (false, false) => dom_match || dow_match,
        }
    }
}

impl Field {
    fn parse(token: &str, min: u32, max: u32, names: Option<&std::collections::HashMap<&'static str, u32>>, is_dow: bool) -> Result<Self> {
        let mut allowed = HashSet::new();

        // Empty or "*" => Any
        if token.trim() == "*" {
            return Ok(Self { allowed, min, max });
        }

        for part in token.split(',') {
            let part = part.trim();
            if part.is_empty() { continue; }

            // Handle names
            let mut part = if let Some(map) = names {
                // replace names with numbers (case-insensitive)
                let upper = part.to_ascii_uppercase();
                if let Some(&num) = map.get(upper.as_str()) {
                    num.to_string()
                } else {
                    part.to_string()
                }
            } else {
                part.to_string()
            };

            // For DOW: allow 7 => 0 (Sunday)
            if is_dow && part == "7" {
                part = "0".to_string();
            }

            // Step forms: "*/n" or "a-b/n"
            if let Some((lhs, step_s)) = part.split_once('/') {
                let step = parse_u(lhs, step_s, min, max)?; // parse step, lhs can be "*" or "a-b"
                if lhs == "*" {
                    for v in (min..=max).step_by(step as usize) {
                        allowed.insert(v);
                    }
                } else if let Some((a_s, b_s)) = lhs.split_once('-') {
                    let a = parse_num(a_s, min, max, names, is_dow)?;
                    let b = parse_num(b_s, min, max, names, is_dow)?;
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    for v in (lo..=hi).step_by(step as usize) {
                        allowed.insert(v);
                    }
                } else {
                    return Err(anyhow!("Invalid stepped token '{}'", part));
                }
                continue;
            }

            // "*/n"
            if let Some(step_s) = part.strip_prefix("*/") {
                let step: u32 = step_s.parse().context("Invalid step")?;
                for v in (min..=max).step_by(step as usize) {
                    allowed.insert(v);
                }
                continue;
            }

            // "a-b"
            if let Some((a_s, b_s)) = part.split_once('-') {
                let a = parse_num(a_s, min, max, names, is_dow)?;
                let b = parse_num(b_s, min, max, names, is_dow)?;
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                for v in lo..=hi {
                    allowed.insert(v);
                }
                continue;
            }

            // single number
            let n = parse_num(&part, min, max, names, is_dow)?;
            allowed.insert(n);
        }

        Ok(Self { allowed, min, max })
    }

    #[inline]
    fn matches(&self, v: u32) -> bool {
        if self.allowed.is_empty() { return true; }
        self.allowed.contains(&v)
    }

    #[inline]
    fn first(&self) -> Option<u32> {
        if self.allowed.is_empty() { return Some(self.min); }
        self.allowed.iter().cloned().min()
    }

    #[inline]
    fn next_ge(&self, v: u32) -> Option<u32> {
        if self.allowed.is_empty() {
            if v < self.min { return Some(self.min); }
            if v > self.max { return None; }
            return Some(v);
        }
        let mut cand: Option<u32> = None;
        for &x in &self.allowed {
            if x >= v {
                cand = Some(match cand {
                    Some(c) => c.min(x),
                    None => x,
                });
            }
        }
        if cand.is_none() {
            // wrap not allowed here; caller handles carry
        }
        cand
    }

    #[inline]
    fn next_gt(&self, v: u32) -> Option<u32> {
        if self.allowed.is_empty() {
            if v < self.max { return Some(v + 1); }
            return None;
        }
        let mut cand: Option<u32> = None;
        for &x in &self.allowed {
            if x > v {
                cand = Some(match cand {
                    Some(c) => c.min(x),
                    None => x,
                });
            }
        }
        cand
    }
}

impl FieldDomDow {
    fn parse_dom(token: &str) -> Result<Self> {
        let base = Field::parse(token, 1, 31, None, false)?;
        Ok(Self { any: base.allowed.is_empty(), allowed: base.allowed, min: 1, max: 31 })
    }
    fn parse_dow(token: &str) -> Result<Self> {
        let base = Field::parse(token, 0, 6, Some(&weekday_name_map()), true)?;
        Ok(Self { any: base.allowed.is_empty(), allowed: base.allowed, min: 0, max: 6 })
    }
    #[inline]
    fn matches_dom(&self, day: u32) -> bool {
        if self.any { return true; }
        self.allowed.contains(&day)
    }
    #[inline]
    fn matches_dow(&self, dow0sun: u32) -> bool {
        if self.any { return true; }
        self.allowed.contains(&dow0sun)
    }
}

// --------- helpers ----------

fn normalize_to_six(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    match parts.len() {
        5 => format!("0 {}", expr.trim()),
        _ => expr.trim().to_string(),
    }
}

fn set_ymd_hms(tz: &Tz, y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> Result<DateTime<Tz>> {
    tz.with_ymd_and_hms(y, m, d, h, min, s)
        .single()
        .ok_or_else(|| anyhow!("Invalid local time (DST gap/overlap): {y}-{m}-{d} {h}:{min}:{s}"))
}

fn set_hms(tz: &Tz, dt: DateTime<Tz>, h: u32, m: u32, s: u32) -> Result<DateTime<Tz>> {
    set_ymd_hms(tz, dt.year(), dt.month(), dt.day(), h, m, s)
}

fn parse_u(lhs: &str, step: &str, _min: u32, _max: u32) -> Result<usize> {
    if !lhs.is_empty() && lhs != "*" && !lhs.contains('-') {
        return Err(anyhow!("Invalid stepped lhs '{}'", lhs));
    }
    let st: u32 = step.parse().context("Invalid step value")?;
    if st == 0 { return Err(anyhow!("Step must be > 0")); }
    Ok(st as usize)
}

fn parse_num(token: &str, min: u32, max: u32, names: Option<&std::collections::HashMap<&'static str, u32>>, is_dow: bool) -> Result<u32> {
    let t = token.trim();
    // names (already handled in Field::parse for ranges/steps/lists), but single could still be a name
    if let Some(map) = names {
        let up = t.to_ascii_uppercase();
        if let Some(&n) = map.get(up.as_str()) {
            return Ok(n);
        }
    }
    let mut n: u32 = u32::from_str(t).context(format!("Invalid number '{}'", t))?;
    if is_dow && n == 7 { n = 0; } // 7 => 0 (Sunday)
    if n < min || n > max {
        return Err(anyhow!("Value {} out of range {}..{}", n, min, max));
    }
    Ok(n)
}

fn month_name_map() -> std::collections::HashMap<&'static str, u32> {
    use std::iter::FromIterator;
    std::collections::HashMap::from_iter([
        ("JAN", 1), ("FEB", 2), ("MAR", 3), ("APR", 4), ("MAY", 5), ("JUN", 6),
        ("JUL", 7), ("AUG", 8), ("SEP", 9), ("OCT", 10), ("NOV", 11), ("DEC", 12),
    ])
}

fn weekday_name_map() -> std::collections::HashMap<&'static str, u32> {
    use std::iter::FromIterator;
    // 0=SUN .. 6=SAT
    std::collections::HashMap::from_iter([
        ("SUN", 0), ("MON", 1), ("TUE", 2), ("WED", 3),
        ("THU", 4), ("FRI", 5), ("SAT", 6),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_minutely_5field_defaults_seconds0() {
        let from = Utc.with_ymd_and_hms(2025, 9, 26, 10, 0, 10).unwrap();
        let tz = "UTC";
        let next = CronParser::next_execution("*/1 * * * *", from, tz).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2025, 9, 26, 10, 1, 0).unwrap());
    }

    #[test]
    fn test_every_5_minutes_6field() {
        let tz = "Asia/Kolkata";
        let from = Utc.with_ymd_and_hms(2025, 9, 26, 10, 2, 30).unwrap();
        let next = CronParser::next_execution("0 */5 * * * *", from, tz).unwrap();
        assert!(next > from);
    }

    #[test]
    fn test_named_month_and_weekday() {
        let tz = "UTC";
        let from = Utc.with_ymd_and_hms(2025, 1, 30, 23, 59, 59).unwrap();
        // First MON in FEB at 09:00:00
        let next = CronParser::next_execution("0 0 9 1-7 FEB MON", from, tz).unwrap();
        assert!(next > from);
    }

    #[test]
    fn test_dow_sunday_0_or_7() {
        let tz = "UTC";
        let from = Utc.with_ymd_and_hms(2025, 9, 26, 10, 0, 0).unwrap(); // Friday
        let n0 = CronParser::next_execution("0 0 * * * 0", from, tz).unwrap();
        let n7 = CronParser::next_execution("0 0 * * * 7", from, tz).unwrap();
        assert_eq!(n0, n7);
    }
}
