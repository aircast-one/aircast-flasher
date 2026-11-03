use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

const API_KEY_PREFIX: &str = "hskey-api-";
const KEY_LIFETIME_DAYS: i64 = 90;
const TIMEOUT: Duration = Duration::from_secs(15);

pub fn is_api_key(key: &str) -> bool {
    key.trim().starts_with(API_KEY_PREFIX)
}

#[derive(Deserialize)]
struct User {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Users {
    users: Vec<User>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    pre_auth_key: PreAuthKey,
}

#[derive(Deserialize)]
struct PreAuthKey {
    key: String,
}

async fn call<T: DeserializeOwned>(request: RequestBuilder, api_key: &str) -> Result<T, String> {
    let response = request
        .bearer_auth(api_key.trim())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Headscale: {e}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => Err(
            "Headscale refused the API key — it may have expired. Make a new one with `headscale apikeys create --expiration 365d`."
                .to_string(),
        ),
        status if !status.is_success() => Err(format!(
            "Headscale answered {status}: {}",
            response.text().await.unwrap_or_default()
        )),
        _ => response
            .json()
            .await
            .map_err(|e| format!("Unexpected answer from Headscale: {e}")),
    }
}

fn preauth_request(user_id: &str, now: chrono::DateTime<chrono::Utc>) -> serde_json::Value {
    serde_json::json!({
        "user": user_id,
        "reusable": false,
        "ephemeral": false,
        "expiration": (now + chrono::Duration::days(KEY_LIFETIME_DAYS)).to_rfc3339(),
    })
}

pub async fn mint_key(control_server: &str, api_key: &str, user: &str) -> Result<String, String> {
    let base = control_server.trim().trim_end_matches('/');
    let user = user.trim();
    let users: Users = call(crate::http().get(format!("{base}/api/v1/user")), api_key).await?;
    let id = users
        .users
        .into_iter()
        .find(|u| u.name == user)
        .map(|u| u.id)
        .ok_or_else(|| format!("Headscale has no user named \"{user}\""))?;
    let created: Created = call(
        crate::http()
            .post(format!("{base}/api/v1/preauthkey"))
            .json(&preauth_request(&id, chrono::Utc::now())),
        api_key,
    )
    .await?;
    Ok(created.pre_auth_key.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_headscale_api_keys_mint() {
        assert!(is_api_key(" hskey-api-abc-def"));
        assert!(!is_api_key("hskey-auth-abc-def"));
        assert!(!is_api_key("tskey-auth-abc"));
    }

    #[test]
    fn minted_keys_are_one_time_and_outlive_the_trip_to_first_boot() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z")
            .unwrap()
            .to_utc();
        let body = preauth_request("1", now);
        assert_eq!(body["user"], "1");
        assert_eq!(body["reusable"], false);
        assert_eq!(body["ephemeral"], false);
        assert_eq!(body["expiration"], "2027-02-01T12:00:00+00:00");
    }
}
