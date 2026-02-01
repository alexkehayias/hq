use crate::core::db::async_db;
use anyhow::{Result, anyhow};
use std::io::{self, Write};

#[derive(clap::ValueEnum, Clone)]
pub enum ServiceKind {
    Gmail,
}

impl ServiceKind {
    pub fn to_str(&self) -> &'static str {
        match self {
            ServiceKind::Gmail => "gmail",
        }
    }
}

pub async fn run(service: ServiceKind, vec_db_path: &str) -> Result<()> {
    match service {
        ServiceKind::Gmail => {
            use crate::google::oauth::exchange_code_for_token;

            // Prompt the user for their email address
            print!("Enter the email address you are authenticating: ");
            io::stdout().flush().unwrap();
            let mut user_email = String::new();
            io::stdin()
                .read_line(&mut user_email)
                .expect("Failed to read email address");
            let user_email = user_email.trim().to_owned();

            let client_id = std::env::var("HQ_GMAIL_CLIENT_ID")
                .expect("Set HQ_GMAIL_CLIENT_ID in your environment");
            let client_secret = std::env::var("HQ_GMAIL_CLIENT_SECRET")
                .expect("Set HQ_GMAIL_CLIENT_SECRET in your environment");
            let redirect_uri = std::env::var("HQ_GMAIL_REDIRECT_URI")
                .unwrap_or_else(|_| "urn:ietf:wg:oauth:2.0:oob".to_string());
            let scope = "https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/calendar.calendars.readonly https://www.googleapis.com/auth/calendar.events.readonly";
            let auth_url = format!(
                "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
                urlencoding::encode(&client_id),
                urlencoding::encode(&redirect_uri),
                urlencoding::encode(scope)
            );
            println!(
                "\nPlease open the following URL in your browser and authorize access:\n\n{}\n",
                auth_url
            );
            print!("Paste the authorization code shown by Google here: ");
            io::stdout().flush().unwrap();
            let mut code = String::new();
            io::stdin()
                .read_line(&mut code)
                .expect("Failed to read code");
            let code = code.trim();

            let token =
                exchange_code_for_token(&client_id, &client_secret, code, &redirect_uri).await?;

            // Store the refresh token in the DB and use that to fetch an access token from now on.
            let db = async_db(&vec_db_path)
                .await
                .expect("Failed to connect to db");
            let refresh_token = token
                .refresh_token
                .clone()
                .ok_or(anyhow!("No refresh token in response"))?;

            db.call(move |conn| {
                conn.execute(
                    "INSERT INTO auth (id, service, refresh_token) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET service = excluded.service, refresh_token = excluded.refresh_token",
                    (&user_email, service.to_str(), &refresh_token),
                )
                    .expect("Failed to insert/update refresh token in DB");
                println!("Refresh token for {} saved to DB.", user_email);
                Ok(())
            }).await?;
        }
    }

    Ok(())
}
