//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). `cargo` and `git` are invoked through `std::process::Command`.

mod assets;
mod layers;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  assets   check that every icon/image/font/asset is attributed in ATTRIBUTION.md
  layers   enforce the crate dependency layering (docs/architecture.md)
  wasm     cargo check --target wasm32-unknown-unknown for the wasm-safe crates
  deny     cargo deny check (advisories, licences, bans, sources; see deny.toml)
  perf     the speed gates, in release (200 tasks reschedule under 10 ms, bare and with 30 resources and 300 assignments; 2,000 reported; Level All on the staffed plan under 2 s; Microsoft Project XML export of 2,000 tasks under 1 s)
  ci       fmt --check, clippy -D warnings, test, perf, assets, layers, wasm, deny (stops at first failure)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::run(&root()),
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("deny") => cmd_deny(),
        Some("perf") => cmd_perf(),
        Some("ci") => cmd_ci(),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent dir").to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (docs/architecture.md)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    let mut results = Vec::new();
    for pkg in &set {
        let mut c = cargo();
        c.args(["check", "--target", "wasm32-unknown-unknown", "-p", pkg]);
        let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown -p {pkg}")).is_ok();
        results.push((pkg.clone(), ok));
    }
    println!("\nwasm32-unknown-unknown check:");
    for (p, ok) in &results {
        println!("  {:<6} {p}", if *ok { "ok" } else { "FAIL" });
    }
    let failed = results.iter().filter(|r| !r.1).count();
    if failed == 0 { Ok(()) } else { Err(format!("{failed} crate(s) failed the wasm check")) }
}

/// `cargo deny check` against deny.toml. cargo-deny is a separate install: without it the step
/// says how to get it and passes on a developer's machine, and fails where `CI` is set.
fn cmd_deny() -> Result<(), String> {
    let installed = cargo().args(["deny", "--version"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !installed {
        let hint = "cargo-deny is not installed: `cargo install cargo-deny --locked`";
        if std::env::var_os("CI").is_some() {
            return Err(hint.into());
        }
        eprintln!("deny: skipped, {hint}");
        return Ok(());
    }
    let mut c = cargo();
    c.args(["deny", "check"]);
    run(c, "cargo deny check")
}

/// The rescheduling speed gate: an `#[ignore]`d test, timed in release only (a debug build's
/// timings mean nothing). It takes the median of several runs and prints the timings.
fn cmd_perf() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--release", "-p", "projectcraft-engine", "--test", "performance", "--", "--ignored", "--nocapture"]);
    run(c, "cargo test --release -p projectcraft-engine --test performance -- --ignored --nocapture")
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets -- -D warnings")
            }),
        ),
        (
            "test",
            Box::new(|| {
                let mut c = cargo();
                c.args(["test", "--workspace"]);
                run(c, "cargo test --workspace")
            }),
        ),
        ("perf", Box::new(cmd_perf)),
        ("assets", Box::new(|| assets::run(&root()))),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
        ("deny", Box::new(cmd_deny)),
    ];
    // `XTASK_CI_SKIP=clippy,wasm`: steps another runner covers (CI runs them on Linux only).
    let skip = std::env::var("XTASK_CI_SKIP").unwrap_or_default();
    let skipped: Vec<&str> = skip.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let steps: Vec<Step> = steps.into_iter().filter(|(name, _)| !skipped.contains(name)).collect();
    if !skipped.is_empty() {
        eprintln!("ci: skipping {} (XTASK_CI_SKIP)", skipped.join(", "));
    }
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}
