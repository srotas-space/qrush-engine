// /Users/snm/ws/xsnm/ws/crates/qrush-engine/src/services/cron_service.rs

use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use tera::Context;
use crate::cron::cron_scheduler::CronScheduler;
use crate::services::template_service::render_template;
use crate::cron::cron_job::CronJobMeta;


#[derive(Deserialize)]
pub struct CronActionRequest {
    pub action: String,
    pub job_id: String,
    pub enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub queue: String,
    pub cron_expression: String,
    pub job_type: String,
    pub payload: serde_json::Value,
}




pub async fn render_cron_jobs() -> impl Responder {
    // 1) fetch from redis (this returns HashMap<String, CronJobMeta>)
    let map = match CronScheduler::list_cron_jobs().await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to list cron jobs: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to list cron jobs");
        }
    };

    // 2) convert map -> vec (template expects `{% for job in cron_jobs %}`)
    let mut cron_jobs: Vec<CronJobMeta> = map
        .into_iter()
        .map(|(id, mut meta)| {
            // Ensure `job.id` exists in template
            // If CronJobMeta already has id set, this is harmless.
            meta.id = id;
            meta
        })
        .collect();

    // 3) optional sort: next_run asc
    cron_jobs.sort_by(|a, b| a.next_run.cmp(&b.next_run));

    // 4) render
    let mut ctx = Context::new();
    ctx.insert("title", "Cron Jobs");
    ctx.insert("cron_jobs", &cron_jobs);

    render_template("cron_jobs.html.tera", ctx).await
}


// pub async fn render_cron_jobs() -> impl Responder {
//     match CronScheduler::list_cron_jobs().await {
//         Ok(cron_jobs) => {
//             let mut ctx = Context::new();
//             ctx.insert("title", "Cron Jobs");
//             ctx.insert("cron_jobs", &cron_jobs);
//             render_template("cron_jobs.html.tera", ctx).await
//         }
//         Err(e) => {
//             eprintln!("Failed to fetch cron jobs: {:?}", e);
//             HttpResponse::InternalServerError().body("Failed to fetch cron jobs")
//         }
//     }
// }

pub async fn cron_action(payload: web::Json<CronActionRequest>) -> impl Responder {
    let action = &payload.action;
    let job_id = &payload.job_id;

    let result = match action.as_str() {
        "toggle" => {
            let enabled = payload.enabled.unwrap_or(true);
            CronScheduler::toggle_cron_job(job_id.clone(), enabled).await
        }
        "delete" => {
            CronScheduler::delete_cron_job(job_id.clone()).await
        }
        "run_now" => {
            match CronScheduler::run_now(job_id.clone()).await {
                Ok(enqueued_id) => {
                    return HttpResponse::Ok().json(serde_json::json!({
                        "status": "success",
                        "action": "run_now",
                        "enqueued_job_id": enqueued_id
                    }));
                }
                Err(e) => Err(e)
            }
        }
        _ => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid action"
            }));
        }
    };

    match result {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "action": action
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e.to_string()
        }))
    }
}

pub async fn create_cron_job(_payload: web::Json<CreateCronJobRequest>) -> impl Responder {
    // This would need to be implemented based on your job registry
    // For now, return a placeholder response
    HttpResponse::Ok().json(serde_json::json!({
        "status": "Cron job creation endpoint - implement based on your job types"
    }))
}