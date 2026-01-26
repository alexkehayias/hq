use crate::api;
use crate::core::AppConfig;

pub async fn run(host: String, port: String) {
    let config = AppConfig::default();
    api::serve(host, port, config).await;
}
