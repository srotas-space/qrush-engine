# QRush Engine — Client Application Integration Guide

This document shows how to integrate **QRush Engine** in a client application and how to run the worker runtime.

> This guide does **not** assume any specific framework, but includes a complete **Actix-Web** example.

---

## What QRush Engine Does

QRush Engine provides:

- worker pools (per-queue concurrency + priority)
- delayed jobs processing
- cron scheduler
- optional metrics/routes integration into your app (Actix-Web scope)

Your client application can use **either** of these modes:

### Mode A — Embedded (App boots QRush Engine internally)
- Your app process initializes queues + cron + worker pools.
- Useful for local dev, small deployments, or “single binary” setups.
- You can mount QRush Engine routes under your app.

### Mode B — Separate Worker Runtime (Recommended at scale)
- Your app runs normally.
- A dedicated `qrush-engine` process runs workers.
- Your app only enqueues jobs and exposes APIs.

Both modes use the **same job definitions** and **same registration** patterns.

---

## Dependency

Add to your client application:

```toml
[dependencies]
qrush-engine = "0.6"
dotenvy = "0.15"
actix-web = "4"
```

---

## Define a Job (Factory Handler Pattern)

Example job: `NotifyUser`

```rust
use qrush_engine::job::Job;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use anyhow::{Result, Error};
use futures::future::{BoxFuture, FutureExt};

#[derive(Debug, Serialize, Deserialize)]
pub struct NotifyUser {
    pub user_id: String,
    pub message: String,
}

#[async_trait]
impl Job for NotifyUser {
    fn name(&self) -> &'static str { "NotifyUser" }
    fn queue(&self) -> &'static str { "default" }

    async fn perform(&self) -> Result<()> {
        println!("📬 NotifyUser: '{}' → {}", self.message, self.user_id);
        Ok(())
    }

    async fn on_error(&self, err: &Error) {
        eprintln!("❌ NotifyUser error: {:?}", err);
    }
}

impl NotifyUser {
    pub fn name() -> &'static str { "notify_user" }

    // Factory handler: payload JSON string -> Box<dyn Job>
    pub fn handler(payload: String) -> BoxFuture<'static, Result<Box<dyn Job>>> {
        async move {
            let job: NotifyUser = serde_json::from_str(&payload)?;
            Ok(Box::new(job) as Box<dyn Job>)
        }
        .boxed()
    }
}
```

---

## Define a Cron Job (Optional)

A cron job is typically **also a normal Job**, so it can be enqueued manually too.

```rust
use qrush_engine::cron::cron_job::CronJob;

impl CronJob for DailyReportJob {
    fn cron_id(&self) -> &'static str { "daily_report_job" }
    fn name(&self) -> &'static str { "Daily Report Job" }
    fn queue(&self) -> &'static str { "default" }
    fn cron_expression(&self) -> &'static str { "0 9 * * *" } // every day 09:00
    fn timezone(&self) -> &'static str { "Asia/Kolkata" }
    fn enabled(&self) -> bool { true }
}
```

---

## Client App: Organize QRush Engine Code

Recommended layout:

```
src/
  qrushesengines/
    mod.rs
    initiate.rs
    jobs/
      mod.rs
      notify_user.rs
    crons/
      mod.rs
      daily_report_job.rs
  main.rs
```

---

## `src/qrushesengines/mod.rs`

```rust
pub mod initiate;
pub mod jobs;
pub mod crons;
```

---

## `src/qrushesengines/initiate.rs`

This is the single place where you register **all jobs** + **cron jobs**.

```rust
use anyhow::Result;
use qrush_engine::registry::register_job;
use qrush_engine::cron::cron_scheduler::CronScheduler;

use crate::qrushesengines::jobs::notify_user::NotifyUser;
use crate::qrushesengines::crons::daily_report_job::DailyReportJob;

pub struct QrushEngine;

impl QrushEngine {
    pub async fn initialize(_basic_auth: Option<qrush_engine::config::QrushBasicAuthConfig>) -> Result<()> {
        // Register jobs
        register_job(NotifyUser::name(), NotifyUser::handler);

        // Register cron jobs (stores meta in Redis + schedules next tick)
        let daily = DailyReportJob { report_type: "daily".to_string() };
        CronScheduler::register_cron_job(daily).await?;

        Ok(())
    }

    /// Optional: create a worker config object to attach to actix app data.
    pub fn setup_worker_sync() -> QrushEngineWorkerConfig {
        QrushEngineWorkerConfig {
            worker_id: nanoid::nanoid!(),
            initialized_at: std::time::SystemTime::now(),
            integration_mode: "engine".to_string(),
        }
    }

    /// Optional: mount QRush Engine routes (metrics etc.) into your Actix app.
    pub fn configure_routes(cfg: &mut actix_web::web::ServiceConfig) {
        qrush_engine::routes::metrics_route::qrush_metrics_routes(cfg);
    }
}

#[derive(Clone, Debug)]
pub struct QrushEngineWorkerConfig {
    pub worker_id: String,
    pub initialized_at: std::time::SystemTime,
    pub integration_mode: String,
}
```

---

# Actix-Web Example (Full `main.rs`)

This includes exactly the usage you asked for:

```rust
use crate::qrushesengines::initiate::QrushEngine;

QrushEngine::initialize(None).await;

let qrush_engine_worker_config = QrushEngine::setup_worker_sync();

.service(
    web::scope("/qrush-engine")
        .configure(|cfg| QrushEngine::configure_routes(cfg))
)
```

Complete `main.rs`:

```rust
use actix_web::{web, App, HttpServer};
use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

use qrush_engine::config::QueueConfig;

mod qrushesengines;
use crate::qrushesengines::initiate::QrushEngine;

#[actix_web::main]
async fn main() -> Result<()> {
    // Load .env if present
    let _ = dotenvy::dotenv();

    // Logs
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // 1) Register jobs + cron metadata
    QrushEngine::initialize(None).await?;

    // 2) Start worker pools inside this process (Embedded mode)
    //    If you're running a separate `qrush-engine` process, DO NOT call this here.
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let queues = vec![
        QueueConfig::new("default", 10, 1),
        QueueConfig::new("critical", 25, 0),
    ];

    QueueConfig::initialize(redis_url, queues).await?;

    // 3) Optional worker config (your style)
    let qrush_engine_worker_config = QrushEngine::setup_worker_sync();

    info!("Starting Actix server…");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(qrush_engine_worker_config.clone()))
            // Mount QRush Engine routes
            .service(
                web::scope("/qrush-engine")
                    .configure(|cfg| QrushEngine::configure_routes(cfg))
            )
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await?;

    Ok(())
}
```

---

## Separate Worker Runtime

Run workers as a separate process:

```bash
REDIS_URL="redis://127.0.0.1:6379" RUST_LOG="info,qrush_engine=debug" qrush-engine   --queues default:10,critical:25   --shutdown-grace-secs 10
```

If you use a separate worker runtime, keep only this in your app boot:

```rust
QrushEngine::initialize(None).await?;
```

and remove `QueueConfig::initialize(...)` from the app process.

---

## CLI Commands

### Basic

```bash
qrush-engine --queues default:10
```

### Multiple queues

```bash
qrush-engine --queues critical:25,default:10,low:2
```

### Graceful shutdown

```bash
qrush-engine --queues default:10 --shutdown-grace-secs 10
```

### Logging

```bash
RUST_LOG="info,qrush_engine=debug" qrush-engine --queues default:10
```

---

## Environment Variables

- `QRUSH_REDIS_URL` (preferred)
- `REDIS_URL` (fallback)
- `QRUSH_BASIC_AUTH` (optional `user:pass`)
- `RUST_LOG`

Example `.env`:

```env
REDIS_URL=redis://127.0.0.1:6379
QRUSH_BASIC_AUTH=qrush:password
RUST_LOG=info
```

---

## Notes & Best Practices

- Always register jobs using `register_job(Job::name(), Job::handler)` before running workers.
- Prefer running worker runtime as a separate process for scaling and isolation.
- Run multiple worker processes to increase throughput.
- Use separate queues for critical vs background workloads.
