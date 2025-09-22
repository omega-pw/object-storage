use serde::{Deserialize, Serialize};
use tihu::SharedString;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Oss {
    pub access_key: SharedString,
    pub secret_key: SharedString,
    pub endpoint: SharedString,
    pub region: SharedString,
    pub bucket: SharedString,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub oss: Oss,
}
