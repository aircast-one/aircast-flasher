use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use flasher_ops::flash::FlashEvent;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    Implementation, ProgressNotificationParam, RequestMetaObject, ServerCapabilities, ServerConfig,
};
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{fleet, request, session};

const INSTRUCTIONS: &str = "Aircast Flasher: write Aircast OS to SD cards for Raspberry Pi drones and manage the fleet. \
Flashing erases the whole card: call list_cards, name the exact disk, and repeat it in confirmErase. \
Writing asks for an administrator password on this computer unless the server runs as root. \
After a flash, find_device locates the drone on the LAN once it boots; list_devices shows drones on the tailnet.";

#[derive(Clone)]
pub struct Flasher {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelParams {
    #[schemars(
        description = "stable, staging or development. Defaults to the first with releases."
    )]
    pub channel: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadParams {
    #[schemars(description = "Version, e.g. v0.3.6. Defaults to the newest stable release.")]
    pub release: Option<String>,
    pub channel: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlashCardParams {
    #[serde(flatten)]
    pub request: request::FlashRequest,
    #[schemars(description = "Repeat the disk path to confirm the card may be erased.")]
    pub confirm_erase: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FindParams {
    #[schemars(description = "Hostname the card was flashed with, e.g. falcon-01.")]
    pub hostname: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceParams {
    #[schemars(description = "Device address or name, e.g. falcon-01.local or http://10.42.0.1.")]
    pub target: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoginParams {
    #[schemars(description = "Device address or name, e.g. falcon-01.local.")]
    pub target: String,
    #[schemars(description = "Point the device at this Headscale server first.")]
    pub control_server: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignInFinishParams {
    #[schemars(description = "The token cloud_sign_in returned.")]
    pub token: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkLostParams {
    #[schemars(description = "Card id from cloud_list_cards.")]
    pub id: String,
    #[schemars(description = "Repeat the card's label to confirm. Marking lost can't be undone.")]
    pub confirm_label: String,
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

fn cancel_flag(token: CancellationToken) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let set = flag.clone();
    tokio::spawn(async move {
        token.cancelled().await;
        set.store(true, Ordering::SeqCst);
    });
    flag
}

fn progress_sender(
    meta: &RequestMetaObject,
    peer: Peer<RoleServer>,
) -> tokio::sync::mpsc::UnboundedSender<(String, u64, u64)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, u64, u64)>();
    let token = meta.get_progress_token();
    tokio::spawn(async move {
        while let Some((message, done, total)) = rx.recv().await {
            if let Some(token) = &token {
                let param = ProgressNotificationParam::new(token.clone(), done as f64)
                    .with_message(message);
                let param = match total {
                    0 => param,
                    total => param.with_total(total as f64),
                };
                let _ = peer.notify_progress(param).await;
            }
        }
    });
    tx
}

#[tool_router]
impl Flasher {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List Aircast OS releases that can be written to a card.")]
    async fn list_releases(
        &self,
        Parameters(p): Parameters<ChannelParams>,
    ) -> Result<String, String> {
        json(&flasher_ops::releases::list_releases(p.channel).await?)
    }

    #[tool(
        description = "List the removable SD cards this computer can write, with their disk paths."
    )]
    async fn list_cards(&self) -> Result<String, String> {
        json(&flasher_ops::devices::list_block_devices().await?)
    }

    #[tool(
        description = "Download an Aircast OS image into the shared cache and return its path. flash_card downloads on its own; use this to fetch ahead of time."
    )]
    async fn download_image(
        &self,
        Parameters(p): Parameters<DownloadParams>,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> Result<String, String> {
        let release = request::pick_release(p.release.as_deref(), p.channel).await?;
        let progress = progress_sender(&meta, peer);
        let path = request::download(&release, &cancel_flag(cancel), &mut |d| {
            let _ = progress.send(("Downloading".into(), d.downloaded_bytes, d.total_bytes));
        })
        .await?;
        json(&serde_json::json!({ "version": release.version, "path": path }))
    }

    #[tool(
        description = "Erase an SD card, write Aircast OS (or a local image) and configure WiFi, hostname, SSH, tailnet, disk encryption and fight mode. Takes a few minutes and asks for an administrator password unless the server runs as root."
    )]
    async fn flash_card(
        &self,
        Parameters(p): Parameters<FlashCardParams>,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> Result<String, String> {
        let request = p.request;
        if p.confirm_erase.trim() != request.disk.trim() {
            return Err(format!(
                "confirmErase must repeat the disk ({}) — flashing erases the whole card",
                request.disk
            ));
        }
        let problems = request::problems(&request);
        if !problems.is_empty() {
            return Err(format!(
                "Fix this to continue:\n- {}",
                problems.join("\n- ")
            ));
        }

        let card = request::find_card(&request.disk).await?;
        let cancel = cancel_flag(cancel);
        let progress = progress_sender(&meta, peer);
        let image = match &request.image {
            Some(path) => path.clone(),
            None => {
                let release =
                    request::pick_release(request.release.as_deref(), request.channel.clone())
                        .await?;
                request::download(&release, &cancel, &mut |d| {
                    let _ =
                        progress.send(("Downloading".into(), d.downloaded_bytes, d.total_bytes));
                })
                .await?
            }
        };
        let events = Arc::new(move |event: FlashEvent| {
            if let FlashEvent::Progress(p) = event {
                let phase = serde_json::to_value(p.phase)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                let _ = progress.send((phase, p.bytes_processed, p.total_bytes));
            }
        });
        json(&request::run(&request, request::Prepared { card, image }, cancel, events).await?)
    }

    #[tool(
        description = "List Aircast devices answering on this computer's Tailscale or Headscale tailnet."
    )]
    async fn list_devices(&self) -> Result<String, String> {
        json(&fleet::devices().await?)
    }

    #[tool(
        description = "Find a freshly flashed device on this network (hostname.local, or the device's own hotspot). Returns null until it has booted."
    )]
    async fn find_device(&self, Parameters(p): Parameters<FindParams>) -> Result<String, String> {
        json(&serde_json::json!({ "address": fleet::find(&p.hostname).await }))
    }

    #[tool(description = "Whether a device is signed in to a tailnet, and under which name.")]
    async fn device_status(
        &self,
        Parameters(p): Parameters<DeviceParams>,
    ) -> Result<String, String> {
        json(&fleet::status(&p.target).await?)
    }

    #[tool(
        description = "Start Tailscale sign-in on a device. Returns the URL a person must open to approve it, or an empty URL when it is already signed in."
    )]
    async fn device_tailscale_login(
        &self,
        Parameters(p): Parameters<DeviceLoginParams>,
    ) -> Result<String, String> {
        json(&serde_json::json!({
            "authUrl": fleet::login(&p.target, p.control_server.as_deref()).await?
        }))
    }

    #[tool(description = "Who is signed in to Aircast Cloud on this computer, if anyone.")]
    async fn cloud_account(&self) -> Result<String, String> {
        json(&serde_json::json!({ "email": session::load().map(|s| s.email) }))
    }

    #[tool(
        description = "Start an Aircast Cloud sign-in. Opens the browser and returns the URL to approve plus a token for cloud_sign_in_finish."
    )]
    async fn cloud_sign_in(&self) -> Result<String, String> {
        let start = flasher_ops::cloud::sign_in_start().await?;
        let _ = open::that(&start.auth_url);
        json(&serde_json::json!({ "authUrl": start.auth_url, "token": start.token }))
    }

    #[tool(
        description = "Wait for the person to approve the Aircast Cloud sign-in, then keep the session on this computer."
    )]
    async fn cloud_sign_in_finish(
        &self,
        Parameters(p): Parameters<SignInFinishParams>,
    ) -> Result<String, String> {
        let email = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            session::wait_for_sign_in(&p.token),
        )
        .await
        .map_err(|_| "Nobody approved the sign-in within 5 minutes — start again".to_string())??;
        json(&serde_json::json!({ "email": email }))
    }

    #[tool(
        description = "List the cards that unlock through Aircast Cloud, with when each last unlocked and whether it is marked lost."
    )]
    async fn cloud_list_cards(&self) -> Result<String, String> {
        json(&session::forget_if_signed_out(
            flasher_ops::cloud::list_cards(&session::token()).await,
        )?)
    }

    #[tool(
        description = "Mark a Cloud-unlock card lost so it can never unlock again. Can't be undone."
    )]
    async fn cloud_mark_lost(
        &self,
        Parameters(p): Parameters<MarkLostParams>,
    ) -> Result<String, String> {
        let token = session::token();
        let cards = session::forget_if_signed_out(flasher_ops::cloud::list_cards(&token).await)?;
        let card = cards
            .iter()
            .find(|card| card.id == p.id)
            .ok_or_else(|| format!("No Cloud card {}", p.id))?;
        if card.label != p.confirm_label {
            return Err(format!(
                "confirmLabel must repeat the card's label ({})",
                card.label
            ));
        }
        session::forget_if_signed_out(flasher_ops::cloud::mark_lost(&token, &p.id).await)?;
        json(&serde_json::json!({ "lost": p.id }))
    }
}

#[tool_handler]
impl ServerHandler for Flasher {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "aircast-flasher",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

pub async fn serve() -> Result<(), String> {
    let service = Flasher::new()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("MCP server failed to start: {e}"))?;
    service
        .waiting()
        .await
        .map(|_| ())
        .map_err(|e| format!("MCP server stopped: {e}"))
}
