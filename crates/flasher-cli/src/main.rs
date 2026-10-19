mod fleet;
mod mcp;
mod request;
mod session;

use std::io::{BufRead as _, IsTerminal as _};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::{ArgGroup, Parser, Subcommand};
use flasher_ops::flash::FlashEvent;
use flasher_ops::{BlockDevice, FlashPhase};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

use request::{
    EncryptionRequest, FlashRequest, SshChoice, SshRequest, TailnetRequest, WifiRequest,
};

#[derive(Parser)]
#[command(
    name = "aircast-flasher-cli",
    version,
    about = "Flash Aircast SD cards and manage the fleet from the command line. `mcp` serves the same tools to agents."
)]
struct Cli {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "List Aircast OS releases")]
    Releases {
        #[arg(long, help = "stable, staging or development")]
        channel: Option<String>,
    },
    #[command(about = "List the SD cards this computer can write")]
    Cards,
    #[command(about = "Download an Aircast OS image into the shared cache")]
    Download {
        #[arg(
            long,
            help = "Version to download, e.g. v0.3.6 (default: newest stable)"
        )]
        release: Option<String>,
        #[arg(long, help = "stable, staging or development")]
        channel: Option<String>,
    },
    #[command(about = "Erase an SD card, write Aircast OS and configure it")]
    Flash(Box<FlashArgs>),
    #[command(about = "List Aircast devices answering on your tailnet")]
    Devices,
    #[command(about = "Find a freshly flashed device on this network by hostname")]
    Find { hostname: String },
    #[command(subcommand, about = "Inspect a device or sign it in to a tailnet")]
    Device(DeviceCommand),
    #[command(subcommand, about = "Aircast Cloud: sign-in and Cloud-unlock cards")]
    Cloud(CloudCommand),
    #[command(about = "Serve these tools over MCP on stdio")]
    Mcp,
}

#[derive(clap::Args)]
#[command(group(ArgGroup::new("login").args(["ssh_key", "ssh_password", "no_ssh"])))]
struct FlashArgs {
    #[arg(long, help = "Card to erase, as `cards` lists it (e.g. /dev/disk4)")]
    disk: String,
    #[arg(
        long,
        conflicts_with = "release",
        help = "Local .img, .img.xz or .img.gz"
    )]
    image: Option<String>,
    #[arg(long, help = "Aircast OS version (default: newest stable)")]
    release: Option<String>,
    #[arg(long, help = "stable, staging or development")]
    channel: Option<String>,
    #[arg(long)]
    hostname: Option<String>,
    #[arg(long, help = "WiFi network the device joins on first boot")]
    wifi_ssid: Option<String>,
    #[arg(
        long,
        env = "AIRCAST_WIFI_PASSWORD",
        hide_env_values = true,
        default_value = ""
    )]
    wifi_password: String,
    #[arg(long, help = "Two-letter country (default: this computer's)")]
    wifi_country: Option<String>,
    #[arg(
        long,
        help = "SSH public key text or .pub file (default: your only key in ~/.ssh)"
    )]
    ssh_key: Option<String>,
    #[arg(long, env = "AIRCAST_DEVICE_PASSWORD", hide_env_values = true)]
    ssh_password: Option<String>,
    #[arg(long, help = "Turn SSH off")]
    no_ssh: bool,
    #[arg(
        long,
        env = "AIRCAST_TAILSCALE_KEY",
        hide_env_values = true,
        help = "Tailnet pre-auth key"
    )]
    tailscale_key: Option<String>,
    #[arg(
        long,
        requires = "tailscale_key",
        help = "Headscale control server URL"
    )]
    control_server: Option<String>,
    #[arg(
        long = "tang",
        help = "Tang key server URL (repeat for more); turns on disk encryption"
    )]
    tang: Vec<String>,
    #[arg(long, help = "Key servers needed to unlock (default 1)")]
    threshold: Option<u32>,
    #[arg(long, help = "Also unlock through Aircast Cloud (needs `cloud login`)")]
    cloud_unlock: bool,
    #[arg(long, help = "Where to save the recovery key when encrypting")]
    recovery_file: Option<String>,
    #[arg(
        long = "fight",
        help = "Fight mode: no logs on the drone, tailnet-only, SSH key only"
    )]
    fight_mode: bool,
    #[arg(long, short = 'y', help = "Erase without asking")]
    yes: bool,
}

#[derive(Subcommand)]
enum DeviceCommand {
    #[command(about = "Whether the device is on a tailnet")]
    Status { target: String },
    #[command(about = "Start Tailscale sign-in on the device and print the URL to approve")]
    Login {
        target: String,
        #[arg(long, help = "Point the device at a Headscale server first")]
        control_server: Option<String>,
    },
}

#[derive(Subcommand)]
enum CloudCommand {
    #[command(about = "Sign in with Google in the browser")]
    Login,
    #[command(about = "Forget the Cloud session on this computer")]
    Logout,
    #[command(about = "Show who is signed in")]
    Whoami,
    #[command(about = "List cards that unlock through Aircast Cloud")]
    Cards,
    #[command(about = "Mark a card lost so it never unlocks again — can't be undone")]
    Lost {
        id: String,
        #[arg(long, short = 'y', help = "Don't ask")]
        yes: bool,
    },
}

impl FlashArgs {
    fn request(&self) -> FlashRequest {
        let encrypting = !self.tang.is_empty() || self.cloud_unlock || self.recovery_file.is_some();
        FlashRequest {
            disk: self.disk.clone(),
            image: self.image.clone(),
            release: self.release.clone(),
            channel: self.channel.clone(),
            hostname: self.hostname.clone(),
            wifi: self.wifi_ssid.as_ref().map(|ssid| WifiRequest {
                ssid: ssid.clone(),
                password: self.wifi_password.clone(),
                country: self.wifi_country.clone(),
            }),
            ssh: match (&self.ssh_key, &self.ssh_password, self.no_ssh) {
                (_, _, true) => Some(SshRequest {
                    mode: SshChoice::Off,
                    key: None,
                    password: None,
                }),
                (_, Some(password), _) => Some(SshRequest {
                    mode: SshChoice::Password,
                    key: None,
                    password: Some(password.clone()),
                }),
                (Some(key), _, _) => Some(SshRequest {
                    mode: SshChoice::Key,
                    key: Some(key.clone()),
                    password: None,
                }),
                _ => None,
            },
            tailscale: self.tailscale_key.as_ref().map(|key| TailnetRequest {
                auth_key: key.clone(),
                control_server: self.control_server.clone(),
            }),
            encryption: encrypting.then(|| EncryptionRequest {
                tang_servers: self.tang.clone(),
                threshold: self.threshold,
                cloud_unlock: self.cloud_unlock,
                recovery_file: self.recovery_file.clone().unwrap_or_default(),
            }),
            fight_mode: self.fight_mode,
        }
    }
}

fn say(line: String) -> Result<(), String> {
    println!("{line}");
    Ok(())
}

fn tell(line: String) -> Result<(), String> {
    eprintln!("{line}");
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    serde_json::to_string_pretty(value)
        .map(|json| println!("{json}"))
        .map_err(|e| e.to_string())
}

fn byte_bar(message: &str) -> ProgressBar {
    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template(
            "{msg:12} [{bar:32}] {bytes}/{total_bytes} {binary_bytes_per_sec} {eta}",
        )
        .expect("progress template")
        .progress_chars("=> "),
    );
    bar.set_message(message.to_string());
    bar
}

fn phase_name(phase: FlashPhase) -> &'static str {
    match phase {
        FlashPhase::Decompressing => "Unpacking",
        FlashPhase::Writing => "Writing",
        FlashPhase::Verifying => "Verifying",
        FlashPhase::Customizing => "Configuring",
    }
}

fn card_line(card: &BlockDevice) -> String {
    let mounted = match card.mount_points.as_slice() {
        [] => String::new(),
        points => format!("  mounted at {}", points.join(", ")),
    };
    format!("{}  {}  {}{mounted}", card.path, card.name, card.size_human)
}

fn confirm_erase(card: &BlockDevice, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "Refusing to erase {} without a prompt — pass --yes",
            card.path
        ));
    }
    eprintln!("This erases everything on {}.", card_line(card));
    eprint!("Type {} to continue: ", card.path);
    let answer = std::io::stdin()
        .lock()
        .lines()
        .next()
        .and_then(Result::ok)
        .unwrap_or_default();
    match answer.trim() == card.path {
        true => Ok(()),
        false => Err("Not erased".to_string()),
    }
}

fn cancel_on_ctrl_c() -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            flag.store(true, Ordering::SeqCst);
        }
    });
    cancel
}

async fn flash(args: &FlashArgs, json: bool) -> Result<(), String> {
    let request = args.request();
    let problems = request::problems(&request);
    if !problems.is_empty() {
        return Err(format!(
            "Fix this to continue:\n{}",
            problems
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let card = request::find_card(&request.disk).await?;
    let release = match &request.image {
        Some(_) => None,
        None => {
            Some(request::pick_release(request.release.as_deref(), request.channel.clone()).await?)
        }
    };
    if let Some(release) = &release {
        eprintln!("Image: Aircast OS {}", release.version);
    }
    confirm_erase(&card, args.yes)?;

    let cancel = cancel_on_ctrl_c();
    let image = match (&request.image, &release) {
        (Some(path), _) => path.clone(),
        (None, Some(release)) => {
            let bar = byte_bar("Downloading");
            let path = request::download(release, &cancel, &mut |p| {
                bar.set_length(p.total_bytes);
                bar.set_position(p.downloaded_bytes);
            })
            .await;
            bar.finish_and_clear();
            path?
        }
        (None, None) => unreachable!("a release is picked whenever no image is given"),
    };

    let bar = byte_bar("Preparing");
    let events = {
        let bar = bar.clone();
        Arc::new(move |event: FlashEvent| {
            if let FlashEvent::Progress(p) = event {
                bar.set_message(phase_name(p.phase));
                bar.set_length(p.total_bytes.max(p.bytes_processed));
                bar.set_position(p.bytes_processed);
            }
        })
    };
    let outcome = request::run(&request, request::Prepared { card, image }, cancel, events).await;
    bar.finish_and_clear();
    let outcome = outcome?;

    match json {
        true => print_json(&outcome),
        false => {
            println!(
                "Flashed {} — remove the card and boot the device.",
                outcome.disk
            );
            println!("SSH: {}", outcome.ssh);
            if let Some(address) = &outcome.address {
                println!(
                    "Once it boots: {address}  (`aircast-flasher-cli find {}`)",
                    outcome.hostname.as_deref().unwrap_or("")
                );
            }
            if let Some(file) = &outcome.recovery_file {
                println!("Recovery key saved to {file} — keep it secret, it's the only copy.");
            }
            Ok(())
        }
    }
}

async fn cloud(command: &CloudCommand, json: bool) -> Result<(), String> {
    match command {
        CloudCommand::Login => {
            let email = session::sign_in(|url| {
                eprintln!("Approve the sign-in in your browser: {url}");
            })
            .await?;
            println!("Signed in as {email}");
            Ok(())
        }
        CloudCommand::Logout => session::clear().map(|()| println!("Signed out")),
        CloudCommand::Whoami => match (session::load(), json) {
            (Some(s), true) => print_json(&serde_json::json!({ "email": s.email })),
            (Some(s), false) => say(s.email.to_string()),
            (None, _) => Err(flasher_ops::cloud::SIGNED_OUT.to_string()),
        },
        CloudCommand::Cards => {
            let cards = session::forget_if_signed_out(
                flasher_ops::cloud::list_cards(&session::token()).await,
            )?;
            match json {
                true => print_json(&cards),
                false => {
                    cards.iter().for_each(|card| {
                        let state = match (&card.lost_at, &card.last_unlock_at) {
                            (Some(at), _) => format!("lost {at}"),
                            (None, Some(at)) => format!("last unlocked {at}"),
                            (None, None) => "not unlocked yet".to_string(),
                        };
                        println!("{}  {}  {state}", card.id, card.label);
                    });
                    Ok(())
                }
            }
        }
        CloudCommand::Lost { id, yes } => {
            if !yes {
                return Err(format!(
                    "Marking {id} lost can't be undone — the card never unlocks again. Pass --yes"
                ));
            }
            session::forget_if_signed_out(
                flasher_ops::cloud::mark_lost(&session::token(), id).await,
            )
            .map(|()| println!("Marked {id} lost"))
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    let json = cli.json;
    match &cli.command {
        Command::Releases { channel } => {
            let releases = flasher_ops::releases::list_releases(channel.clone()).await?;
            match json {
                true => print_json(&releases),
                false => {
                    releases.releases.iter().for_each(|r| {
                        println!(
                            "{:14} {}  {:>9}{}",
                            r.version,
                            r.created_at.get(..10).unwrap_or(&r.created_at),
                            flasher_ops::format_bytes(r.image.size),
                            if r.prerelease { "  prerelease" } else { "" }
                        );
                    });
                    Ok(())
                }
            }
        }
        Command::Cards => {
            let cards = flasher_ops::devices::list_block_devices().await?;
            match (json, cards.is_empty()) {
                (true, _) => print_json(&cards),
                (false, true) => tell("No SD card found — insert one.".to_string()),
                (false, false) => say(cards.iter().map(card_line).collect::<Vec<_>>().join("\n")),
            }
        }
        Command::Download { release, channel } => {
            let release = request::pick_release(release.as_deref(), channel.clone()).await?;
            let bar = byte_bar("Downloading");
            let cancel = cancel_on_ctrl_c();
            let path = request::download(&release, &cancel, &mut |p| {
                bar.set_length(p.total_bytes);
                bar.set_position(p.downloaded_bytes);
            })
            .await;
            bar.finish_and_clear();
            let path = path?;
            match json {
                true => {
                    print_json(&serde_json::json!({ "version": release.version, "path": path }))
                }
                false => say(path.to_string()),
            }
        }
        Command::Flash(args) => flash(args, json).await,
        Command::Devices => {
            let fleet = fleet::devices().await?;
            match json {
                true => print_json(&fleet),
                false => {
                    fleet.devices.iter().for_each(|d| {
                        println!("{:20} {}  {}", d.name, d.address, d.ip);
                    });
                    if fleet.devices.is_empty() {
                        eprintln!("No Aircast device is answering on {}.", fleet.tailnet);
                    }
                    if fleet.offline > 0 {
                        eprintln!("{} more offline.", fleet.offline);
                    }
                    Ok(())
                }
            }
        }
        Command::Find { hostname } => match (fleet::find(hostname).await, json) {
            (found, true) => print_json(&serde_json::json!({ "address": found })),
            (Some(address), false) => say(address.to_string()),
            (None, false) => Err(format!(
                "{hostname} isn't answering yet — it can take a few minutes after first power-on"
            )),
        },
        Command::Device(DeviceCommand::Status { target }) => {
            let state = fleet::status(target).await?;
            match json {
                true => print_json(&state),
                false => say((match state.connected {
                    true => format!("On the tailnet as {}", state.name),
                    false => "Not on a tailnet".to_string(),
                })
                .to_string()),
            }
        }
        Command::Device(DeviceCommand::Login {
            target,
            control_server,
        }) => {
            let url = fleet::login(target, control_server.as_deref()).await?;
            match (json, url.is_empty()) {
                (true, _) => print_json(&serde_json::json!({ "authUrl": url })),
                (false, true) => say("Already signed in".to_string()),
                (false, false) => say(format!("Approve the device: {url}")),
            }
        }
        Command::Cloud(command) => cloud(command, json).await,
        Command::Mcp => mcp::serve().await,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args().any(|arg| arg == "--flash-helper") {
        flasher_ops::helper::run_flash_helper();
    }
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
