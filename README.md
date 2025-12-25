# QRush Engine

[![Crates.io](https://img.shields.io/crates/v/qrush-engine)](https://crates.io/crates/qrush-engine)
[![Documentation](https://docs.rs/qrush-engine/badge.svg)](https://docs.rs/qrush-engine)
[![License](https://img.shields.io/crates/l/qrush-engine)](LICENSE)
[![Downloads](https://img.shields.io/crates/d/qrush-engine)](https://crates.io/crates/qrush-engine)
[![Recent Downloads](https://img.shields.io/crates/dr/qrush-engine)](https://crates.io/crates/qrush-engine)

![QRushEngine](https://srotas-suite-space.s3.ap-south-1.amazonaws.com/sqrush.png)



# QRush Engine Integration Guide (Actix Web)



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
  qengines/
    mod.rs
    initiate.rs
    jobs/
      mod.rs
      notify_user_job.rs
    crons/
      mod.rs
      daily_report_job.rs
  main.rs
```

---

## `src/qengines/mod.rs`

```rust
pub mod initiate;
pub mod jobs;
pub mod crons;
```

---


## `src/qengines/initiate.rs`

```rust
use actix_web::web;
use std::sync::Arc;
use tokio::sync::{Notify, OnceCell};
use qrush_engine::config::{QueueConfig, QUEUE_INITIALIZED, set_basic_auth, QrushBasicAuthConfig};
use qrush_engine::registry::register_job;
use qrush_engine::cron::cron_scheduler::CronScheduler;
use qrush_engine::routes::metrics_route::qrush_metrics_routes;
use crate::qengines::jobs::notify_user_job::NotifyUserJob;
use crate::qengines::crons::daily_report_job::DailyReportJob;
use nanoid::nanoid;

// Integrated-specific initialization tracker
static QRUSH_INIT: OnceCell<Arc<Notify>> = OnceCell::const_new();

pub struct QrushEngine;

#[derive(Clone, Debug)]
pub struct QrushEngineWorkerConfig {
    pub worker_id: String,
    pub initialized_at: std::time::SystemTime,
    pub integration_mode: String,
}

impl QrushEngine {
    /// 🌍 GLOBAL initialization - call this ONCE in main.rs
    pub async fn initialize(basic_auth: Option<QrushBasicAuthConfig>) {
        // Check if already initialized globally
        if let Some(existing_notify) = QRUSH_INIT.get() {
            println!("QRush already initialized globally (integrated mode), waiting for completion...");
            existing_notify.notified().await;
            return;
        }

        let queue_notify = Arc::new(Notify::new());
        let _ = QRUSH_INIT.set(queue_notify.clone());

        println!("🌍 Starting GLOBAL QRush initialization (INTEGRATED mode)...");

        let basic_auth = basic_auth.or_else(|| {
            std::env::var("QRUSH_ENGINE_BASIC_AUTH").ok().and_then(|auth| {
                let parts: Vec<&str> = auth.splitn(2, ':').collect();
                if parts.len() == 2 {
                    Some(QrushBasicAuthConfig {
                        username: parts[0].to_string(),
                        password: parts[1].to_string(),
                    })
                } else {
                    None
                }
            })
        });

        let _ = set_basic_auth(basic_auth);
        let _ = QUEUE_INITIALIZED.set(queue_notify.clone());

        // Register jobs globally
        println!("Registering jobs for integrated mode...");
        register_job(NotifyUserJob::name(), NotifyUserJob::handler);
        register_job(DailyReportJob::name(), DailyReportJob::handler);

        // Initialize queues in background
        tokio::spawn({
            let queue_notify = queue_notify.clone();
            async move {
                let redis_url = std::env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

                println!("Connecting to Redis: {}", redis_url);

                let queues = vec![
                    QueueConfig::new("default", 5, 1),
                    QueueConfig::new("critical", 10, 0),
                    QueueConfig::new("integrated", 3, 2), // Special queue for integrated mode
                ];

                if let Err(err) = QueueConfig::initialize(redis_url, queues).await {
                    eprintln!("Failed to initialize qrush (integrated): {:?}", err);
                } else {
                    println!("QRush queues started successfully (integrated mode)");
                }
                
                queue_notify.notify_waiters();
            }
        });
        // Wait for queue initialization
        queue_notify.notified().await;
        println!("🚀 Global queue initialization complete (integrated mode)");
        // Register cron jobs after queues are ready
        Self::register_cron_jobs().await;
        println!("🎯 GLOBAL QRush initialization complete (INTEGRATED mode)!");
    }

    /// Register cron jobs for integrated mode
    async fn register_cron_jobs() {
        println!("Registering integrated mode cron jobs...");
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        let daily_report_job = DailyReportJob {
            report_type: "integrated_report".to_string(),
        };
        
        match CronScheduler::register_cron_job(daily_report_job).await {
            Ok(_) => {
                println!("DailyReportJob Cron Job registered for integrated mode");
            }
            Err(e) => {
                println!("Failed to register integrated Cron Job: {:?}", e);
            }
        }
    }


    /*-----------------------------------------------------------
    Utilities
    ------------------------------------------------------------*/

    // Generate nano uniq id
    pub fn gen_uniq_nanoid() -> String {
        nanoid!()
    }

    // WORKER setup - call this in each HttpServer::new()
    // used for debugging/monitoring purposes
    // fn test(qrush_worker_config: web::Data<QrushEngineWorkerConfig>)
    pub fn setup_worker_sync() -> QrushEngineWorkerConfig {
        let worker_id = Self::gen_uniq_nanoid();
        println!("Setting up QRush integrated worker: {}", worker_id);
        QrushEngineWorkerConfig {
            worker_id,
            initialized_at: std::time::SystemTime::now(),
            integration_mode: "integrated".to_string(),
        }
    }

    /// Get QRush metrics routes for integration into main app
    pub fn configure_routes(cfg: &mut web::ServiceConfig) {
        println!("🔧 Configuring integrated QRush routes...");
        qrush_metrics_routes(cfg);
    }

    /// Check if QRush is initialized
    pub fn is_initialized() -> bool {
        QRUSH_INIT.get().is_some()
    }
    /*-----------------------------------------------------------
    Utilities
    ------------------------------------------------------------*/
}
```

---


## `src/qengines/jobs/mod.rs`

```rust
pub mod notify_user_job;
```

---


## `src/qengines/jobs/notify_user_job.rs`

```rust
use qrush_engine::job::Job;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use anyhow::{Result, Error};
use futures::future::{BoxFuture, FutureExt};

#[derive(Debug, Serialize, Deserialize)]
pub struct NotifyUserJob {
    pub user_id: String,
    pub message: String,
}

#[async_trait]
impl Job for NotifyUserJob {
    fn name(&self) -> &'static str {
        "NotifyUserJob"
    }

    fn queue(&self) -> &'static str {
        "default"
    }

    async fn before(&self) -> Result<()> {
        println!("⏳ Before NotifyUserJob job for user: {}", self.user_id);
        Ok(())
    }

    async fn perform(&self) -> Result<()> {
        // Your code here
        println!("📬 Performing NotifyUserJob: '{}' to user {}", self.message, self.user_id);
        Ok(())
    }

    async fn after(&self) {
        println!("✅ After NotifyUserJob job for user: {}", self.user_id);
    }

    async fn on_error(&self, err: &Error) {
        eprintln!("❌ Error in NotifyUserJob job for user {}: {:?}", self.user_id, err);
    }

    async fn always(&self) {
        println!("🔁 Always block executed for NotifyUserJob job");
    }
}


impl NotifyUserJob {
    pub fn name() -> &'static str {
        "notify_user"
    }

    //  handler signature matching registry
    pub fn handler(payload: String) -> BoxFuture<'static, Result<Box<dyn Job>>> {
        async move {
            let job: NotifyUserJob = serde_json::from_str(&payload)?;
            Ok(Box::new(job) as Box<dyn Job>)
        }
        .boxed()
    }
}
```

---


## `src/qengines/crons/mod.rs`

```rust
pub mod daily_report_job;
```

---

## `src/qengines/crons/daily_report_job.rs`

```rust
use async_trait::async_trait;
use futures::future::BoxFuture;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use qrush_engine::job::Job;
use qrush_engine::cron::cron_job::CronJob;

#[derive(Debug, Serialize, Deserialize)]
pub struct DailyReportJob {
    pub report_type: String,
}

#[async_trait]
impl Job for DailyReportJob {
    async fn perform(&self) -> Result<()> {
        println!("Generating {} report...", self.report_type);
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        let text = format!("{} report generated successfully", self.report_type);
        send_slack_notification(&text).await?;
        println!("{} report generated successfully", self.report_type);
        Ok(())
    }

    fn name(&self) -> &'static str { "DailyReportJob" }

    fn queue(&self) -> &'static str { "default" }
}

#[async_trait]
impl CronJob for DailyReportJob {
    fn cron_expression(&self) -> &'static str {
        "0 * * * * *"
    }

    fn cron_id(&self) -> &'static str { "daily_report" }
}

impl DailyReportJob {
    pub fn name() -> &'static str { "DailyReportJob" }

    pub fn handler(payload: String) -> BoxFuture<'static, Result<Box<dyn Job>>> {
        Box::pin(async move {
            let job: DailyReportJob = serde_json::from_str(&payload)?;
            Ok(Box::new(job) as Box<dyn Job>)
        })
    }
}




async fn send_slack_notification(text: &str) -> Result<()> {
    use anyhow::Context;

    let webhook_url = std::env::var("SLACK_WEBHOOK_URL")
        .context("SLACK_WEBHOOK_URL not set")?;

    let client = reqwest::Client::new();
    let payload = serde_json::json!({ "text": text });

    let resp = client
        .post(&webhook_url)
        .json(&payload) // ✅ works because `json` feature is enabled
        .send()
        .await
        .context("Failed to send request to Slack webhook")?;

    Ok(())
}

```

---

# Actix-Web Example (Full `main.rs`)

This includes exactly the usage you asked for:

```rust
use crate::qengine::initiate::QrushEngine;

QrushEngine::initialize(None).await;

let qrush_engine_worker_config = QrushEngine::setup_worker_sync();

.service(
    web::scope("/qrush-engine")
        .configure(|cfg| QrushEngine::configure_routes(cfg))
)
```

Complete `main.rs`:

```rust
use actix_web::{web, App, HttpServer, HttpResponse, Responder, middleware::Logger};
use dotenv::dotenv;
use std::env;
use crate::qengines::initiate::QrushEngine;

mod qengines;



#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _ = dotenvy::dotenv();
    
    QrushEngine::initialize(None).await;
    
    HttpServer::new(move || {
        // Worker-specific setup - only enqueues jobs for this worker
        let qrush_engine_worker_config = QrushEngine::setup_worker_sync();
        

        App::new()
            .app_data(web::Data::new(qrush_engine_worker_config))
            .wrap(Logger::default())
            // Qrush engine metrics routes
            .service(
                web::scope("/qrush-engine")
                    .configure(|cfg| QrushEngine::configure_routes(cfg))
            )
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
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

- `QRUSH_ENGINE_REDIS_URL` (preferred)
- `REDIS_URL` (fallback)
- `QRUSH_ENGINE_BASIC_AUTH` (optional `user:pass`)
- `RUST_LOG`

Example `.env`:

```env
QRUSH_ENGINE_REDIS_URL=redis://127.0.0.1:6379
QRUSH_ENGINE_BASIC_AUTH=qrush:password
RUST_LOG=info
```

---

## Notes & Best Practices

- Always register jobs using `register_job(Job::name(), Job::handler)` before running workers.
- Prefer running worker runtime as a separate process for scaling and isolation.
- Run multiple worker processes to increase throughput.
- Use separate queues for critical vs background workloads.


---

## Common Cron Expressions

- `"0 * * * * *"` - Every minute
- `"0 */5 * * * *"` - Every 5 minutes  
- `"0 0 * * * *"` - Every hour
- `"0 0 0 * * *"` - Daily at midnight
- `"0 0 0 * * 1"` - Every Monday
- `"0 0 0 1 * *"` - First day of month

---

## Notes & tips

- **Scheduling**: `enqueue_in(job, delay_secs)` uses seconds (integer), matching your test app.
- **QueueConfig**: you used three queues (`default`, `critical`, `integrated`) with different concurrency/priority; tune as needed.
- **CronJobs**: implement both `Job` and `CronJob` traits, register with `CronScheduler::register_cron_job()` after queue initialization, supports standard cron expressions with 6-field format (sec min hour day month weekday).
- **Register jobs before init**: ensure `register_job(name, handler)` runs before workers start.
- **Templates**: metrics UI uses Tera templates. If a Tera parse error occurs, restart the process (once_cell poison).
- **Security**: protect metrics with Basic Auth via `QRUSH_ENGINE_BASIC_AUTH` or your own middleware.



---

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.


## 🙏 Acknowledgments

- Built with [Actix Web](https://actix.rs/) - Fast, powerful web framework
- UI powered by [TailwindCSS](https://tailwindcss.com/) - Utility-first CSS framework


---

Made with ❤️ by the [Srotas Space] (https://srotas.space/open-source)

---

[![GitHub stars](https://img.shields.io/github/stars/srotas-space/qrush-engine?style=social)](https://github.com/srotas-space/qrush-engine)
[![LinkedIn Follow](https://img.shields.io/badge/LinkedIn-Follow-blue?style=social&logo=linkedin)](https://www.linkedin.com/company/srotas-space)


## Support

- **Documentation**: [docs.rs/qrush-engine](https://docs.rs/qrush-engine)
- **Issues**: [GitHub Issues](https://github.com/srotas-space/qrush-engine/issues)
- **Discussions**: [GitHub Discussions](https://github.com/srotas-space/qrush-engine/discussions)
## qrush-engine (Sidekiq-style worker process)

`qrush-engine` is the recommended way to run QRush in production: **a separate OS process** dedicated to executing jobs
(similar to how Sidekiq runs separately from a Rails web server).
