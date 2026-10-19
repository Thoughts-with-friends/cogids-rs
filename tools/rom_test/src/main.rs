//! `rom_test`: runs per-feature ROM test cases.
//!
//! ```text
//! rom_test [--config PATH] [--feature ID,..] [--case TEXT] [--jobs N]
//!          [--update-baseline] [--no-png] [--list]
//! rom_test --rom PATH [--feature ID] [--frames N] [--press ..] [--touch ..]
//!          [--cheat ..] [--hd N] [--xbrz] [--snapshot N] [--expect KEY[=V]] ..
//! ```
//!
//! The private config (`docs/cogids-rs/local/roms.toml`), its results,
//! saves and baseline stay in git-ignored folders: ROM names and paths are
//! never committed. See `docs/cogids-rs/20_rom_tests.md`.
mod check;
mod cli;
mod config;
mod input;
mod report;
mod run;

use cli::Args;
use config::{Case, Config, FEATURES};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The workspace root (two levels above this crate).
fn workspace() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    // Drop Windows' `\\?\` prefix so paths print and join normally.
    PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"))
}

/// `docs/cogids-rs/local`: private config and results (git-ignored).
fn local_dir() -> PathBuf {
    workspace().join("docs/cogids-rs/local")
}

fn print_list(config: Option<&Config>) {
    println!("features (id  chapter  name):");
    for (id, ch, name) in FEATURES {
        let n = config.map_or(0, |c| c.cases.iter().filter(|k| k.feature == id).count());
        println!("  {id:<11} {ch:>2}  {name}  ({n} cases)");
    }
    if let Some(c) = config {
        for case in &c.cases {
            println!(
                "  - [{}] {} ({} frames)",
                case.feature, case.name, case.frames
            );
        }
    }
}

fn selected<'a>(config: &'a Config, args: &Args) -> Vec<&'a Case> {
    config
        .cases
        .iter()
        .filter(|c| args.features.is_empty() || args.features.contains(&c.feature))
        .filter(|c| {
            args.case
                .as_ref()
                .is_none_or(|t| c.name.contains(t.as_str()))
        })
        .collect()
}

/// Runs `cases` on `jobs` threads; results in the order of `cases`.
fn run_all(
    config: &Config,
    cases: &[&Case],
    run_dir: &Path,
    jobs: usize,
    png: bool,
) -> Vec<report::CaseResult> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, cases.len().max(1)) {
            let spawned = std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn_scoped(s, || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&case) = cases.get(i) else { break };
                        let dir = run_dir.join(run::slug(&case.name));
                        let outcome = std::fs::create_dir_all(&dir)
                            .map_err(|e| e.to_string())
                            .and_then(|()| run::run(config, case, &dir, png));
                        println!("[{}] {} done", case.feature, case.name);
                        if let Ok(mut r) = results.lock() {
                            r.push((i, case.clone(), outcome));
                        }
                    }
                });
            if let Err(e) = spawned {
                eprintln!("cannot start a worker: {e}");
            }
        }
    });
    let mut done = results.into_inner().unwrap_or_default();
    done.sort_by_key(|r| r.0);
    done.into_iter()
        .map(|(_, case, outcome)| report::CaseResult::new(case, outcome))
        .collect()
}

/// The config to run and the folder its results go to, or `None` when
/// there is nothing to run.
fn load(args: &Args) -> Result<Option<(Config, PathBuf)>, String> {
    if let Some(case) = args.rom_case()? {
        let config = Config::single(case)?;
        let results = args
            .results
            .clone()
            .unwrap_or_else(|| local_dir().join("results"));
        return Ok(Some((config, results)));
    }
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| local_dir().join("roms.toml"));
    if !path.is_file() {
        if args.list {
            print_list(None);
        }
        println!("No ROM test config at {}.", path.display());
        println!("Either run one ROM:   rom_test --rom PATH --feature boot --expect no_terminate");
        println!(
            "or copy docs/cogids-rs/roms.example.toml to docs/cogids-rs/local/roms.toml (git-ignored)."
        );
        return Ok(None);
    }
    let config = Config::load(&path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
    let results = match &args.results {
        Some(r) => r.clone(),
        None => dir.join(
            config
                .settings
                .results
                .clone()
                .unwrap_or_else(|| "results".into()),
        ),
    };
    Ok(Some((config, results)))
}

fn main_inner() -> Result<bool, String> {
    let args = Args::parse()?;
    if args.help {
        print!("{}", include_str!("help.txt"));
        return Ok(true);
    }
    let Some((config, results_root)) = load(&args)? else {
        return Ok(true);
    };
    if args.list {
        print_list(Some(&config));
        return Ok(true);
    }
    let cases = if args.rom.is_some() {
        config.cases.iter().collect()
    } else {
        selected(&config, &args)
    };
    if cases.is_empty() {
        println!("No case matches.");
        return Ok(true);
    }
    let run_dir = results_root.join(report::run_id());
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
    let jobs = args.jobs.or(config.settings.jobs).unwrap_or(2);
    println!(
        "Running {} cases ({jobs} at a time) -> {}",
        cases.len(),
        run_dir.display()
    );
    let mut results = run_all(&config, &cases, &run_dir, jobs, !args.no_png);
    let baseline_path = results_root.join("baseline.json");
    let baseline = report::load_baseline(&baseline_path);
    for r in &mut results {
        r.evaluate(baseline.get(&r.case.name));
    }
    report::write(&run_dir, &results_root, &results)?;
    if args.update_baseline {
        report::save_baseline(&baseline_path, baseline, &results)?;
        println!("Baseline updated: {}", baseline_path.display());
    }
    println!("Report: {}", results_root.join("latest.md").display());
    Ok(report::print_summary(&results))
}

fn main() -> std::process::ExitCode {
    match main_inner() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::from(2)
        }
    }
}
