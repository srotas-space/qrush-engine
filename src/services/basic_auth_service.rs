// src/services/basic_auth_service.rs
//
// Optional Actix middleware for QRush metrics/routes.
// Enabled when you embed qrush-engine routes into an Actix app.
// (qrush-engine itself does not require Actix to run.)

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::future::{ready, Either, Ready};
use std::future::Ready as StdReady;

use crate::config::get_basic_auth;

pub struct BasicAuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for BasicAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = BasicAuthMiddlewareService<S>;
    type InitError = ();
    type Future = StdReady<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        std::future::ready(Ok(BasicAuthMiddlewareService { service }))
    }
}

pub struct BasicAuthMiddlewareService<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for BasicAuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = Either<
        futures_util::future::LocalBoxFuture<'static, Result<Self::Response, Self::Error>>,
        Ready<Result<Self::Response, Self::Error>>,
    >;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // If no auth configured, allow.
        let Some(cfg) = get_basic_auth() else {
            let fut = self.service.call(req);
            return Either::Left(Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_left_body())
            }));
        };

        // Extract "Authorization: Basic base64(user:pass)"
        let auth_hdr = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Some(encoded) = auth_hdr.strip_prefix("Basic ").map(str::trim) {
            if let Ok(decoded) = STANDARD.decode(encoded) {
                if let Ok(pair) = String::from_utf8(decoded) {
                    let mut it = pair.splitn(2, ':');
                    let user = it.next().unwrap_or("");
                    let pass = it.next().unwrap_or("");
                    if user == cfg.username && pass == cfg.password {
                        let fut = self.service.call(req);
                        return Either::Left(Box::pin(async move {
                            let res = fut.await?;
                            Ok(res.map_into_left_body())
                        }));
                    }
                }
            }
        }

        let res = actix_web::HttpResponse::Unauthorized()
            .finish()
            .map_into_right_body();

        Either::Right(ready(Ok(req.into_response(res))))
    }
}
