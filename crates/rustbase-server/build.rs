//! Pre-build hook: ensure the SvelteKit dashboard is built before
//! `include_dir!` snapshots it.
//!
//! Strategy:
//!
//! - `ui/build/` already present and newer than every input under
//!   `ui/src` → skip work entirely. `include_dir!` re-emits whatever
//!   was last written there.
//! - Otherwise, run `bun install` (if `ui/node_modules` is missing)
//!   then `bun --cwd ui run build`.
//! - If `bun` isn't on PATH, write a minimal placeholder `index.html`
//!   into `ui/build/` so `include_dir!` has something to point at and
//!   `cargo build` still succeeds. The placeholder explains how to
//!   restore the real dashboard. This keeps the Rust crate usable
//!   in CI lanes / environments where the JS toolchain is absent.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(workspace) = crate_dir.parent().and_then(Path::parent) else {
        // crate_dir is always crates/rustbase-server/, so its grandparent
        // (the workspace root) exists by construction. The fallback is
        // here only because Path::parent is structurally Option.
        eprintln!("cargo:warning=could not resolve workspace root from {crate_dir:?}");
        return;
    };
    let ui = workspace.join("ui");
    let build_dir = ui.join("build");
    let src_dir = ui.join("src");

    // Cargo: invalidate when any UI source or config changes.
    println!(
        "cargo:rerun-if-changed={}",
        ui.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ui.join("svelte.config.js").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ui.join("vite.config.ts").display()
    );
    println!("cargo:rerun-if-changed={}", src_dir.display());

    // Skip if no ui/ at all — this build path is for the repo layout;
    // a stripped-down vendored copy can still build without it.
    if !ui.is_dir() {
        return;
    }

    // If a build/ already exists with an index.html, trust it.
    // `cargo:rerun-if-changed` above will trigger us again when the
    // SvelteKit sources change.
    if build_dir.join("index.html").is_file() && fresher_than_inputs(&build_dir, &src_dir) {
        return;
    }

    let Some(bun) = which("bun") else {
        eprintln!(
            "cargo:warning=`bun` not on PATH; writing a placeholder dashboard. \
             Install bun (https://bun.sh) and re-run cargo build to embed the \
             real SvelteKit artifact."
        );
        write_placeholder(&build_dir);
        return;
    };

    if !ui.join("node_modules").is_dir() {
        run(&bun, &["install"], &ui, "bun install");
    }
    run(&bun, &["run", "build"], &ui, "bun run build");

    if !build_dir.join("index.html").is_file() {
        eprintln!("cargo:warning=ui build finished but build/index.html is missing");
        write_placeholder(&build_dir);
    }
}

fn fresher_than_inputs(build: &Path, src: &Path) -> bool {
    let Ok(build_mtime) = build
        .join("index.html")
        .metadata()
        .and_then(|m| m.modified())
    else {
        return false;
    };
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    for entry in walkdir(src) {
        if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
            if m > newest {
                newest = m;
            }
        }
    }
    build_mtime >= newest
}

/// Minimal stand-alone iterator instead of a walkdir dep.
fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(cmd);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn run(bin: &Path, args: &[&str], cwd: &Path, label: &str) {
    // The embedded dashboard mounts at `/_/` (see dashboard.rs). SvelteKit's
    // adapter-static needs `paths.base` set at build time so the generated
    // HTML references `/_/_app/...` (not `/_app/...`) AND so the runtime
    // `goto()` / `<a href="...">` calls auto-prefix the base. svelte.config.js
    // reads this from `VITE_BASE`.
    let status = Command::new(bin)
        .args(args)
        .env("VITE_BASE", "/_")
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {label}: {e}"));
    if !status.success() {
        panic!("{label} failed with status {status}");
    }
}

fn write_placeholder(build_dir: &Path) {
    let _ = std::fs::create_dir_all(build_dir);
    let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>RustBaas</title>\
<style>body{font:15px/1.5 -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0;background:#fafafa;color:#111}main{max-width:480px;padding:2rem;text-align:center}h1{font-weight:600;letter-spacing:-0.01em;margin:0 0 .5rem}p{color:#666;margin:0}code{background:#eef;padding:.15em .35em;border-radius:3px}</style></head>\
<body><main><h1>RustBaas</h1><p>Dashboard not built. Install <a href=\"https://bun.sh\">bun</a> and rerun <code>cargo build</code>, or run <code>bun --cwd ui run build</code> manually.</p></main></body></html>";
    let _ = std::fs::write(build_dir.join("index.html"), html);
}
