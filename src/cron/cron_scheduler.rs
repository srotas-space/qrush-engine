// src/cron/cron_scheduler.rs
//
// Minimal cron scheduler:
// - register_cron_job(job): stores CronJobMeta in Redis + schedules next run in a ZSET.
// - start(): background loop that claims due cron jobs and enqueues their payload into the target queue.
//
// Redis keys:
// - qrush:cron:jobs      (HASH)  id -> json(meta)
// - qrush:cron:schedule  (ZSET)  score=unix_ts, member=cron_id
// - qrush:cron:jobs:meta (HASH)  id:enabled -> "0|1" (optional)

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{error, info, warn};

use crate::config::get_shutdown_notify;
use crate::cron::cron_job::{CronJob, CronJobMeta};
use crate::cron::cron_parser::CronParser;
use crate::queue;
use crate::utils::constants::{CLAIM_BATCH_LIMIT, CRON_JOBS_KEY, CRON_JOBS_META_KEY, CRON_SCHEDULE_KEY};
use crate::utils::rdconfig::get_redis_connection;

pub struct CronScheduler;

impl CronScheduler {
    /// Start the cron scheduler worker (runs forever until shutdown signal).
    pub async fn start() {
        tokio::spawn(async move {
            let shutdown = get_shutdown_notify();

            let mut tick = interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

            info!("CronScheduler loop started");

            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        info!("CronScheduler received shutdown");
                        break;
                    }
                    _ = tick.tick() => {
                        if let Err(e) = Self::tick_once().await {
                            warn!(err=?e, "CronScheduler tick failed");
                        }
                    }
                }
            }

            info!("CronScheduler loop exited");
        });
    }

    async fn tick_once() -> Result<()> {
        let now = Utc::now();
        let now_ts = now.timestamp();

        let mut conn = get_redis_connection().await?;

        // Claim due jobs (simple strategy: read, then remove).
        // For multi-engine concurrency, consider a Lua script + claim token.
        let due: Vec<String> = conn
            .zrangebyscore_limit(
                CRON_SCHEDULE_KEY,
                i64::MIN,
                now_ts,
                0,
                CLAIM_BATCH_LIMIT as isize,
            )
            .await
            .unwrap_or_default();

        if due.is_empty() {
            return Ok(());
        }

        // Remove claimed ids
        let _: i64 = conn.zrem(CRON_SCHEDULE_KEY, due.clone()).await.unwrap_or(0);

        for cron_id in due {
            if let Err(e) = Self::run_one(&mut conn, &cron_id, now).await {
                error!(cron_id=%cron_id, err=?e, "cron run failed");
            }
        }

        Ok(())
    }

    async fn run_one(
        conn: &mut redis::aio::MultiplexedConnection,
        cron_id: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let meta_json: Option<String> = conn.hget(CRON_JOBS_KEY, cron_id).await.ok();
        let Some(meta_json) = meta_json else { return Ok(()); };

        let mut meta: CronJobMeta = serde_json::from_str(&meta_json)?;

        if !meta.enabled {
            return Ok(());
        }

        // Enqueue as a normal job (payload already stored as JSON string)
        queue::enqueue_raw(&meta.queue, meta.payload.clone()).await?;

        meta.last_run = Some(now.to_rfc3339());

        // Compute next run
        let next = CronParser::next_execution(&meta.cron_expression, now, &meta.timezone)?;
        meta.next_run = next.to_rfc3339();

        // Persist meta + reschedule
        let meta_out = serde_json::to_string(&meta)?;
        let _: () = conn.hset(CRON_JOBS_KEY, cron_id, meta_out).await?;
        let _: () = conn.zadd(CRON_SCHEDULE_KEY, cron_id, next.timestamp()).await?;

        Ok(())
    }

    /// Register a cron job (stores payload + schedules next run).
    pub async fn register_cron_job<T>(job: T) -> Result<()>
    where
        T: CronJob + serde::Serialize + 'static,
    {
        let mut conn = get_redis_connection().await?;
        let now = Utc::now();

        if !job.enabled() {
            return Err(anyhow!("cron job '{}' is disabled", job.cron_id()));
        }

        let payload = serde_json::to_string(&job)?;
        let next = CronParser::next_execution(job.cron_expression(), now, job.timezone())?;

        let meta = CronJobMeta {
            id: job.cron_id().to_string(),
            name: job.name().to_string(),
            queue: job.queue().to_string(),
            cron_expression: job.cron_expression().to_string(),
            timezone: job.timezone().to_string(),
            enabled: job.enabled(),
            last_run: None,
            next_run: next.to_rfc3339(),
            created_at: now.to_rfc3339(),
            payload,
        };

        let meta_json = serde_json::to_string(&meta)?;

        let _: () = conn.hset(CRON_JOBS_KEY, &meta.id, meta_json).await?;
        let _: () = conn.zadd(CRON_SCHEDULE_KEY, &meta.id, next.timestamp()).await?;

        // Optional enabled marker
        let enabled_val = if meta.enabled { "1" } else { "0" };
        let _: () = conn
            .hset(CRON_JOBS_META_KEY, format!("{}:enabled", &meta.id), enabled_val)
            .await
            .unwrap_or(());

        info!(cron_id=%meta.id, next_run=%meta.next_run, "cron job registered");
        Ok(())
    }

    /// List cron jobs (for UI/debugging)
    pub async fn list_cron_jobs() -> Result<HashMap<String, CronJobMeta>> {
        let mut conn = get_redis_connection().await?;
        let map: HashMap<String, String> = conn.hgetall(CRON_JOBS_KEY).await.unwrap_or_default();

        let mut out = HashMap::new();
        for (k, v) in map {
            if let Ok(meta) = serde_json::from_str::<CronJobMeta>(&v) {
                out.insert(k, meta);
            }
        }
        Ok(out)
    }

    /// Enable/disable an existing cron job.
    pub async fn toggle_cron_job(job_id: String, enabled: bool) -> Result<()> {
        let mut conn = get_redis_connection().await?;

        // Update the main meta record (if it exists)
        if let Ok(Some(meta_json)) = conn.hget::<_, _, Option<String>>(CRON_JOBS_KEY, &job_id).await {
            let mut meta: CronJobMeta = serde_json::from_str(&meta_json)?;
            meta.enabled = enabled;

            if enabled {
                let now = Utc::now();
                let next = CronParser::next_execution(&meta.cron_expression, now, &meta.timezone)?;
                meta.next_run = next.to_rfc3339();

                let meta_out = serde_json::to_string(&meta)?;
                let _: () = conn.hset(CRON_JOBS_KEY, &job_id, meta_out).await?;
                let _: () = conn.zadd(CRON_SCHEDULE_KEY, &job_id, next.timestamp()).await?;
            } else {
                // disable: remove from schedule
                let meta_out = serde_json::to_string(&meta)?;
                let _: () = conn.hset(CRON_JOBS_KEY, &job_id, meta_out).await?;
                let _: i64 = conn.zrem(CRON_SCHEDULE_KEY, &job_id).await.unwrap_or(0);
            }
        }

        // Update marker (optional)
        let enabled_val = if enabled { "1" } else { "0" };
        let _: () = conn
            .hset(CRON_JOBS_META_KEY, format!("{}:enabled", &job_id), enabled_val)
            .await?;

        Ok(())
    }

    /// Delete a cron job completely (meta + schedule).
    pub async fn delete_cron_job(job_id: String) -> Result<()> {
        let mut conn = get_redis_connection().await?;

        let _: i64 = conn.zrem(CRON_SCHEDULE_KEY, &job_id).await.unwrap_or(0);
        let _: i64 = conn.hdel(CRON_JOBS_KEY, &job_id).await.unwrap_or(0);
        let _: i64 = conn
            .hdel(CRON_JOBS_META_KEY, format!("{}:enabled", &job_id))
            .await
            .unwrap_or(0);

        Ok(())
    }

    /// Run a cron job immediately (best-effort).
    pub async fn run_now(job_id: String) -> Result<()> {
        let mut conn = get_redis_connection().await?;

        let meta_json: Option<String> = conn.hget(CRON_JOBS_KEY, &job_id).await.ok();
        let Some(meta_json) = meta_json else {
            return Err(anyhow!("cron job not found: {}", job_id));
        };

        let meta: CronJobMeta = serde_json::from_str(&meta_json)?;
        if !meta.enabled {
            return Err(anyhow!("cron job is disabled: {}", job_id));
        }

        queue::enqueue_raw(&meta.queue, meta.payload.clone()).await?;
        Ok(())
    }
}
