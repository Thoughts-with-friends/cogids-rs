//! Results: `report.md` and `results.json` per run, `latest.md` beside the
//! runs, and the snapshot-hash baseline (`baseline.json`).
use crate::check::{self, Check};
use crate::config::{Case, FEATURES};
use crate::run::Metrics;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Snapshot hashes per case name.
pub type Baseline = BTreeMap<String, BTreeMap<String, String>>;

/// One case after it ran.
#[derive(Debug)]
pub struct CaseResult {
    pub case: Case,
    pub outcome: Result<Metrics, String>,
    pub checks: Vec<Check>,
}

impl CaseResult {
    pub const fn new(case: Case, outcome: Result<Metrics, String>) -> Self {
        Self {
            case,
            outcome,
            checks: Vec::new(),
        }
    }

    pub fn evaluate(&mut self, base: Option<&BTreeMap<String, String>>) {
        if let Ok(m) = &self.outcome {
            self.checks = check::evaluate(&self.case, m, base);
        }
    }

    /// Ran, and no (non-informational) check failed.
    pub fn passed(&self) -> bool {
        self.outcome.is_ok() && self.checks.iter().all(|c| c.ok || c.info)
    }
}

/// Folder name of a run: seconds since the Unix epoch.
pub fn run_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("run-{secs}")
}

fn status(r: &CaseResult) -> &'static str {
    match (&r.outcome, r.passed()) {
        (Err(_), _) => "ERROR",
        (Ok(_), true) => "PASS",
        (Ok(_), false) => "FAIL",
    }
}

fn markdown(results: &[CaseResult]) -> String {
    let mut s = String::from("# ROM 実行テストの結果\n\n");
    let pass = results.iter().filter(|r| r.passed()).count();
    let _ = writeln!(s, "{pass} / {} 件が成功\n", results.len());
    for (id, ch, name) in FEATURES {
        let rs: Vec<&CaseResult> = results.iter().filter(|r| r.case.feature == id).collect();
        if rs.is_empty() {
            continue;
        }
        let _ = writeln!(s, "## {ch}. {name}（`{id}`）\n");
        s.push_str("| ケース | 結果 | ms/フレーム | チェック |\n|---|---|---|---|\n");
        for r in rs {
            let (ms, detail) = match &r.outcome {
                Ok(m) => (
                    format!("{:.2}", m.ms_per_frame),
                    r.checks
                        .iter()
                        .map(|c| {
                            format!(
                                "{} {}: {}",
                                if c.ok {
                                    "✓"
                                } else if c.info {
                                    "△"
                                } else {
                                    "✗"
                                },
                                c.name,
                                c.detail
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("<br>"),
                ),
                Err(e) => ("-".to_owned(), e.replace('|', "/")),
            };
            let memo = r
                .case
                .memo
                .as_deref()
                .map_or_else(String::new, |m| format!("<br>{}", m.replace('|', "/")));
            let _ = writeln!(
                s,
                "| {}{memo} | {} | {ms} | {detail} |",
                r.case.name,
                status(r)
            );
        }
        s.push('\n');
    }
    s
}

fn json(results: &[CaseResult]) -> serde_json::Value {
    let cases: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let (metrics, error) = match &r.outcome {
                Ok(m) => (
                    serde_json::json!({
                        "frames": m.frames, "ms_per_frame": m.ms_per_frame, "max_polygons": m.max_polygons,
                        "audio_frames": m.audio_frames, "distinct_screens": m.distinct_screens,
                        "hashes": m.hashes, "pngs": m.pngs,
                    }),
                    None,
                ),
                Err(e) => (serde_json::Value::Null, Some(e.clone())),
            };
            serde_json::json!({
                "feature": r.case.feature, "name": r.case.name, "status": status(r),
                "checks": r.checks, "metrics": metrics, "error": error,
            })
        })
        .collect();
    serde_json::json!({ "cases": cases })
}

/// Writes the run's report files.
///
/// # Errors
/// If a file cannot be written.
pub fn write(run_dir: &Path, results_root: &Path, results: &[CaseResult]) -> Result<(), String> {
    let md = markdown(results);
    let text = serde_json::to_string_pretty(&json(results)).map_err(|e| e.to_string())?;
    for (path, body) in [
        (run_dir.join("report.md"), md.as_str()),
        (run_dir.join("results.json"), text.as_str()),
        (results_root.join("latest.md"), md.as_str()),
    ] {
        std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// The stored baseline (empty if there is none).
pub fn load_baseline(path: &Path) -> Baseline {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Replaces the baseline of every case that ran.
///
/// # Errors
/// If the file cannot be written.
pub fn save_baseline(
    path: &Path,
    mut base: Baseline,
    results: &[CaseResult],
) -> Result<(), String> {
    for r in results {
        if let Ok(m) = &r.outcome {
            base.insert(r.case.name.clone(), m.hashes.clone());
        }
    }
    let text = serde_json::to_string_pretty(&base).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Prints one line per case; `true` if all passed.
pub fn print_summary(results: &[CaseResult]) -> bool {
    for r in results {
        let failed: Vec<&str> = r
            .checks
            .iter()
            .filter(|c| !c.ok && !c.info)
            .map(|c| c.name.as_str())
            .collect();
        match &r.outcome {
            Err(e) => println!("ERROR [{}] {}: {e}", r.case.feature, r.case.name),
            Ok(_) if failed.is_empty() => println!("PASS  [{}] {}", r.case.feature, r.case.name),
            Ok(_) => println!(
                "FAIL  [{}] {}: {}",
                r.case.feature,
                r.case.name,
                failed.join(", ")
            ),
        }
    }
    results.iter().all(CaseResult::passed)
}
