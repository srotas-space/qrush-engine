// src/bin/qrush_engine.rs
//
// qrush-engine: Sidekiq-style worker runtime (separate process).
//
// This binary is intentionally focused on:
// - booting worker pools (queues + concurrency)
// - booting delayed-job handler
// - booting cron scheduler
// - graceful shutdown via SIGINT/SIGTERM
//
// Example:
//   qrush-engine --redis redis://127.0.0.1:6379 --queues default:10,critical:25
//
// Notes:
// - Jobs must be registered via qrush_engine::registry::register_job(...) *before* jobs are produced.
// - This binary starts workers/scheduler based on QueueConfig::initialize(...).

use clap::Parser;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use qrush_engine::config::{trigger_shutdown, QueueConfig};

#[derive(Parser, Debug)]
#[command(name = "qrush-engine", version, about = "Sidekiq-style worker runtime for QRush")]
struct Args {
    /// Redis connection URL (also supports env QRUSH_ENGINE_REDIS_URL / REDIS_URL)
    #[arg(long, env = "QRUSH_ENGINE_REDIS_URL", default_value = "redis://127.0.0.1:6379")]
    redis: String,

    /// Queue list, optionally with concurrency and priority:
    ///   default:10
    ///   critical:25:0
    #[arg(long, default_value = "default:10")]
    queues: String,

    /// Grace period before exit after shutdown signal
    #[arg(long, default_value_t = 5)]
    shutdown_grace_secs: u64,
}

fn parse_queues(spec: &str) -> Vec<QueueConfig> {
    spec.split(',')
        .filter_map(|raw| {
            let s = raw.trim();
            if s.is_empty() {
                return None;
            }
            let parts: Vec<&str> = s.split(':').collect();
            let name = parts.get(0).unwrap_or(&"default").trim().to_string();
            let concurrency = parts
                .get(1)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(5);
            let priority = parts
                .get(2)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);

            Some(QueueConfig::new(name, concurrency, priority))
        })
        .collect()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // Load .env if present (mac local convenience)
    match dotenvy::dotenv() {
        Ok(path) => println!("Loaded .env from: {}", path.display()),
        Err(e) => eprintln!("⚠️  .env not loaded: {e}"),
    }

    // Tracing (respects RUST_LOG)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    // Support REDIS_URL if user didn't set QRUSH_ENGINE_REDIS_URL explicitly.
    let redis_url = std::env::var("QRUSH_ENGINE_REDIS_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| args.redis.clone());

    let queues = parse_queues(&args.queues);
    if queues.is_empty() {
        warn!("No queues configured; defaulting to default:10");
    }

    info!(redis = %redis_url, queues = %args.queues, "Starting qrush-engine");

    QueueConfig::initialize(redis_url, if queues.is_empty() { vec![QueueConfig::new("default", 10, 0)] } else { queues }).await?;

    info!("qrush-engine running. Press Ctrl+C to stop.");

    // Wait for shutdown signal
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C");
        }
        _ = async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                    sigterm.recv().await;
                    return;
                }
            }
            std::future::pending::<()>().await
        } => {}
    }

    // Signal worker loops to stop
    trigger_shutdown();

    // Give workers a short grace period to exit loops
    let grace = Duration::from_secs(args.shutdown_grace_secs);
    info!(?grace, "Waiting for graceful shutdown");
    tokio::time::sleep(grace).await;

    info!("qrush-engine exited");
    Ok(())
}
