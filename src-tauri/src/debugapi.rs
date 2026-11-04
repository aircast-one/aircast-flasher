use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

const PORT_FLAG: &str = "--debug-port";
const PORT_ENV: &str = "AIRCAST_FLASHER_DEBUG_PORT";
const ALLOW_WRITE_ENV: &str = "AIRCAST_FLASHER_DEBUG_ALLOW_WRITE";
const HEADER: &str = "x-aircast-flasher-debug";
const REQUEST_EVENT: &str = "debug://request";
const UI_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_EVENTS: usize = 20;
const WRITE_REFUSED: &str =
    "Writing a card over the debug API needs AIRCAST_FLASHER_DEBUG_ALLOW_WRITE=1 in the app's environment.";
const HEADER_MISSING: &str = "Send the header X-Aircast-Flasher-Debug: 1 with every request.";
const UI_SILENT: &str = "The Flasher window did not answer in time.";
const NOT_JSON: &str = "The body must be a JSON object such as {\"action\":\"state\"}.";

#[derive(Default)]
pub struct Pending {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
}

impl Pending {
    fn wait(&self) -> (u64, oneshot::Receiver<Value>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, sender);
        (id, receiver)
    }

    fn take(&self, id: u64) -> Option<oneshot::Sender<Value>> {
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
    }
}

#[derive(Deserialize)]
struct EventsQuery {
    tail: Option<usize>,
}

fn port(args: &[String], env: Option<String>) -> Option<u16> {
    args.windows(2)
        .find(|pair| pair[0] == PORT_FLAG)
        .and_then(|pair| pair[1].parse().ok())
        .or_else(|| env.and_then(|value| value.parse().ok()))
}

fn allows_write(value: Option<String>) -> bool {
    value.as_deref() == Some("1")
}

fn refusal(request: &Value, allow_write: bool) -> Option<&'static str> {
    (request["action"] == "flash" && !allow_write).then_some(WRITE_REFUSED)
}

fn last_lines(text: &str, count: usize) -> Vec<Value> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..]
        .iter()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| json!({ "raw": line })))
        .collect()
}

fn failed(status: StatusCode, reason: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "ok": false, "reason": reason.into() })),
    )
        .into_response()
}

pub fn start(app: &AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    let Some(port) = port(&args, std::env::var(PORT_ENV).ok()) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let router = Router::new()
            .route("/health", get(health))
            .route("/state", get(state))
            .route("/action", post(action))
            .route("/disks", get(disks))
            .route("/events", get(events))
            .layer(axum::middleware::from_fn(guard))
            .with_state(app);
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                if let Err(e) = axum::serve(listener, router).await {
                    eprintln!("debug api stopped: {e}");
                }
            }
            Err(e) => eprintln!("debug api cannot listen on 127.0.0.1:{port}: {e}"),
        }
    });
}

async fn guard(request: Request, next: Next) -> Response {
    match request.headers().get(HEADER).map(|value| value.as_bytes()) {
        Some(b"1") => next.run(request).await,
        _ => failed(StatusCode::FORBIDDEN, HEADER_MISSING),
    }
}

async fn health(State(app): State<AppHandle>) -> Json<Value> {
    Json(json!({
        "app": "aircast-flasher",
        "version": app.package_info().version.to_string(),
        "allowWrite": allows_write(std::env::var(ALLOW_WRITE_ENV).ok()),
    }))
}

async fn state(State(app): State<AppHandle>) -> Response {
    ask(&app, json!({ "action": "state" })).await
}

async fn action(State(app): State<AppHandle>, body: axum::body::Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return failed(StatusCode::BAD_REQUEST, NOT_JSON);
    };
    match refusal(&request, allows_write(std::env::var(ALLOW_WRITE_ENV).ok())) {
        Some(reason) => failed(StatusCode::FORBIDDEN, reason),
        None => ask(&app, request).await,
    }
}

async fn disks() -> Response {
    match flasher_ops::devices::list_block_devices().await {
        Ok(devices) => Json(json!(devices)).into_response(),
        Err(reason) => failed(StatusCode::INTERNAL_SERVER_ERROR, reason),
    }
}

async fn events(State(app): State<AppHandle>, Query(query): Query<EventsQuery>) -> Response {
    let count = query.tail.unwrap_or(DEFAULT_EVENTS);
    match crate::telemetry::log_path(&app)
        .and_then(|path| std::fs::read_to_string(path).map_err(|e| e.to_string()))
    {
        Ok(text) => Json(json!(last_lines(&text, count))).into_response(),
        Err(reason) => failed(StatusCode::NOT_FOUND, reason),
    }
}

async fn ask(app: &AppHandle, request: Value) -> Response {
    let pending = app.state::<Pending>();
    let (id, answer) = pending.wait();
    if let Err(e) = app.emit(REQUEST_EVENT, json!({ "id": id, "request": request })) {
        pending.take(id);
        return failed(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    match tokio::time::timeout(UI_TIMEOUT, answer).await {
        Ok(Ok(result)) => Json(result).into_response(),
        _ => {
            pending.take(id);
            failed(StatusCode::GATEWAY_TIMEOUT, UI_SILENT)
        }
    }
}

#[tauri::command]
pub fn debug_reply(id: u64, result: Value, pending: tauri::State<'_, Pending>) {
    if let Some(sender) = pending.take(id) {
        let _ = sender.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_listens_only_when_launched_with_a_port() {
        assert_eq!(port(&args(&["flasher"]), None), None);
        assert_eq!(
            port(&args(&["flasher", "--debug-port", "28800"]), None),
            Some(28800)
        );
        assert_eq!(port(&args(&["flasher"]), Some("28801".into())), Some(28801));
        assert_eq!(
            port(
                &args(&["flasher", "--debug-port", "28800"]),
                Some("28801".into())
            ),
            Some(28800),
            "the flag wins over the environment"
        );
        assert_eq!(
            port(&args(&["flasher", "--debug-port", "nope"]), None),
            None
        );
    }

    #[test]
    fn writing_a_card_needs_its_own_switch_but_driving_the_wizard_does_not() {
        let flash = json!({ "action": "flash" });
        assert_eq!(refusal(&flash, false), Some(WRITE_REFUSED));
        assert_eq!(refusal(&flash, true), None);
        assert_eq!(
            refusal(&json!({ "action": "goto", "page": "storage" }), false),
            None
        );
        assert!(!allows_write(None));
        assert!(!allows_write(Some("true".into())));
        assert!(allows_write(Some("1".into())));
    }

    #[test]
    fn the_event_log_tail_keeps_the_newest_lines_in_order() {
        let log = "{\"event\":\"a\"}\n{\"event\":\"b\"}\n\nnot json\n";
        assert_eq!(
            last_lines(log, 2),
            vec![json!({ "event": "b" }), json!({ "raw": "not json" })]
        );
        assert_eq!(last_lines(log, 10).len(), 3);
        assert!(last_lines("", 5).is_empty());
    }

    #[test]
    fn an_answer_reaches_the_request_that_asked_and_only_once() {
        let pending = Pending::default();
        let (first, mut first_answer) = pending.wait();
        let (second, _) = pending.wait();
        assert_ne!(first, second);
        let sender = pending.take(first).expect("the first request is waiting");
        sender.send(json!({ "ok": true })).unwrap();
        assert_eq!(first_answer.try_recv().unwrap(), json!({ "ok": true }));
        assert!(
            pending.take(first).is_none(),
            "a second answer finds no one waiting"
        );
    }
}
