use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

const HEADSCALE_API_PREFIX: &str = "hskey-api-";
const TAILSCALE_API_PREFIX: &str = "tskey-api-";
const TAILSCALE_KEYS_URL: &str = "https://api.tailscale.com/api/v2/tailnet/-/keys";
const KEY_LIFETIME_DAYS: i64 = 90;
const TIMEOUT: Duration = Duration::from_secs(15);

pub fn is_headscale_api_key(key: &str) -> bool {
    key.trim().starts_with(HEADSCALE_API_PREFIX)
}

pub fn is_api_key(key: &str) -> bool {
    is_headscale_api_key(key) || key.trim().starts_with(TAILSCALE_API_PREFIX)
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
struct HeadscaleCreated {
    pre_auth_key: AuthKey,
}

#[derive(Deserialize)]
struct AuthKey {
    key: String,
}

async fn call<T: DeserializeOwned>(
    service: &str,
    request: RequestBuilder,
    api_key: &str,
    refused: &str,
) -> Result<T, String> {
    let response = request
        .bearer_auth(api_key.trim())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach {service}: {e}"))?;
    match response.status() {
        StatusCode::UNAUTHORIZED => Err(refused.to_string()),
        status if !status.is_success() => Err(format!(
            "{service} answered {status}: {}",
            response.text().await.unwrap_or_default()
        )),
        _ => response
            .json()
            .await
            .map_err(|e| format!("Unexpected answer from {service}: {e}")),
    }
}

fn headscale_request(user_id: &str, now: chrono::DateTime<chrono::Utc>) -> serde_json::Value {
    serde_json::json!({
        "user": user_id,
        "reusable": false,
        "ephemeral": false,
        "expiration": (now + chrono::Duration::days(KEY_LIFETIME_DAYS)).to_rfc3339(),
    })
}

fn tailscale_request() -> serde_json::Value {
    serde_json::json!({
        "capabilities": {
            "devices": {
                "create": { "reusable": false, "ephemeral": false, "preauthorized": true }
            }
        },
        "expirySeconds": chrono::Duration::days(KEY_LIFETIME_DAYS).num_seconds(),
        "description": "Aircast Flasher",
    })
}

async fn mint_headscale(control_server: &str, api_key: &str, user: &str) -> Result<String, String> {
    const REFUSED: &str = "Headscale refused the API key — it may have expired. Make a new one with `headscale apikeys create --expiration 365d`.";
    let base = control_server.trim().trim_end_matches('/');
    let user = user.trim();
    let users: Users = call(
        "Headscale",
        crate::http().get(format!("{base}/api/v1/user")),
        api_key,
        REFUSED,
    )
    .await?;
    let id = users
        .users
        .into_iter()
        .find(|u| u.name == user)
        .map(|u| u.id)
        .ok_or_else(|| format!("Headscale has no user named \"{user}\""))?;
    let created: HeadscaleCreated = call(
        "Headscale",
        crate::http()
            .post(format!("{base}/api/v1/preauthkey"))
            .json(&headscale_request(&id, chrono::Utc::now())),
        api_key,
        REFUSED,
    )
    .await?;
    Ok(created.pre_auth_key.key)
}

async fn mint_tailscale(api_key: &str) -> Result<String, String> {
    let created: AuthKey = call(
        "Tailscale",
        crate::http()
            .post(TAILSCALE_KEYS_URL)
            .json(&tailscale_request()),
        api_key,
        "Tailscale refused the API access token — tokens last at most 90 days. Generate a new one at https://login.tailscale.com/admin/settings/keys (Generate access token).",
    )
    .await?;
    Ok(created.key)
}

pub async fn resolve(control_server: &str, key: &str, user: &str) -> Result<String, String> {
    let key = key.trim();
    let self_hosted = !control_server.trim().is_empty();
    match (key, self_hosted) {
        (k, true) if k.starts_with(HEADSCALE_API_PREFIX) => {
            mint_headscale(control_server, k, user).await
        }
        (k, false) if k.starts_with(HEADSCALE_API_PREFIX) => {
            Err("A Headscale API key needs the Headscale control server URL".to_string())
        }
        (k, false) if k.starts_with(TAILSCALE_API_PREFIX) => mint_tailscale(k).await,
        (k, true) if k.starts_with(TAILSCALE_API_PREFIX) => Err(
            "A Tailscale API access token only works with Tailscale, not a self-hosted control server"
                .to_string(),
        ),
        (k, _) => Ok(k.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_api_keys_mint() {
        assert!(is_api_key(" hskey-api-abc-def"));
        assert!(is_api_key("tskey-api-abc-def"));
        assert!(!is_api_key("hskey-auth-abc-def"));
        assert!(!is_api_key("tskey-auth-abc"));
    }

    #[test]
    fn headscale_keys_are_one_time_and_outlive_the_trip_to_first_boot() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z")
            .unwrap()
            .to_utc();
        let body = headscale_request("1", now);
        assert_eq!(body["user"], "1");
        assert_eq!(body["reusable"], false);
        assert_eq!(body["ephemeral"], false);
        assert_eq!(body["expiration"], "2027-02-01T12:00:00+00:00");
    }

    #[test]
    fn tailscale_keys_are_one_time_preapproved_and_last_ninety_days() {
        let body = tailscale_request();
        let create = &body["capabilities"]["devices"]["create"];
        assert_eq!(create["reusable"], false);
        assert_eq!(create["ephemeral"], false);
        assert_eq!(create["preauthorized"], true);
        assert_eq!(body["expirySeconds"], 7_776_000);
    }

    #[tokio::test]
    async fn pre_auth_keys_pass_through_and_mismatched_api_keys_are_refused() {
        assert_eq!(
            resolve("", " tskey-auth-abc ", "").await,
            Ok("tskey-auth-abc".to_string())
        );
        assert_eq!(
            resolve("https://hs.example.com", "hskey-auth-abc", "").await,
            Ok("hskey-auth-abc".to_string())
        );
        assert!(resolve("", "hskey-api-abc-def", "pilots")
            .await
            .unwrap_err()
            .contains("control server URL"));
        assert!(resolve("https://hs.example.com", "tskey-api-abc", "")
            .await
            .unwrap_err()
            .contains("only works with Tailscale"));
    }
}
