use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use json5;
use serde::{Deserialize, Serialize};
use std::fs::read_to_string;
use std::net::IpAddr;
use std::net::Ipv4Addr;

#[derive(Serialize, Deserialize, Debug)]
pub struct Oss {
    pub access_key: String,
    pub secret_key: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub log_cfg_path: Option<String>,
    pub aes_key: [u8; 32],
    pub oss: Oss,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OriginConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub log_cfg_path: Option<String>,
    pub aes_key: String,
    pub oss: Oss,
}

pub fn load_env_cfg(env_cfg_path: &str) -> Result<Config, String> {
    let content = read_to_string(env_cfg_path).map_err(|err| -> String {
        log::error!("read file to string error: {}", err);
        return String::from("read file to string error");
    })?;
    let ret: json5::Result<OriginConfig> = json5::from_str(&content);
    match ret {
        Ok(OriginConfig {
            host,
            port,
            log_cfg_path,
            aes_key,
            oss,
        }) => {
            let aes_key_bytes = BASE64_STANDARD.decode(&aes_key).map_err(|err| -> String {
                log::error!("{:?}", err);
                return "通信秘钥格式格式错误".into();
            })?;
            if 32 != aes_key_bytes.len() {
                return Err("通信秘钥必须是256位".into());
            }
            let mut aes_key = [0u8; 32];
            aes_key.copy_from_slice(&aes_key_bytes);
            return Ok(Config {
                host: IpAddr::V4(
                    host.map(|host| host.parse::<Ipv4Addr>().unwrap_or(Ipv4Addr::UNSPECIFIED))
                        .unwrap_or(Ipv4Addr::UNSPECIFIED),
                ),
                port: port.unwrap_or(80),
                log_cfg_path: log_cfg_path,
                aes_key: aes_key,
                oss,
            });
        }
        Err(err) => {
            log::error!("环境配置文件格式不正确: {:?}", err);
            return Err(String::from("parse json error"));
        }
    };
}
