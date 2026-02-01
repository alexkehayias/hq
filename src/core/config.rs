use std::env;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub notes_path: String,
    pub index_path: String,
    pub vec_db_path: String,
    pub deploy_key_path: String,
    pub vapid_key_path: String,
    pub note_search_api_url: String,
    pub searxng_api_url: String,
    pub gmail_api_client_id: String,
    pub gmail_api_client_secret: String,
    pub google_search_api_key: String,
    pub google_search_cx_id: String,
    pub openai_model: String,
    pub openai_api_hostname: String,
    pub openai_api_key: String,
    pub system_message: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let host = "127.0.0.1";
        let port = "2222";
        let storage_path = env::var("HQ_STORAGE_PATH").unwrap_or("./".to_string());
        let index_path = format!("{}/index", storage_path);
        let notes_path = format!("{}/notes", storage_path);
        let vec_db_path = format!("{}/db", storage_path);
        let deploy_key_path = env::var("HQ_NOTES_DEPLOY_KEY_PATH")
            .expect("Missing env var HQ_NOTES_REPO_URL");
        let vapid_key_path =
            env::var("HQ_VAPID_KEY_PATH").expect("Missing env var HQ_VAPID_KEY_PATH");
        let note_search_api_url =
            env::var("HQ_NOTE_SEARCH_API_URL").unwrap_or(format!("http://{}:{}", host, port));
        let searxng_api_url =
            env::var("HQ_SEARXNG_API_URL").unwrap_or(format!("http://{}:{}", host, "8080"));
        let gmail_api_client_id =
            std::env::var("HQ_GMAIL_CLIENT_ID").expect("Missing HQ_GMAIL_CLIENT_ID");
        let gmail_api_client_secret = std::env::var("HQ_GMAIL_CLIENT_SECRET")
            .expect("Missing HQ_GMAIL_CLIENT_SECRET");
        let openai_api_hostname = env::var("HQ_LOCAL_LLM_HOST")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());
        let openai_api_key =
            env::var("OPENAI_API_KEY").unwrap_or_else(|_| "thiswontworkforopenai".to_string());
        let openai_model =
            env::var("HQ_LOCAL_LLM_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());
        let system_message = env::var("HQ_SYSTEM_MESSAGE")
            .unwrap_or_else(|_| "You are a helpful assistant.".to_string());
        let google_search_api_key = std::env::var("HQ_GOOGLE_SEARCH_API_KEY")
            .expect("Missing env var HQ_GOOGLE_SEARCH_API_KEY");
        let google_search_cx_id = std::env::var("HQ_GOOGLE_SEARCH_CX_ID")
            .expect("Missing env var HQ_GOOGLE_SEARCH_CX_ID");

        Self {
            notes_path: notes_path.clone(),
            index_path,
            vec_db_path: vec_db_path.clone(),
            deploy_key_path,
            vapid_key_path,
            note_search_api_url: note_search_api_url.clone(),
            searxng_api_url,
            gmail_api_client_id,
            gmail_api_client_secret,
            google_search_api_key,
            google_search_cx_id,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            system_message,
        }
    }
}
