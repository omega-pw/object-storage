use crate::context::Context;
use crate::sdk;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::primitives::ByteStreamError;
use aws_sdk_s3::primitives::DateTimeFormat;
use aws_sdk_s3::primitives::SdkBody;
use base64::engine::Engine;
use base64::prelude::BASE64_STANDARD;
use crypto::digest::Digest;
use crypto::sha2::Sha512;
use futures::stream::Stream;
use futures::stream::TryStreamExt;
use futures::StreamExt;
use headers::Header;
use headers::{ContentLength, ContentType, ETag, HeaderMapExt, LastModified};
use http::HeaderValue;
use http_body_util::BodyExt;
use http_body_util::BodyStream;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::{header::CONTENT_TYPE, Request, Response, StatusCode};
use log;
use multer::Field;
use multer::Multipart;
use sdk::storage::DeleteReq;
use sdk::storage::GetReq;
use serde;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Error as IoError;
use std::io::ErrorKind as IoErrorKind;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use tihu::LightString;
use tihu_native::http::Body;
use tihu_native::ErrNo;

struct FuturesStreamCompatByteStream(ByteStream);
impl Stream for FuturesStreamCompatByteStream {
    type Item = Result<Bytes, ByteStreamError>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        ByteStream::poll_next(Pin::new(&mut self.0), cx)
    }
}

async fn get_file_meta(
    context: &Context,
    key: &str,
) -> Result<GetObjectAttributesOutput, Box<dyn std::error::Error + Send + Sync>> {
    let oss_client = context.get_oss_client();
    let bucket = context.get_bucket();
    let resp = oss_client
        .get_object_attributes()
        .bucket(bucket.as_ref())
        .key(key)
        .send()
        .await?;
    return Ok(resp);
}

async fn get_file(
    context: &Context,
    get_req: GetReq,
) -> Result<Response<Body>, Box<dyn std::error::Error + Send + Sync>> {
    let oss_client = context.get_oss_client();
    let bucket = context.get_bucket();
    match oss_client
        .get_object()
        .bucket(bucket.as_ref())
        .key(&get_req.key)
        .send()
        .await
    {
        Ok(resp) => {
            let mut response = Response::new(Body::from_bytes_stream(
                FuturesStreamCompatByteStream(resp.body),
            ));
            if let Some(content_type) = resp.content_type {
                response
                    .headers_mut()
                    .insert(ContentType::name(), HeaderValue::from_str(&content_type)?);
            }
            if let Some(content_length) = resp.content_length {
                if 0 <= content_length {
                    response
                        .headers_mut()
                        .typed_insert(ContentLength(content_length as u64));
                }
            }
            if let Some(last_modified) = resp.last_modified {
                response.headers_mut().insert(
                    LastModified::name(),
                    HeaderValue::from_str(&last_modified.fmt(DateTimeFormat::HttpDate)?)?,
                );
            }
            if let Some(e_tag) = resp.e_tag {
                response
                    .headers_mut()
                    .insert(ETag::name(), HeaderValue::from_str(&e_tag)?);
            }
            return Ok(response);
        }
        Err(err) => match err {
            SdkError::ServiceError(err) => match err.err() {
                GetObjectError::NoSuchKey(_) => {
                    let mut response = Response::new("file not found!".into());
                    response
                        .headers_mut()
                        .typed_insert(ContentType::text_utf8());
                    *response.status_mut() = StatusCode::NOT_FOUND;
                    return Ok(response);
                }
                _ => {
                    return Err(err.into_err().into());
                }
            },
            _ => {
                return Err(err.into());
            }
        },
    }
}

pub async fn get(context: Arc<Context>, get_req: GetReq) -> Result<Response<Body>, hyper::Error> {
    match get_file(&context, get_req).await {
        Ok(resp) => {
            return Ok(resp);
        }
        Err(err) => {
            log::error!("{:?}", err);
            let mut resp = Response::new(Body::from("Fetch file failed!"));
            *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            resp.headers_mut().typed_insert(ContentType::text_utf8());
            return Ok(resp);
        }
    }
}

async fn save_file(
    context: &Context,
    content_length: u64,
    content_type: impl Into<String>,
    data: ByteStream,
    key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let oss_client = context.get_oss_client();
    let bucket = context.get_bucket();
    let _resp = oss_client
        .put_object()
        .bucket(bucket.as_ref())
        .key(key)
        .content_length(content_length as i64)
        .content_type(content_type)
        .body(data)
        .send()
        .await?;
    return Ok(());
}

async fn consume_field_data<'a>(
    field_data: &mut Field<'a>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    while let Some(_) = field_data.try_next().await? {
        //
    }
    return Ok(());
}

async fn handle_multipart(
    context: &Arc<Context>,
    mut multipart: Multipart<'static>,
    hash: String,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut field_map: HashMap<String, String> = HashMap::new();
    let mut ret_opt: Option<Result<String, Box<dyn std::error::Error + Send + Sync>>> = None;
    while let Some(mut field) = multipart.next_field().await? {
        let context = context.clone();
        if ret_opt.is_some() {
            consume_field_data(&mut field).await?;
        } else {
            if let (true, Some(content_type)) = (
                field.file_name().is_some(),
                field.content_type().map(Clone::clone),
            ) {
                //文件域
                let field_name = field.name();
                if Some("file") == field_name {
                    if let Some(content_length) = field_map.remove("size") {
                        match u64::from_str_radix(&content_length, 10) {
                            Err(err) => {
                                log::error!("size不是正整型数字, {:?}", err);
                                ret_opt.replace(Err(err.into()));
                                consume_field_data(&mut field).await?;
                            }
                            Ok(content_length) => {
                                if 0 == content_length {
                                    log::error!("上传的文件内容为空");
                                    ret_opt.replace(Err("Empty file is not allowed".into()));
                                    consume_field_data(&mut field).await?;
                                } else {
                                    let existed = get_file_meta(&context, &hash).await.is_ok();
                                    if existed {
                                        //文件存在的情况不访问oss服务
                                        let mut actual_content_length = 0;
                                        let mut hasher = Sha512::new();
                                        while let Some(chunk) = field.try_next().await? {
                                            actual_content_length += chunk.len();
                                            hasher.input(&chunk);
                                        }
                                        if content_length != actual_content_length as u64 {
                                            log::error!("上传的文件实际大小不一致");
                                            ret_opt.replace(Err("File size not match".into()));
                                        } else {
                                            let mut out: [u8; 64] = [0; 64];
                                            hasher.result(&mut out);
                                            let actual_hash = BASE64_STANDARD.encode(&out);
                                            if actual_hash != hash {
                                                log::error!("sha512和文件的实际hash不一致");
                                                ret_opt.replace(Err(
                                                    "Sha512 of content is invalid".into(),
                                                ));
                                            } else {
                                                ret_opt.replace(Ok(actual_hash));
                                            }
                                        }
                                    } else {
                                        let hash_clone = hash.clone();
                                        let actual_content_length = Arc::new(AtomicUsize::new(0));
                                        let actual_content_length_clone =
                                            actual_content_length.clone();
                                        let hasher = Arc::new(Mutex::new(Sha512::new()));
                                        let hasher_clone = hasher.clone();
                                        let new_stream = field
                                            .inspect_ok(move |chunk| {
                                                actual_content_length_clone
                                                    .fetch_add(chunk.len(), Ordering::Relaxed);
                                                hasher_clone.lock().unwrap().input(chunk);
                                            })
                                            .map_err(|err| -> IoError {
                                                IoError::new(IoErrorKind::Other, err)
                                            });
                                        save_file(
                                            &context,
                                            content_length,
                                            content_type.as_ref(),
                                            ByteStream::from(SdkBody::from_body_1_x(
                                                Body::from_bytes_stream(new_stream),
                                            )),
                                            &hash_clone,
                                        )
                                        .await?;
                                        let actual_content_length =
                                            actual_content_length.load(Ordering::Relaxed);
                                        if content_length != actual_content_length as u64 {
                                            log::error!("上传的文件实际大小不一致");
                                            if let Err(err) = delete_file(&context, &hash).await {
                                                log::error!("移除错误的上传文件失败, {:?}", err);
                                            }
                                            ret_opt.replace(Err("File size not match".into()));
                                        } else {
                                            // buffer.flush().await?;
                                            // buffer.shutdown().await?;
                                            let mut out: [u8; 64] = [0; 64];
                                            hasher.lock().unwrap().result(&mut out);
                                            let actual_hash = BASE64_STANDARD.encode(&out);
                                            if actual_hash != hash {
                                                log::error!("sha512和文件的实际hash不一致");
                                                if let Err(err) = delete_file(&context, &hash).await
                                                {
                                                    log::error!(
                                                        "移除错误的上传文件失败, {:?}",
                                                        err
                                                    );
                                                }
                                                ret_opt.replace(Err(
                                                    "Sha512 of content is invalid".into(),
                                                ));
                                            } else {
                                                ret_opt.replace(Ok(actual_hash));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        ret_opt.replace(Err(
                            "Field \"size\"、\"sha512\" or \"token\" not found.".into()
                        ));
                        consume_field_data(&mut field).await?;
                    }
                } else {
                    //不是名为file的文件域，消耗掉网络数据，继续寻找
                    consume_field_data(&mut field).await?;
                }
            } else {
                if let Some(field_name) = field.name().map(ToString::to_string) {
                    // 文本域
                    let mut data = Vec::new();
                    while let Some(chunk) = field.try_next().await? {
                        data.extend(chunk);
                    }
                    let value = String::from_utf8(data)?;
                    field_map.insert(field_name.to_string(), value);
                } else {
                    // 文本域且没有名字，消耗掉网络数据，继续寻找
                    consume_field_data(&mut field).await?;
                }
            }
        }
    }
    return ret_opt.unwrap_or_else(|| Err("Field \"file\" not found.".into()));
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PutResp {
    pub key: String,
}

pub async fn put(
    context: Arc<Context>,
    req: Request<Incoming>,
) -> Result<Response<Body>, hyper::Error> {
    let boundary = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
        .and_then(|ct| multer::parse_boundary(ct).ok());
    let sha512 = req.headers().get("X-sha512");
    let sha512 = sha512
        .map(|sha512| sha512.to_str().map(|sha512| sha512.to_string()).ok())
        .flatten();
    if let (Some(boundary), Some(sha512)) = (boundary, sha512) {
        let body_stream = BodyStream::new(req.into_body()).filter_map(|result| async move {
            result.map(|frame| frame.into_data().ok()).transpose()
        });
        let multipart = Multipart::new(body_stream, boundary);
        let resp = match handle_multipart(&context, multipart, sha512).await {
            Ok(key) => Ok(PutResp { key: key }),
            Err(err) => {
                log::error!("请求格式不正确, {:?}", err);
                Err(ErrNo::CommonError(LightString::from_static(
                    "请求格式不正确",
                )))
            }
        };
        let mut response = Response::new(Body::from(context.result_to_json_resp(resp)));
        response.headers_mut().typed_insert(ContentType::json());
        return Ok(response);
    } else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("BAD REQUEST"))
            .unwrap());
    }
}

async fn delete_file(
    context: &Context,
    key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let oss_client = context.get_oss_client();
    let bucket = context.get_bucket();
    let _resp = oss_client
        .delete_object()
        .bucket(bucket.as_ref())
        .key(key)
        .send()
        .await?;
    return Ok(());
}

pub async fn delete(
    context: Arc<Context>,
    req: Request<Incoming>,
) -> Result<Response<Body>, hyper::Error> {
    let req_body = req.into_body().collect().await?.to_bytes();
    let resp = match serde_json::from_slice::<DeleteReq>(&req_body) {
        Ok(delete_req) => match delete_file(&context, &delete_req.key).await {
            Ok(()) => Ok(()),
            Err(err) => {
                log::error!("删除文件失败, {:?}", err);
                Err(ErrNo::CommonError(LightString::from_static("删除文件失败")))
            }
        },
        Err(err) => {
            log::error!("请求参数格式错误: {:?}", err);
            Err(ErrNo::ParamFormatError)
        }
    };
    let mut response = Response::new(Body::from(context.result_to_json_resp(resp)));
    response.headers_mut().typed_insert(ContentType::json());
    return Ok(response);
}
