//! OAuth 2.0 token exchange & refresh for Gmail API

use anyhow::{Error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::{Connection, rusqlite};

/// Response from Google's token endpoint
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub scope: String,
    pub token_type: String,
}

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// An expected authorization problem: the account is not connected or its
/// credentials are no longer valid.
///
/// These are not server faults, so callers should surface them as warnings
/// and client errors (401), not as 500s.
#[derive(Debug, thiserror::Error)]
pub enum AuthorizationError {
    /// No stored refresh token exists for the requested account.
    #[error("No Google authorization found for this account")]
    Missing,

    /// Google rejected the stored refresh token (`invalid_grant`).
    ///
    /// Google returns `invalid_grant` when a refresh token has expired, been
    /// revoked, or the account's password changed.
    #[error("Google authorization expired; reconnect the account ({0})")]
    Expired(String),
}

/// Whether a token endpoint error body reports an invalid/expired grant.
fn is_invalid_grant(body: &str) -> bool {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: String,
    }

    serde_json::from_str::<ErrorBody>(body)
        .map(|e| e.error == "invalid_grant")
        .unwrap_or_else(|_| body.contains("invalid_grant"))
}

/// Exchange authorization code for access and refresh tokens (see curl above)
pub async fn exchange_code_for_token(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, anyhow::Error> {
    let client = Client::new();

    let params = [
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];
    let res = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Token exchange failed: {} ({})", status, text);
    }
    let token: TokenResponse = serde_json::from_str(&text)?;
    Ok(token)
}

/// Refresh access token using refresh token (see curl above)
pub async fn refresh_access_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, anyhow::Error> {
    let client = Client::new();

    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    let res = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        if is_invalid_grant(&text) {
            return Err(AuthorizationError::Expired(text).into());
        }
        return Err(anyhow::anyhow!(
            "Token refresh failed: {} ({})",
            status,
            text
        ));
    }

    let mut token: TokenResponse = serde_json::from_str(&text)?;
    // The refresh endpoint does not always return refresh_token, so preserve the old one.
    token.refresh_token = Some(refresh_token.to_string());
    Ok(token)
}

/// Look up the stored refresh token for an account.
///
/// Returns [`AuthorizationError::Missing`] when the account has never been
/// connected (or its authorization was removed).
pub async fn find_refresh_token(db: &Connection, email: &str) -> Result<String, Error> {
    let email = email.to_string();
    let token = db
        .call(move |conn| {
            conn.prepare("SELECT refresh_token FROM auth WHERE id = ?1")
                .and_then(|mut stmt| stmt.query_row([&email], |row| row.get(0)))
        })
        .await;

    match token {
        Ok(token) => Ok(token),
        Err(tokio_rusqlite::Error::Error(rusqlite::Error::QueryReturnedNoRows)) => {
            Err(AuthorizationError::Missing.into())
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn find_all_gmail_auth_emails(db: &Connection) -> Result<Vec<String>, Error> {
    let auths = db.call(|conn| {
        let result: Vec<String> = conn
            .prepare("SELECT id FROM auth WHERE service = 'gmail'")?
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        Ok::<_, rusqlite::Error>(result)
    });
    Ok(auths.await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_invalid_grant() {
        assert!(is_invalid_grant(
            r#"{"error": "invalid_grant", "error_description": "Token has been expired or revoked."}"#
        ));
        // Fall back to substring matching when the body is not JSON.
        assert!(is_invalid_grant("invalid_grant: Token has been expired"));
    }

    #[test]
    fn ignores_other_oauth_errors() {
        assert!(!is_invalid_grant(r#"{"error": "invalid_client"}"#));
        assert!(!is_invalid_grant("Token refresh failed: 500"));
    }
}
