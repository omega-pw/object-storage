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
use hyper::{Request, Response, StatusCode};
use object_storage_sdk as sdk;
use sdk::storage::GetReq;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tihu::SharedString;
use tihu_native::http::Body;
use tihu_native::http::BoxBody;
use tihu_native::http::HttpHandler;
use tihu_native::http::RequestData;

pub struct GetMapping {
    path_prefix: SharedString, //请求路径前缀
    key_prefix: SharedString,  //资源对象前缀
}

pub struct UploadMapping {
    path: SharedString,       //上传请求路径
    key_prefix: SharedString, //资源对象前缀
}

pub struct OssHandler {
    context: Arc<Context>,
    get_mapping: Vec<GetMapping>,
    upload_mapping: Vec<UploadMapping>,
    delete_path: Option<SharedString>,
    namespaces: Vec<SharedString>,
}

impl OssHandler {
    pub async fn try_init_from_config(
        config: Config,
        adjust_error_code: impl Fn(i32) -> i32 + Send + Sync + 'static,
    ) -> Result<Self, anyhow::Error> {
        let context = Context::try_init_from_config(config, adjust_error_code)?;
        let context = Arc::new(context);
        return Ok(Self {
            context: context,
            get_mapping: Vec::new(),
            upload_mapping: Vec::new(),
            delete_path: None,
            namespaces: Vec::new(),
        });
    }

    pub fn add_get_mapping(
        &mut self,
        path_prefix: SharedString,
        key_prefix: SharedString,
    ) -> &mut Self {
        self.get_mapping.push(GetMapping {
            path_prefix: path_prefix.clone(),
            key_prefix: key_prefix,
        });
        self.namespaces.push(path_prefix);
        return self;
    }

    pub fn add_upload_mapping(
        &mut self,
        path: SharedString,
        key_prefix: SharedString,
    ) -> &mut Self {
        self.upload_mapping.push(UploadMapping {
            path: path.clone(),
            key_prefix: key_prefix,
        });
        self.namespaces.push(path);
        return self;
    }

    pub fn set_delete_path(&mut self, delete_path: SharedString) -> &mut Self {
        self.delete_path.replace(delete_path);
        return self;
    }
}

#[async_trait]
impl HttpHandler for OssHandler {
    fn namespace(&self) -> &[SharedString] {
        return &self.namespaces;
    }
    async fn handle(
        &self,
        request: Request<Incoming>,
        _remote_addr: SocketAddr,
        _request_data: &mut RequestData,
        prefix: Option<&str>,
    ) -> Result<Response<BoxBody>, anyhow::Error> {
        let prefix = prefix.unwrap_or("");
        let (_, route) = request.uri().path().split_at(prefix.len());
        for get_mapping in &self.get_mapping {
            let path_prefix = &get_mapping.path_prefix;
            let key_prefix = &get_mapping.key_prefix;
            if route.starts_with(path_prefix.as_ref()) {
                let key = format!(
                    "{}{}",
                    key_prefix,
                    String::from_utf8_lossy(&route.as_bytes()[path_prefix.len()..])
                );
                if key.is_empty() {
                    let mut response = text_response("file not found!");
                    *response.status_mut() = StatusCode::NOT_FOUND;
                    return Ok(response.map(From::from));
                } else {
                    let query = request
                        .uri()
                        .query()
                        .map(|query| {
                            form_urlencoded::parse(query.as_bytes()).collect::<HashMap<_, _>>()
                        })
                        .unwrap_or_default();
                    let filename = query.get("filename").map(|filename| filename.as_ref());
                    let response =
                        action::storage::get(self.context.clone(), GetReq { key: key }, filename)
                            .await?;
                    return Ok(response.map(From::from));
                }
            }
        }

        for upload_mapping in &self.upload_mapping {
            let path = &upload_mapping.path;
            let key_prefix = &upload_mapping.key_prefix;
            if route == path.as_ref() {
                let hash = request.headers().get("X-Hash");
                let hash = hash
                    .map(|hash| hash.to_str().map(|hash| hash.to_string()).ok())
                    .flatten();
                if let Some(hash) = hash {
                    let response =
                        action::storage::put(self.context.clone(), request, key_prefix, hash)
                            .await?;
                    return Ok(response.map(From::from));
                } else {
                    let response = Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::from("BAD REQUEST"))
                        .unwrap();
                    return Ok(response.map(From::from));
                }
            }
        }

        if let Some(delete_path) = self.delete_path.as_ref() {
            if route == delete_path.as_ref() {
                let hash = request.headers().get("X-Hash");
                let hash = hash
                    .map(|hash| hash.to_str().map(|hash| hash.to_string()).ok())
                    .flatten();
                if let Some(hash) = hash {
                    let response =
                        action::storage::delete(self.context.clone(), request, hash).await?;
                    return Ok(response.map(From::from));
                } else {
                    let response = Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::from("BAD REQUEST"))
                        .unwrap();
                    return Ok(response.map(From::from));
                }
            }
        }
        let mut response = text_response("file not found!");
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Ok(response.map(From::from));
    }
}

fn text_response<T: Into<Body>>(body: T) -> Response<Body> {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .typed_insert(ContentType::text_utf8());
    return response;
}
