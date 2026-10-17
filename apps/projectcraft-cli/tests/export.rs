//! `projectcraft-cli export SOURCE --format FORMAT OUT` writes a copy for another program.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_projectcraft-cli"))
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("projectcraft-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn export_csv_writes_the_task_sheet_and_it_imports_again() {
    let dir = tmp_dir("export-csv");
    let out = dir.join("Office move.csv");
    let run = cli().args(["export", "--sample", "--format", "csv"]).arg(&out).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let bytes = fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"\xEF\xBB\xBFID,Task Name,Duration,"), "{}", String::from_utf8_lossy(&bytes));
    assert!(bytes.ends_with(b"\r\n"));
    let back = cli().args(["schedule", "--import"]).arg(&out).output().unwrap();
    assert!(back.status.success(), "{}", String::from_utf8_lossy(&back.stderr));
    assert!(String::from_utf8_lossy(&back.stdout).contains("Agree the brief"));
}

#[test]
fn export_refuses_a_wrong_name_or_format() {
    let dir = tmp_dir("export-bad");
    for (format, name) in [("csv", "plan.xml"), ("csv", "plan.projectcraft"), ("xer", "plan.csv")] {
        let run = cli().args(["export", "--sample", "--format", format]).arg(dir.join(name)).output().unwrap();
        assert!(!run.status.success(), "{format} {name}");
        assert!(!dir.join(name).exists(), "{name} was written");
    }
}
