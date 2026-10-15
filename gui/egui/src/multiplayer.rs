//! Multiplayer panel: LAN host / join and the readiness light.
//!
//! ```text
//!  App::update
//!    Multiplayer::tick(emu)   session.tick (join worker, ENet pump while no
//!                             console runs), re-attach the console's link when
//!                             a session starts/ends, evaluate the light
//!    Multiplayer::ui          [Host] [Leave]  address [Join]  ● ready / not
//!  App::run_emulator
//!    Multiplayer::link()      NetLink for the new console
//! ```
//!
//! While a join is in progress (`busy`) the console is not run: melonDS's
//! LAN backend is driven from the join worker then.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ds_emu::Emulator;
use ds_net::NetLink;
use ds_net_gui::{
    LanSession, Light, PeerState, PlayerStatus, Readiness, ReadyLight, ReadyProbe, SessionState,
};
use eframe::egui;

use crate::config::Config;

/// How long a console may wait for its peers per frame before the frame
/// carries on without them (see `NetLink`): the emulator runs on the UI
/// thread, which must not freeze when the other side stalls.
const STALL_BUDGET: Duration = Duration::from_millis(100);

const GREEN: egui::Color32 = egui::Color32::from_rgb(0x2E, 0xCC, 0x40);
const RED: egui::Color32 = egui::Color32::from_rgb(0xE5, 0x39, 0x35);

pub struct Multiplayer {
    session: LanSession,
    probe: ReadyProbe,
    light: ReadyLight,
    last: Light,
    /// Session generation the running console's link was made in.
    link_generation: u64,
    /// Game code of a ROM file, cached by path.
    game_code: Option<(PathBuf, [u8; 4])>,
    error: Option<String>,
    /// The host address box follows the hosts found on the LAN until the
    /// user types in it (clearing it hands it back).
    address_auto: bool,
    /// Hosts found on the LAN (address, game).
    hosts: Vec<(IpAddr, Option<String>)>,
    /// This machine's LAN address, to tell the other players.
    local_ip: Option<IpAddr>,
}

impl Multiplayer {
    /// Selects melonDS's LAN backend (before any console runs) and starts
    /// the readiness probe, repainting `ctx` on every answer.
    pub fn new(ctx: &egui::Context) -> Self {
        let session = LanSession::new();
        let probe = ReadyProbe::start();
        let ctx = ctx.clone();
        probe.on_update(move || ctx.request_repaint());
        Self {
            session,
            probe,
            light: ReadyLight::new(),
            last: Light {
                ready: false,
                text: String::new(),
                peers: Vec::new(),
            },
            link_generation: 0,
            game_code: None,
            error: None,
            address_auto: true,
            hosts: Vec::new(),
            local_ip: ds_net_gui::local_ip(),
        }
    }

    /// A join is in progress: do not run the console this frame.
    pub const fn busy(&self) -> bool {
        self.session.busy()
    }

    /// In a LAN session: the speed stays at 1x so the peers keep in step.
    pub const fn in_session(&self) -> bool {
        self.session.state().in_session()
    }

    /// The Wi-Fi link for a new console (instance 0: one console per
    /// process).
    pub fn link(&mut self) -> Box<dyn ds_emu::MpLink> {
        self.link_generation = self.session.generation();
        Box::new(NetLink::new(0).with_stall_budget(Some(STALL_BUDGET)))
    }

    /// Once per GUI frame, before the console runs.
    pub fn tick(&mut self, emu: Option<&mut Emulator>, running: bool, config: &mut Config) {
        self.session.tick(running && emu.is_some() && !self.busy());

        // Look for hosts while not in a session, and prefill the address
        // box with the nearest one (unless the user typed an address).
        let offline = !self.session.state().in_session() && !self.busy();
        self.probe.set_discover(offline);
        self.hosts = self
            .probe
            .hosts()
            .into_iter()
            .map(|(ip, state)| (ip, state.game()))
            .collect();
        if offline
            && self.address_auto
            && let Some((ip, _)) = self.hosts.first()
        {
            let ip = ip.to_string();
            if config.lan_address != ip {
                config.lan_address = ip;
            }
        }

        // melonDS puts a console on a LAN session only when it powers its
        // Wi-Fi on; a running console re-joins through a fresh link.
        let game = match emu {
            Some(emu) => {
                if self.session.generation() != self.link_generation && !self.busy() {
                    log::info!("LAN: session changed, re-attaching the console");
                    emu.set_mp_link(Some(self.link()));
                }
                emu.cart
                    .rom
                    .get(0x0C..0x10)
                    .map_or([0; 4], |code| [code[0], code[1], code[2], code[3]])
            }
            None => config
                .selected_rom
                .as_deref()
                .map_or([0; 4], |path| self.rom_game_code(path)),
        };

        let local = PeerState {
            running: running && game != [0; 4],
            game_code: game,
            ..PeerState::default()
        };
        self.last = self
            .light
            .evaluate(&self.session, &self.probe, &config.lan_address, local);
    }

    /// Game code (header 0x0C..0x10) of a ROM file.
    fn rom_game_code(&mut self, path: &Path) -> [u8; 4] {
        if let Some((cached, code)) = &self.game_code
            && cached == path
        {
            return *code;
        }
        let code = read_game_code(path).unwrap_or([0; 4]);
        self.game_code = Some((path.to_owned(), code));
        code
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, config: &mut Config, mac: &str, instance: u32) {
        let busy = self.busy();
        let in_session = self.session.state().in_session();

        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut config.lan_name).desired_width(90.0));
            ui.separator();
            ui.add_enabled_ui(!busy && !in_session, |ui| {
                if ui.button("Host").clicked() {
                    self.error = self
                        .session
                        .host(&config.lan_name)
                        .err()
                        .map(|e| e.to_string());
                }
            });
            ui.separator();
            ui.label("Host address:");
            let edit = ui.add_enabled(
                !busy && !in_session,
                egui::TextEdit::singleline(&mut config.lan_address)
                    .hint_text("192.168.0.2")
                    .desired_width(130.0),
            );
            if edit.changed() {
                self.address_auto = config.lan_address.trim().is_empty();
            }
            ui.add_enabled_ui(
                !busy && !in_session && !config.lan_address.trim().is_empty(),
                |ui| {
                    if ui.button("Join").clicked() {
                        self.error = None;
                        let address = config.lan_address.trim().to_owned();
                        self.session.join(&config.lan_name, &address);
                    }
                },
            );
            ui.add_enabled_ui(in_session, |ui| {
                if ui.button("Leave").clicked() {
                    self.session.leave();
                }
            });
            ui.separator();
            ui.small(format!("instance{instance}  MAC {mac}"));
        });

        if !in_session {
            ui.horizontal(|ui| {
                ui.small(format!(
                    "This PC: {}",
                    self.local_ip
                        .map_or_else(|| "-".to_owned(), |ip| ip.to_string())
                ));
                ui.separator();
                if self.hosts.is_empty() {
                    ui.small("Hosts found: none yet");
                } else {
                    ui.small("Hosts found:");
                    for (ip, game) in &self.hosts {
                        let label = match game {
                            Some(game) => format!("{ip} ({game})"),
                            None => ip.to_string(),
                        };
                        if ui.small_button(label).clicked() {
                            config.lan_address = ip.to_string();
                            self.address_auto = false;
                        }
                    }
                }
            });
        }

        ui.horizontal(|ui| {
            let (color, verdict) = if self.last.ready {
                (GREEN, "Ready")
            } else {
                (RED, "Not ready")
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 6.0, color);
            ui.colored_label(color, verdict);
            ui.label(format!("- {}", self.last.text));
            ui.separator();
            ui.label(match self.session.state() {
                SessionState::Offline => "offline".to_owned(),
                SessionState::Hosting => "hosting".to_owned(),
                SessionState::Joining(a) => format!("joining {a}..."),
                SessionState::Joined(a) => format!("joined {a}"),
                SessionState::Failed(why) => format!("failed: {why}"),
            });
            if let Some(error) = &self.error {
                ui.colored_label(RED, error);
            }
        });

        if in_session {
            for player in self.session.players() {
                let status = match player.status {
                    PlayerStatus::Host => "host",
                    PlayerStatus::Client => "client",
                    PlayerStatus::Connecting => "connecting",
                    PlayerStatus::Disconnected => "disconnected",
                    PlayerStatus::None => "-",
                };
                let probe = self
                    .last
                    .peers
                    .iter()
                    .find(|p| p.address == std::net::IpAddr::V4(player.address))
                    .map(|p| &p.readiness);
                ui.horizontal(|ui| {
                    let color = match probe {
                        _ if player.is_local => ui.visuals().weak_text_color(),
                        Some(Readiness::Ready { .. }) => GREEN,
                        _ => RED,
                    };
                    ui.colored_label(color, "●");
                    ui.label(format!(
                        "{}{}  {status}  {}  ping {} ms",
                        player.name,
                        if player.is_local { " (you)" } else { "" },
                        player.address,
                        player.ping_ms,
                    ));
                });
            }
        }
    }
}

/// Reads the 4-byte game code at 0x0C of a ROM file.
fn read_game_code(path: &Path) -> Option<[u8; 4]> {
    use std::io::Read as _;
    let mut header = [0; 0x10];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    Some([header[0x0C], header[0x0D], header[0x0E], header[0x0F]])
}
