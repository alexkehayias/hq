use hq::core::AppConfig;
use hq_api as api;

pub async fn run(host: String, port: String) {
    let config = AppConfig::default();
    api::serve(host, port, config).await;
}
