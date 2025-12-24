use actix_web::{test, web, App, HttpResponse};

use qrush_engine::services::basic_auth_service::BasicAuthMiddleware;

async fn ok() -> HttpResponse {
    HttpResponse::Ok().body("ok")
}

#[actix_web::test]
async fn allows_when_no_auth_configured() {
    // IMPORTANT:
    // Do NOT call set_basic_auth(None) here.
    // If your config uses OnceCell, calling set_basic_auth(None) will "lock" it forever.

    let app = test::init_service(
        App::new()
            .wrap(BasicAuthMiddleware)
            .route("/m", web::get().to(ok)),
    )
    .await;

    let req = test::TestRequest::get().uri("/m").to_request();
    let resp = test::call_service(&app, req).await;

    assert!(resp.status().is_success());
}
