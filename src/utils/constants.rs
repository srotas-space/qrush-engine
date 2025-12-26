// /Users/snm/ws/xsnm/ws/crates/qrush-engine/src/utils/constants.rs

// ---------------------------------------------------------
// General
// ---------------------------------------------------------
pub const MAX_RETRIES: usize = 3;
pub const DEFAULT_PAGE: usize = 1;
pub const DEFAULT_LIMIT: usize = 10;

// ---------------------------------------------------------
// Redis Keys - All keys use qrush_engine: prefix
// ---------------------------------------------------------

// Job and Queue Prefixes
pub const PREFIX_QUEUE: &str = "qrush_engine:queue";
pub const PREFIX_JOB: &str = "qrush_engine:job";

// Delayed Jobs
pub const DELAYED_JOBS_KEY: &str = "qrush_engine:delayed_jobs";

// Queues Management
pub const QUEUES_SET: &str = "qrush_engine:queues";
pub const QUEUE_CONFIG_PREFIX: &str = "qrush_engine:queue:config";

// Job Status Lists (per queue)
pub const SUCCESS_LIST_PREFIX: &str = "qrush_engine:success";
pub const FAILED_LIST_PREFIX: &str = "qrush_engine:failed";
pub const RETRY_LIST_PREFIX: &str = "qrush_engine:retry";

// Global Job Counters
pub const COUNTER_SUCCESS: &str = "qrush_engine:qrush:success";
pub const COUNTER_FAILED: &str = "qrush_engine:qrush:failed";
pub const COUNTER_TOTAL_JOBS: &str = "qrush_engine:qrush:total_jobs";

// Failed Jobs List
pub const FAILED_JOBS_LIST: &str = "qrush_engine:failed_jobs";

// Stats Keys (daily)
pub const STATS_JOBS_PREFIX: &str = "qrush_engine:stats:jobs";
pub const STATS_JOBS_FAILED_PREFIX: &str = "qrush_engine:stats:jobs";

// Logs (per queue)
pub const LOGS_PREFIX: &str = "qrush_engine:logs";

// Workers
pub const WORKER_PREFIX: &str = "qrush_engine:worker";

// Cron
pub const CRON_JOBS_KEY: &str = "qrush_engine:cron:jobs";              // HASH  id -> json(meta)
pub const CRON_SCHEDULE_KEY: &str = "qrush_engine:cron:schedule";      // ZSET  score=unix_ts, member=id
pub const CRON_JOBS_META_KEY: &str = "qrush_engine:cron:jobs:meta";    // HASH  id:enabled -> "0|1" (optional)

// ---------------------------------------------------------
// Cron Worker
// ---------------------------------------------------------
// NOTE: redis::Commands::zrangebyscore_limit expects `count` as `isize`
// but keeping it usize is fine; cast at call site.
pub const CLAIM_BATCH_LIMIT: usize = 200;
