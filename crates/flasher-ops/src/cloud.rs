use std::time::Duration;

use serde::{Deserialize, Serialize};

const DEFAULT_API: &str = "https://api.aircast.one";
pub const SIGNED_OUT: &str = "Signed out of Aircast Cloud — sign in again.";

fn api_base() -> String {
    std::env::var("AIRCAST_API_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API.to_string())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    pub token: String,
    #[serde(alias = "auth_url")]
    pub auth_url: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SignInStatus {
    Pending,
    SignedIn {
        email: String,
        #[serde(skip)]
        session_token: String,
    },
}

#[derive(Deserialize)]
struct PollUser {
    email: String,
}

#[derive(Deserialize)]
struct Poll {
    session_token: Option<String>,
    user: Option<PollUser>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudKeyset {
    pub id: String,
    pub url: String,
    pub advertisement: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCard {
    pub id: String,
    pub label: String,
    #[serde(alias = "created_at")]
    pub created_at: String,
    #[serde(alias = "lost_at", default)]
    pub lost_at: Option<String>,
    #[serde(alias = "last_unlock_at", default)]
    pub last_unlock_at: Option<String>,
    #[serde(alias = "last_unlock_ip", default)]
    pub last_unlock_ip: Option<String>,
}

async fn api_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or(body);
    format!("Aircast Cloud returned {status}: {message}")
}

async fn authed(
    session_token: &str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    if session_token.trim().is_empty() {
        return Err(SIGNED_OUT.to_string());
    }
    let resp = request
        .bearer_auth(session_token)
        .send()
        .await
        .map_err(|e| format!("could not reach Aircast Cloud: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(SIGNED_OUT.to_string());
    }
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(resp)
}

pub async fn sign_in_start() -> Result<SignIn, String> {
    let resp = client()?
        .post(format!("{}/v1/oauth2/user/sessions", api_base()))
        .json(&serde_json::json!({ "platform": "desktop" }))
        .send()
        .await
        .map_err(|e| format!("could not reach Aircast Cloud: {e}"))?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    resp.json::<SignIn>()
        .await
        .map_err(|e| format!("unexpected sign-in response: {e}"))
}

pub async fn sign_in_poll(token: &str) -> Result<SignInStatus, String> {
    let resp = client()?
        .get(format!(
            "{}/v1/oauth2/user/sessions/poll/{token}",
            api_base()
        ))
        .send()
        .await
        .map_err(|e| format!("could not reach Aircast Cloud: {e}"))?;
    match resp.status() {
        reqwest::StatusCode::ACCEPTED => Ok(SignInStatus::Pending),
        reqwest::StatusCode::NOT_FOUND => Err("The sign-in expired. Start it again.".to_string()),
        s if s.is_success() => {
            let poll = resp
                .json::<Poll>()
                .await
                .map_err(|e| format!("unexpected sign-in response: {e}"))?;
            Ok(match (poll.session_token, poll.user) {
                (Some(session_token), Some(user)) => SignInStatus::SignedIn {
                    email: user.email,
                    session_token,
                },
                _ => SignInStatus::Pending,
            })
        }
        _ => Err(api_error(resp).await),
    }
}

pub async fn create_keyset(session_token: &str, label: &str) -> Result<CloudKeyset, String> {
    let request = client()?
        .post(format!("{}/v1/me/tang/keysets", api_base()))
        .json(&serde_json::json!({ "label": label }));
    authed(session_token, request)
        .await?
        .json::<CloudKeyset>()
        .await
        .map_err(|e| format!("unexpected key set response: {e}"))
}

pub async fn list_cards(session_token: &str) -> Result<Vec<CloudCard>, String> {
    let request = client()?.get(format!("{}/v1/me/tang/keysets", api_base()));
    authed(session_token, request)
        .await?
        .json::<Vec<CloudCard>>()
        .await
        .map_err(|e| format!("unexpected cards response: {e}"))
}

pub async fn mark_lost(session_token: &str, id: &str) -> Result<(), String> {
    let request = client()?.post(format!("{}/v1/me/tang/keysets/{id}/lost", api_base()));
    authed(session_token, request).await.map(|_| ())
}

pub fn card_label(hostname: &str) -> String {
    match hostname.trim() {
        "" => "unnamed card".to_string(),
        name => name.to_string(),
    }
}

pub fn with_cloud_server(
    encryption: flasher_core::EncryptionConfig,
    keyset: &CloudKeyset,
) -> flasher_core::EncryptionConfig {
    flasher_core::EncryptionConfig {
        tang_servers: [
            encryption.tang_servers,
            vec![flasher_core::TangServer {
                url: keyset.url.clone(),
                adv: keyset.advertisement.to_string(),
            }],
        ]
        .concat(),
        ..encryption
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_defaults_to_production_and_honours_an_override() {
        std::env::remove_var("AIRCAST_API_URL");
        assert_eq!(api_base(), "https://api.aircast.one");
        std::env::set_var("AIRCAST_API_URL", " https://api.dev.aircast.one/ ");
        assert_eq!(api_base(), "https://api.dev.aircast.one");
        std::env::remove_var("AIRCAST_API_URL");
    }

    #[test]
    fn a_card_without_unlocks_or_loss_parses() {
        let card: CloudCard = serde_json::from_str(
            r#"{"id":"a","label":"falcon-01","created_at":"2026-10-17T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(card.label, "falcon-01");
        assert!(card.lost_at.is_none() && card.last_unlock_at.is_none());
    }

    #[test]
    fn a_created_key_set_keeps_its_advertisement_as_json() {
        let ks: CloudKeyset = serde_json::from_str(
            r#"{"id":"a","label":"x","url":"https://api.aircast.one/v1/tang/a","advertisement":{"payload":"p","signatures":[]}}"#,
        )
        .unwrap();
        assert_eq!(ks.advertisement["payload"], "p");
    }

    #[tokio::test]
    async fn a_missing_session_is_signed_out_without_a_request() {
        assert_eq!(list_cards("").await.unwrap_err(), SIGNED_OUT);
    }

    #[test]
    fn the_cloud_key_set_joins_the_card_servers() {
        let keyset: CloudKeyset = serde_json::from_str(
            r#"{"id":"a","url":"https://api.aircast.one/v1/tang/a","advertisement":{"payload":"p"}}"#,
        )
        .unwrap();
        let encryption = flasher_core::EncryptionConfig {
            tang_servers: vec![flasher_core::TangServer {
                url: "http://10.0.0.5:7500".into(),
                adv: "{}".into(),
            }],
            threshold: 1,
            recovery_key: "k".into(),
        };
        let joined = with_cloud_server(encryption, &keyset);
        assert_eq!(joined.tang_servers.len(), 2);
        assert_eq!(joined.tang_servers[1].url, keyset.url);
        assert_eq!(joined.tang_servers[1].adv, r#"{"payload":"p"}"#);
    }
}
