mod config;

use config::Config;
use hyper::service::service_fn;
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

pub async fn start_http_service(
    handler: Arc<impl HttpHandler>,
    bind_addr: SocketAddr,
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
                        async move { handler.handle(req, remote_addr, None).await }
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
            aes_key: config.aes_key,
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
    start_http_service(handler, bind_addr).await?;
    Ok(())
}
