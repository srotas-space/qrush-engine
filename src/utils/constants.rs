// /Users/snm/ws/xsnm/ws/crates/qrush-engine/src/utils/constants.rs

// ---------------------------------------------------------
// General
// ---------------------------------------------------------
pub const MAX_RETRIES: usize = 3;
pub const DEFAULT_PAGE: usize = 1;
pub const DEFAULT_LIMIT: usize = 10;

// ---------------------------------------------------------
// Redis Keys
// ---------------------------------------------------------
pub const DELAYED_JOBS_KEY: &str = "qrush:delayed_jobs";

// Prefixes
pub const PREFIX_QUEUE: &str = "qrush:queue";
pub const PREFIX_JOB: &str = "qrush:job";

// Cron
pub const CRON_JOBS_KEY: &str = "qrush:cron:jobs";              // HASH  id -> json(meta)
pub const CRON_SCHEDULE_KEY: &str = "qrush:cron:schedule";      // ZSET  score=unix_ts, member=id
pub const CRON_JOBS_META_KEY: &str = "qrush:cron:jobs:meta";    // HASH  id:enabled -> "0|1" (optional)

// ---------------------------------------------------------
// Cron Worker
// ---------------------------------------------------------
// NOTE: redis::Commands::zrangebyscore_limit expects `count` as `isize`
// but keeping it usize is fine; cast at call site.
pub const CLAIM_BATCH_LIMIT: usize = 200;
