mod action;
mod config;
mod context;

use async_trait::async_trait;
pub use config::Config;
pub use config::Oss;
use context::Context;
use headers::{ContentType, HeaderMapExt};
use hyper;
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use object_storage_sdk as sdk;
use sdk::storage::GetReq;
use std::net::SocketAddr;
use std::sync::Arc;
use tihu_native::http::Body;
use tihu_native::http::BoxBody;
use tihu_native::http::HttpHandler;

pub const KEY_PREFIX: &'static str = "/blob/";
const API_PREFIX: &'static str = "/api/oss/";

pub struct OssHandler {
    context: Arc<Context>,
}

impl OssHandler {
    pub async fn try_init_from_config(
        config: Config,
        adjust_error_code: impl Fn(i32) -> i32 + Send + Sync + 'static,
    ) -> Result<Self, anyhow::Error> {
        let context = Context::try_init_from_config(config, adjust_error_code)?;
        let context = Arc::new(context);
        return Ok(Self { context: context });
    }
}

#[async_trait]
impl HttpHandler for OssHandler {
    fn namespace(&self) -> &[&'static str] {
        return &[KEY_PREFIX, API_PREFIX];
    }
    async fn handle(
        &self,
        request: Request<Incoming>,
        _remote_addr: SocketAddr,
        prefix: Option<&str>,
    ) -> Result<Response<BoxBody>, hyper::Error> {
        let prefix = prefix.unwrap_or("");
        let resp = dispatch(self.context.clone(), request, prefix).await?;
        return Ok(resp.map(|body| body.into_inner()));
    }
}

fn text_response<T: Into<Body>>(body: T) -> Response<Body> {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .typed_insert(ContentType::text_utf8());
    return response;
}

async fn dispatch(
    context: Arc<Context>,
    request: Request<Incoming>,
    prefix: &str,
) -> Result<Response<Body>, hyper::Error> {
    let (_, route) = request.uri().path().split_at(prefix.len());
    let key_prefix = KEY_PREFIX;
    if route.starts_with(key_prefix) {
        let key = String::from_utf8_lossy(&route.as_bytes()[key_prefix.len()..]).into_owned();
        if key.is_empty() {
            let mut response = text_response("file not found!");
            *response.status_mut() = StatusCode::NOT_FOUND;
            return Ok(response);
        } else {
            return action::storage::get(context, GetReq { key: key }).await;
        }
    } else if route.starts_with(API_PREFIX) {
        if &Method::GET == request.method() {
            let mut response = text_response("file not found!");
            *response.status_mut() = StatusCode::NOT_FOUND;
            return Ok(response);
        } else if &Method::POST == request.method() {
            let hash = request.headers().get("X-Hash");
            let hash = hash
                .map(|hash| hash.to_str().map(|hash| hash.to_string()).ok())
                .flatten();
            if let Some(hash) = hash {
                match route {
                    sdk::storage::UPLOAD_API => action::storage::put(context, request, hash).await,
                    sdk::storage::DELETE_API => {
                        action::storage::delete(context, request, hash).await
                    }
                    _ => Ok(Response::new(Body::from(context.gen_no_such_api()))),
                }
            } else {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("BAD REQUEST"))
                    .unwrap());
            }
        } else {
            return Ok(Response::new(Body::empty()));
        }
    } else {
        return Ok(Response::new(Body::empty()));
    }
}
