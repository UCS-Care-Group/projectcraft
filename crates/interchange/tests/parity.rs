//! Parity fixtures: plans made in another program (or written by hand to its published rules),
//! each with the Start and Finish dates that program works out. Each one is imported and scheduled
//! by ProjectCraft and its "Dates differ" lines are compared with the fixture's expectations.
//!
//! A fixture is a file in `tests/fixtures/parity/` (`.xml` Microsoft Project XML, `.csv` CSV)
//! with a `.expected` file of the same name beside it: one accepted "Dates differ" line a line,
//! exactly as the import report writes it, and `#` comments saying why each is accepted (a
//! listed gap). Every date difference not listed fails the test, and so does a listed one that no
//! longer happens (the gap closed: take the line out). Adding a fixture needs no code: drop the
//! file and its `.expected` in the folder and give it a row in `ATTRIBUTION.md`. Never Microsoft's
//! sample files or a real organisation's plan: invented tasks only.

use std::path::{Path, PathBuf};

use projectcraft_interchange::{Format, Outcome, import};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity")
}

fn format_of(path: &Path) -> Option<Format> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "xml" => Some(Format::Mspdi),
        "csv" => Some(Format::Csv),
        _ => None,
    }
}

/// The accepted lines of an `.expected` file: every line but blanks and `#` comments.
fn accepted(text: &str) -> Vec<String> {
    let mut v: Vec<String> =
        text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).map(str::to_owned).collect();
    v.sort();
    v
}

#[test]
fn every_parity_fixture_imports_with_only_its_accepted_date_differences() {
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(dir()).unwrap().map(|e| e.unwrap().path()).filter(|p| format_of(p).is_some()).collect();
    fixtures.sort();
    assert!(fixtures.len() >= 3, "the parity fixtures were found: {fixtures:?}");
    let mut failures = Vec::new();
    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = std::fs::read(path).unwrap();
        let i = match import(format_of(path).unwrap(), &bytes) {
            Ok(i) => i,
            Err(e) => {
                failures.push(format!("{name}: didn't import: {e}"));
                continue;
            }
        };
        assert!(!i.project.tasks.is_empty(), "{name}: no tasks");
        let mut ours: Vec<String> = i
            .report
            .lines
            .iter()
            .filter(|l| l.outcome == Outcome::DatesDiffer)
            .map(|l| format!("{}: {} ({}): {}", l.outcome.label(), l.feature, l.place, l.why))
            .collect();
        ours.sort();
        let expected_path = path.with_extension("expected");
        let Ok(expected_text) = std::fs::read_to_string(&expected_path) else {
            failures.push(format!(
                "{name}: no {} beside it; these are its date differences (each to be fixed, or listed there with why):\n{}",
                expected_path.file_name().unwrap().to_string_lossy(),
                ours.join("\n")
            ));
            continue;
        };
        let expected = accepted(&expected_text);
        let new: Vec<&String> = ours.iter().filter(|l| !expected.contains(l)).collect();
        let gone: Vec<&String> = expected.iter().filter(|l| !ours.contains(l)).collect();
        if !new.is_empty() {
            failures.push(format!(
                "{name}: date differences not accepted in its .expected:\n{}",
                new.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n")
            ));
        }
        if !gone.is_empty() {
            failures.push(format!(
                "{name}: accepted date differences that no longer happen (take them out of its .expected):\n{}",
                gone.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_parity_fixture_has_its_expectations_and_attribution() {
    let attribution = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ATTRIBUTION.md")).unwrap();
    for entry in std::fs::read_dir(dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        match path.extension().and_then(|e| e.to_str()) {
            Some("expected") => {
                assert!(
                    std::fs::read_dir(dir()).unwrap().any(|e| {
                        let p = e.unwrap().path();
                        format_of(&p).is_some() && p.with_extension("expected") == path
                    }),
                    "{name} has no fixture beside it"
                );
            }
            _ if format_of(&path).is_some() => {
                let listed = format!("`crates/interchange/tests/fixtures/parity/{name}`");
                assert!(attribution.contains(&listed), "{name} has no row in ATTRIBUTION.md");
            }
            _ => panic!("{name}: a parity fixture is a .xml or .csv file with a .expected beside it"),
        }
    }
}

#[test]
fn expectations_skip_comments_and_blank_lines() {
    let text = "# why\n\nDates differ: Start and finish (ID 2 A): x  \n  # indented comment\nDates differ: Start and finish (ID 1 B): y\n";
    assert_eq!(
        accepted(text),
        vec!["Dates differ: Start and finish (ID 1 B): y".to_owned(), "Dates differ: Start and finish (ID 2 A): x".to_owned()]
    );
}
