//! The multiplayer readiness light without a GUI, for checking it across
//! processes / machines.
//!
//! ```text
//! cargo run -p ds_net_gui --example lan_probe -- --host [--secs 30]
//! cargo run -p ds_net_gui --example lan_probe -- --join <addr> [--wait 5] [--secs 30]
//! ```
//!
//! `--host` hosts a melonDS LAN session right away. `--join` first only
//! probes `<addr>` for `--wait` seconds (the light says whether joining would
//! work), then joins. Both print the session state and the light once a
//! second for `--secs` seconds.

use std::time::{Duration, Instant};

use ds_net_gui::{LanSession, PeerState, ReadyLight, ReadyProbe};

fn arg(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let host = args.iter().any(|a| a == "--host");
    let join = arg(&args, "--join");
    if !host && join.is_none() {
        eprintln!("usage: lan_probe --host | --join <addr> [--wait s] [--secs s]");
        std::process::exit(2);
    }
    let secs: u64 = arg(&args, "--secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let wait: u64 = arg(&args, "--wait")
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let name = if host { "host" } else { "guest" };

    let mut session = LanSession::new();
    let probe = ReadyProbe::start();
    let mut light = ReadyLight::new();
    let address = join.unwrap_or_default();
    println!("[{name}] probe port {:?}", probe.responder_port());

    if host && let Err(e) = session.host(name) {
        eprintln!("[{name}] host failed: {e}");
        std::process::exit(1);
    }

    let start = Instant::now();
    let mut joined = false;
    let mut next_print = start;
    while start.elapsed() < Duration::from_secs(secs) {
        if !host && !joined && start.elapsed() >= Duration::from_secs(wait) {
            session.join(name, &address);
            joined = true;
        }
        session.tick(false);
        let verdict = light.evaluate(&session, &probe, &address, PeerState::default());
        if Instant::now() >= next_print {
            next_print += Duration::from_secs(1);
            println!(
                "[{name}] t={:2}s {:?} light={} ({}) players={}",
                start.elapsed().as_secs(),
                session.state(),
                if verdict.ready { "GREEN" } else { "RED" },
                verdict.text,
                session.players().len(),
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}
