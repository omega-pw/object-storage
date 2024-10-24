use serde;
use serde::{Deserialize, Serialize};
use tihu::Api;
use tihu::LightString;

#[derive(Serialize, Deserialize, Debug)]
pub struct GetReq {
    pub key: String,
}

/**
 * 删除
 */
pub const DELETE_API: &str = "/api/oss/delete";
#[derive(Serialize, Deserialize, Debug)]
pub struct DeleteReq {
    pub key: String,
    pub token: String,
}

pub type DeleteResp = bool;
pub struct DeleteApi;
impl Api for DeleteApi {
    type Input = DeleteReq;
    type Output = DeleteResp;
    fn namespace() -> LightString {
        return LightString::from_static(DELETE_API);
    }
}
