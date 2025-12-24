# qrush-engine

`qrush-engine` is a **Sidekiq-style worker runtime** for Rust.

It runs as a **separate OS process** (your “Sidekiq copy/instance”), consumes jobs from Redis queues, executes them with your `Job` hooks (`before/perform/after/on_error/always`), runs delayed jobs, and runs cron schedules.

## Quick start (mac local)

### 1) Start Redis

```bash
redis-server
```

### 2) Run the engine

```bash
REDIS_URL="redis://127.0.0.1:6379" \
  cargo run --bin qrush-engine -- \
  --queues default:10,critical:25
```

## Queue syntax

`--queues` accepts comma-separated items:

- `default:10` => queue=`default`, concurrency=`10`, priority=`0`
- `critical:25:0` => queue=`critical`, concurrency=`25`, priority=`0`

## Using with your Actix app (Sidekiq model)

- Your Actix API **enqueues** jobs (fast path).
- `qrush-engine` **executes** jobs (background process).

### Job example (same style as your NotifyUser)

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

  async fn before(&self) -> Result<()> { Ok(()) }
  async fn perform(&self) -> Result<()> { Ok(()) }
  async fn after(&self) {}
  async fn on_error(&self, _err: &Error) {}
  async fn always(&self) {}
}

impl NotifyUser {
  pub fn name() -> &'static str { "notify_user" }

  pub fn handler(payload: String) -> BoxFuture<'static, Result<Box<dyn Job>>> {
    async move {
      let job: NotifyUser = serde_json::from_str(&payload)?;
      Ok(Box::new(job) as Box<dyn Job>)
    }.boxed()
  }
}
```

### Register jobs (engine init)

In your engine bootstrap (your app’s engine binary), register job factories:

```rust
use qrush_engine::registry::register_job;

register_job(NotifyUser::name(), NotifyUser::handler);
```

Then run workers via `QueueConfig::initialize(...)` (already done by the `qrush-engine` binary in this crate).

## Notes

- `.env` is loaded automatically by the `qrush-engine` binary (via `dotenvy`), so mac local is easy.
- Redis URL is read from `QRUSH_REDIS_URL` or `REDIS_URL`.
