//! Golden-program test runner.
//!
//! For each `tests/golden/NN_<name>.pas`, the matching
//! `NN_<name>.expected.json` describes what to assert about the run:
//!   - `no_errors`: no Event::Error in the output
//!   - `datasets`: a map of `LIBREF.NAME` → { rows, columns }
//!
//! Add a new pair of files to grow the suite — no Rust changes needed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pas_engine::{Event, Session};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Expected {
    #[serde(default = "default_true")]
    no_errors: bool,
    #[serde(default)]
    datasets: BTreeMap<String, DatasetExpected>,
}

#[derive(Debug, Deserialize)]
struct DatasetExpected {
    rows: u64,
    columns: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn golden_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is the engine crate; the goldens live in the repo
    // root under tests/golden.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("golden")
}

#[test]
fn all_goldens_pass() {
    let dir = golden_dir();
    assert!(dir.exists(), "missing {}", dir.display());
    let mut programs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir golden")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pas"))
        .collect();
    programs.sort();
    assert!(
        !programs.is_empty(),
        "no golden programs found in {}",
        dir.display()
    );

    let mut failures = Vec::new();
    for pas in programs {
        if let Err(e) = run_one(&pas) {
            failures.push(format!(
                "{}: {}",
                pas.file_name().unwrap().to_string_lossy(),
                e
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} golden(s) failed:\n  - {}",
            failures.len(),
            failures.join("\n  - ")
        );
    }
}

fn run_one(pas_path: &Path) -> Result<(), String> {
    let expected_path = pas_path.with_extension("expected.json");
    let expected_text = std::fs::read_to_string(&expected_path)
        .map_err(|e| format!("read expected: {}: {}", expected_path.display(), e))?;
    let expected: Expected =
        serde_json::from_str(&expected_text).map_err(|e| format!("parse expected: {}", e))?;
    let program = std::fs::read_to_string(pas_path).map_err(|e| e.to_string())?;

    let session = Session::new_in_memory().map_err(|e| e.to_string())?;
    let events = session.submit(&program);

    if expected.no_errors {
        let errs: Vec<&Event> = events
            .iter()
            .filter(|e| matches!(e, Event::Error { .. }))
            .collect();
        if !errs.is_empty() {
            return Err(format!("unexpected error events: {:?}", errs));
        }
    }

    for (qualified, exp) in &expected.datasets {
        let (libref, name) = qualified
            .split_once('.')
            .ok_or_else(|| format!("dataset key {:?} must be 'libref.name'", qualified))?;
        let page = session
            .dataset_page(libref, name, 0, exp.rows.max(1), None)
            .map_err(|e| format!("dataset_page {}: {}", qualified, e))?;
        if page.total_rows != exp.rows {
            return Err(format!(
                "dataset {}: expected {} rows, got {}",
                qualified, exp.rows, page.total_rows
            ));
        }
        let actual_cols: Vec<String> = page
            .columns
            .iter()
            .map(|c| c.name.to_ascii_lowercase())
            .collect();
        let expected_cols: Vec<String> =
            exp.columns.iter().map(|c| c.to_ascii_lowercase()).collect();
        if actual_cols != expected_cols {
            return Err(format!(
                "dataset {}: columns mismatch — expected {:?}, got {:?}",
                qualified, expected_cols, actual_cols
            ));
        }
    }
    Ok(())
}
