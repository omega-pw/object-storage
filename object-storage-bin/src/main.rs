mod config;
mod encrypt;

use base64::engine::Engine;
use base64::prelude::BASE64_STANDARD;
use chrono::DateTime;
use chrono::Utc;
use config::Config;
use encrypt::decrypt_by_aes_256;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::tokio::TokioIo;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto;
use log::LevelFilter;
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use object_storage_lib::Config as HandlerConfig;
use object_storage_lib::Oss;
use object_storage_lib::OssHandler;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tihu_native::http::Body;
use tihu_native::http::HttpHandler;
use tokio::net::TcpListener;

pub fn load_config() -> Config {
    let mut args = Vec::new();
    for v in env::args() {
        args.push(v);
    }
    let cfg_path = args.get(1).expect("app require 1 parameter!");
    let config = match config::load_env_cfg(cfg_path) {
        Ok(v) => v,
        Err(err) => panic!("read env cfg file error, {}", err),
    };
    return config;
}

fn init_console_log() -> Result<(), anyhow::Error> {
    let console = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "{d(%Y-%m-%d %H:%M:%S)} {l} {t}: {m}{n}",
        )))
        .build();
    let config = log4rs::config::Config::builder()
        .appender(Appender::builder().build("CONSOLE", Box::new(console)))
        .build(Root::builder().appender("CONSOLE").build(LevelFilter::Warn))?;
    log4rs::init_config(config)?;
    return Ok(());
}

fn init_logger(log_cfg_path: Option<&str>) -> Result<(), anyhow::Error> {
    if let Some(log_cfg_path) = log_cfg_path {
        if let Err(err) = log4rs::init_file(log_cfg_path, Default::default()) {
            println!("init log4rs failed, {}", err);
            if let Err(err) = init_console_log() {
                println!("init console logger failed, {}", err);
            }
        }
    } else {
        if let Err(err) = init_console_log() {
            println!("init console logger failed, {}", err);
        }
    }
    return Ok(());
}

fn validate_token(
    aes_key: &[u8; 32],
    hash: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let token = BASE64_STANDARD.decode(token)?;
    let token = decrypt_by_aes_256(&token, aes_key)?;
    if 64 + 8 != token.len() {
        log::error!("上传token长度不正确");
        return Err("Invalid token format.".into());
    }
    let hash_in_token = BASE64_STANDARD.encode(&token[0..64]);
    if hash_in_token != hash {
        log::error!("上传token里面的hash和sha512不一致");
        return Err("Token invalid.".into());
    }
    let mut expire_time = [0u8; 8];
    expire_time.copy_from_slice(&token[64..]);
    let expire_time = i64::from_be_bytes(expire_time);
    let expire_time = DateTime::from_timestamp_millis(expire_time).ok_or_else(
        || -> Box<dyn std::error::Error + Send + Sync> {
            log::error!("上传token里面的过期时间格式不正确");
            return "Token invalid.".into();
        },
    )?;
    let curr_time = Utc::now();
    if curr_time > expire_time {
        log::error!("上传token已过期");
        return Err("Token expired.".into());
    }
    return Ok(());
}

async fn dispatch(
    req: Request<Incoming>,
    remote_addr: SocketAddr,
    handler: Arc<impl HttpHandler>,
    aes_key: [u8; 32],
) -> Result<Response<Body>, hyper::Error> {
    let route = req.uri().path();
    if !route.starts_with(object_storage_lib::BLOB_PREFIX) {
        let token = req.headers().get("X-token");
        let token = token.map(|token| token.to_str().ok()).flatten();
        let sha512 = req.headers().get("X-sha512");
        let sha512 = sha512.map(|sha512| sha512.to_str().ok()).flatten();
        if let (Some(token), Some(sha512)) = (token, sha512) {
            if let Err(err) = validate_token(&aes_key, sha512, token) {
                log::error!("check permission failed: {}", err);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("BAD REQUEST"))
                    .unwrap());
            }
        } else {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("BAD REQUEST"))
                .unwrap());
        }
    }
    let response = handler.handle(req, remote_addr, None).await?;
    return Ok(response.map(From::from));
}

pub async fn start_http_service(
    handler: Arc<impl HttpHandler>,
    bind_addr: SocketAddr,
    aes_key: [u8; 32],
) -> Result<(), anyhow::Error> {
    let listener = TcpListener::bind(bind_addr).await?;
    let actual_addr = listener.local_addr()?;
    log::info!("Listening on http://{}", actual_addr);
    println!("Listening on http://{}", actual_addr);
    loop {
        let (stream, remote_addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let handler = handler.clone();
        tokio::task::spawn(async move {
            if let Err(err) = auto::Builder::new(TokioExecutor::new())
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let handler = handler.clone();
                        dispatch(req, remote_addr, handler, aes_key)
                    }),
                )
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

fn adjust_error_code(error_code: i32) -> i32 {
    return error_code;
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let config = load_config();
    init_logger(
        config
            .log_cfg_path
            .as_ref()
            .map(|log_cfg_path| log_cfg_path.as_str()),
    )?;
    let oss = config.oss;
    let handler = OssHandler::try_init_from_config(
        HandlerConfig {
            oss: Oss {
                access_key: oss.access_key.into(),
                secret_key: oss.secret_key.into(),
                endpoint: oss.endpoint.into(),
                region: oss.region.into(),
                bucket: oss.bucket.into(),
            },
        },
        adjust_error_code,
    )
    .await?;
    let bind_addr = SocketAddr::new(config.host, config.port);
    let handler = Arc::new(handler);
    start_http_service(handler, bind_addr, config.aes_key).await?;
    Ok(())
}
