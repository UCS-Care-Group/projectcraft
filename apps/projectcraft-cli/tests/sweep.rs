//! The command line sweeps the temporary files unfinished saves left beside a project.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_projectcraft-cli"))
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("projectcraft-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Writes `dir/name`, last modified two hours ago: a save that was killed long since.
fn stale(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    let f = File::create(&p).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(2 * 60 * 60)).unwrap();
    p
}

#[test]
fn save_sweeps_stale_temps_beside_the_file() {
    let dir = tmp_dir("save");
    let plan = dir.join("plan.projectcraft");
    let temp = stale(&dir, ".plan.projectcraft.4242-0.craft_tmp");
    let other = stale(&dir, ".other.projectcraft.4242-0.craft_tmp");
    let out = cli().args(["run", "--sample", "--save"]).arg(&plan).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(plan.exists());
    assert!(!temp.exists(), "stale temp swept after save");
    assert!(other.exists(), "another file's temp kept");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn open_sweeps_stale_temps_beside_the_file() {
    let dir = tmp_dir("open");
    let plan = dir.join("plan.projectcraft");
    let out = cli().args(["run", "--sample", "--save"]).arg(&plan).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let temp = stale(&dir, ".plan.projectcraft.4242-0.craft_tmp");
    let bak_temp = stale(&dir, ".plan.projectcraft.bak.4242-1.craft_tmp");
    let recent = dir.join(".plan.projectcraft.4243-0.craft_tmp");
    File::create(&recent).unwrap();
    let out = cli().arg("schedule").arg(&plan).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stderr.is_empty(), "no warning when the sweep works");
    assert!(!temp.exists() && !bak_temp.exists(), "stale temps swept after open");
    assert!(recent.exists(), "a recent temp may be a save in progress");
    let _ = fs::remove_dir_all(dir);
}

/// A temp that can't be removed is one line on stderr, never a failure.
#[cfg(unix)]
#[test]
fn a_failed_sweep_is_a_warning_not_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp_dir("readonly");
    let plan = dir.join("plan.projectcraft");
    let out = cli().args(["run", "--sample", "--save"]).arg(&plan).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let temp = stale(&dir, ".plan.projectcraft.4242-0.craft_tmp");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let out = cli().arg("schedule").arg(&plan).output().unwrap();
    let removed = !temp.exists(); // a superuser can remove it anyway
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!out.stdout.is_empty(), "the schedule is still printed");
    if !removed {
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.starts_with("warning: couldn't remove a temporary file"), "{err}");
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_missing_file_is_still_an_error_and_sweeps_nothing() {
    let dir = tmp_dir("missing");
    let temp = stale(&dir, ".plan.projectcraft.4242-0.craft_tmp");
    let out = cli().arg("schedule").arg(dir.join("plan.projectcraft")).output().unwrap();
    assert!(!out.status.success());
    assert!(temp.exists());
    let _ = fs::remove_dir_all(dir);
}
