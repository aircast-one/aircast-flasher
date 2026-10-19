use std::sync::Mutex;

use flasher_ops::cloud::{self, CloudCard, CloudKeyset, SignIn, SignInStatus, SIGNED_OUT};
use tauri::State;

struct Session {
    token: String,
    email: String,
}

#[derive(Default)]
pub struct CloudState(Mutex<Option<Session>>);

impl CloudState {
    fn token(&self) -> Result<String, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "cloud session lock poisoned".to_string())?
            .as_ref()
            .map(|s| s.token.clone())
            .unwrap_or_default())
    }

    fn set(&self, session: Option<Session>) -> Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "cloud session lock poisoned".to_string())? = session;
        Ok(())
    }

    fn forget_if_signed_out<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if result.as_ref().is_err_and(|e| e == SIGNED_OUT) {
            self.set(None)?;
        }
        result
    }
}

#[tauri::command]
pub async fn cloud_sign_in_start() -> Result<SignIn, String> {
    cloud::sign_in_start().await
}

#[tauri::command]
pub async fn cloud_sign_in_poll(
    state: State<'_, CloudState>,
    token: String,
) -> Result<SignInStatus, String> {
    let status = cloud::sign_in_poll(&token).await?;
    if let SignInStatus::SignedIn {
        email,
        session_token,
    } = &status
    {
        state.set(Some(Session {
            token: session_token.clone(),
            email: email.clone(),
        }))?;
    }
    Ok(status)
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
    let result = cloud::create_keyset(&state.token()?, &label).await;
    state.forget_if_signed_out(result)
}

#[tauri::command]
pub async fn cloud_list_cards(state: State<'_, CloudState>) -> Result<Vec<CloudCard>, String> {
    let result = cloud::list_cards(&state.token()?).await;
    state.forget_if_signed_out(result)
}

#[tauri::command]
pub async fn cloud_mark_lost(state: State<'_, CloudState>, id: String) -> Result<(), String> {
    let result = cloud::mark_lost(&state.token()?, &id).await;
    state.forget_if_signed_out(result)
}
