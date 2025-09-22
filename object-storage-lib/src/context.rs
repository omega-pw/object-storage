use super::config::Config;
use super::config::Oss;
use aws_sdk_s3::config::BehaviorVersion;
use aws_sdk_s3::config::Credentials;
use aws_sdk_s3::config::SharedCredentialsProvider;
use aws_sdk_s3::Client;
use aws_types::region::Region;
use aws_types::sdk_config::SdkConfig;
use serde::Serialize;
use std::sync::Arc;
use tihu::api::Response;
use tihu::SharedString;
use tihu_native::ErrNo;

fn init_oss_client(oss: &Oss) -> Result<Client, http::uri::InvalidUri> {
    let credentials = Credentials::new(
        oss.access_key.to_string(),
        oss.secret_key.to_string(),
        None,
        None,
        "",
    );
    let shared_config = SdkConfig::builder()
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(oss.endpoint.as_str())
        .region(Region::new(oss.region.to_string()))
        //behavior_version参数必填，否则会报错
        .behavior_version(BehaviorVersion::latest())
        .build();
    let client = aws_sdk_s3::Client::new(&shared_config);
    return Ok(client);
}

pub struct Context {
    pub config: Config,
    oss_client: Arc<Client>,
    adjust_error_code: Arc<dyn Fn(i32) -> i32 + Send + Sync>,
}

impl Context {
    pub fn try_init_from_config(
        config: Config,
        adjust_error_code: impl Fn(i32) -> i32 + Send + Sync + 'static,
    ) -> Result<Context, anyhow::Error> {
        let oss_client = init_oss_client(&config.oss)?;
        let context = Context {
            config: config,
            oss_client: Arc::new(oss_client),
            adjust_error_code: Arc::new(adjust_error_code),
        };
        return Ok(context);
    }

    pub fn get_oss_client(&self) -> Arc<Client> {
        return self.oss_client.clone();
    }

    fn adjust_error_code(&self, error_code: i32) -> i32 {
        let adjust_error_code = &self.adjust_error_code;
        adjust_error_code(error_code)
    }

    fn err_to_resp(&self, err_no: ErrNo) -> Vec<u8> {
        let err_msg = String::new();
        return (err_msg
            + "{\"code\":"
            + &(self.adjust_error_code(err_no.code())).to_string()
            + ",\"message\":\""
            + &err_no.message()
            + "\"}")
            .into_bytes();
    }

    pub fn result_to_json_resp<T: Serialize>(&self, result: Result<T, ErrNo>) -> Vec<u8> {
        let mut resp: Response<T> = match result {
            Ok(data) => Response::success(Some(data)),
            Err(err_no) => err_no.into(),
        };
        resp.code = self.adjust_error_code(resp.code);
        match serde_json::to_vec(&resp) {
            Ok(resp) => resp,
            Err(err) => {
                log::error!("json序列化失败: {:?}", err);
                self.err_to_resp(ErrNo::SerializeError(err))
            }
        }
    }

    pub fn get_bucket(&self) -> SharedString {
        return self.config.oss.bucket.clone();
    }
}
