use serde::{Deserialize, Serialize};
use tihu::LightString;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Oss {
    pub access_key: LightString,
    pub secret_key: LightString,
    pub endpoint: LightString,
    pub region: LightString,
    pub bucket: LightString,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub aes_key: [u8; 32],
    pub oss: Oss,
}
