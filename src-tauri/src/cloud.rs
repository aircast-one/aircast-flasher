use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

const DEFAULT_API: &str = "https://api.aircast.one";
const SIGNED_OUT: &str = "Signed out of Aircast Cloud — sign in again.";

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

struct Session {
    token: String,
    email: String,
}

#[derive(Default)]
pub struct CloudState(Mutex<Option<Session>>);

impl CloudState {
    fn bearer(&self) -> Result<String, String> {
        self.0
            .lock()
            .map_err(|_| "cloud session lock poisoned".to_string())?
            .as_ref()
            .map(|s| s.token.clone())
            .ok_or_else(|| SIGNED_OUT.to_string())
    }

    fn set(&self, session: Option<Session>) -> Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "cloud session lock poisoned".to_string())? = session;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    token: String,
    #[serde(alias = "auth_url")]
    auth_url: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SignInStatus {
    Pending,
    SignedIn { email: String },
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
    id: String,
    url: String,
    advertisement: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCard {
    id: String,
    label: String,
    #[serde(alias = "created_at")]
    created_at: String,
    #[serde(alias = "lost_at", default)]
    lost_at: Option<String>,
    #[serde(alias = "last_unlock_at", default)]
    last_unlock_at: Option<String>,
    #[serde(alias = "last_unlock_ip", default)]
    last_unlock_ip: Option<String>,
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
    state: &CloudState,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    let resp = request
        .bearer_auth(state.bearer()?)
        .send()
        .await
        .map_err(|e| format!("could not reach Aircast Cloud: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        state.set(None)?;
        return Err(SIGNED_OUT.to_string());
    }
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(resp)
}

#[tauri::command]
pub async fn cloud_sign_in_start() -> Result<SignIn, String> {
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

#[tauri::command]
pub async fn cloud_sign_in_poll(
    state: State<'_, CloudState>,
    token: String,
) -> Result<SignInStatus, String> {
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
            let (Some(token), Some(user)) = (poll.session_token, poll.user) else {
                return Ok(SignInStatus::Pending);
            };
            state.set(Some(Session {
                token,
                email: user.email.clone(),
            }))?;
            Ok(SignInStatus::SignedIn { email: user.email })
        }
        _ => Err(api_error(resp).await),
    }
}

#[tauri::command]
pub fn cloud_account(state: State<'_, CloudState>) -> Result<Option<String>, String> {
    Ok(state
        .0
        .lock()
        .map_err(|_| "cloud session lock poisoned".to_string())?
        .as_ref()
        .map(|s| s.email.clone()))
}

#[tauri::command]
pub fn cloud_sign_out(state: State<'_, CloudState>) -> Result<(), String> {
    state.set(None)
}

#[tauri::command]
pub async fn cloud_create_keyset(
    state: State<'_, CloudState>,
    label: String,
) -> Result<CloudKeyset, String> {
    let request = client()?
        .post(format!("{}/v1/me/tang/keysets", api_base()))
        .json(&serde_json::json!({ "label": label }));
    authed(&state, request)
        .await?
        .json::<CloudKeyset>()
        .await
        .map_err(|e| format!("unexpected key set response: {e}"))
}

#[tauri::command]
pub async fn cloud_list_cards(state: State<'_, CloudState>) -> Result<Vec<CloudCard>, String> {
    let request = client()?.get(format!("{}/v1/me/tang/keysets", api_base()));
    authed(&state, request)
        .await?
        .json::<Vec<CloudCard>>()
        .await
        .map_err(|e| format!("unexpected cards response: {e}"))
}

#[tauri::command]
pub async fn cloud_mark_lost(state: State<'_, CloudState>, id: String) -> Result<(), String> {
    let request = client()?.post(format!("{}/v1/me/tang/keysets/{id}/lost", api_base()));
    authed(&state, request).await.map(|_| ())
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

    #[test]
    fn signed_out_state_has_no_bearer() {
        assert_eq!(CloudState::default().bearer().unwrap_err(), SIGNED_OUT);
    }
}
