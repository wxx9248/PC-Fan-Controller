//! Fixtures that tests in more than one module share.

use std::path::Path;

use crate::config::Config;

/// Every installation target's rendered config, parsed. For tests that need the
/// values that ship rather than a hand-made copy of them.
pub fn rendered_target_configs() -> Vec<(String, Config)> {
    render_all_targets()
        .into_iter()
        .map(|(target, text)| {
            let config = toml::from_str(&text)
                .unwrap_or_else(|e| panic!("target {target} produced unparseable TOML: {e}"));
            (target, config)
        })
        .collect()
}

/// Render the config templates for one installation target and hand back the
/// resulting TOML.
///
/// Shelling out to the same renderer the Makefile and `debian/rules` use is
/// the point: a hand-written mirror of the template could drift, and a
/// `include_str!` of the template itself cannot work now that the values are
/// placeholders rather than TOML.
fn render_target(target: &str, into: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new(root.join("scripts/render-config"))
        .args(["--target", target, "--outdir"])
        .arg(into)
        .current_dir(root)
        .output()
        .expect("render-config must be executable");
    assert!(
        out.status.success(),
        "rendering target {target} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(into.join("fan-controllerd.toml"))
        .expect("the renderer must produce fan-controllerd.toml")
}

/// Render every installation target and hand back each one's TOML.
pub fn render_all_targets() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let listing = std::process::Command::new(root.join("scripts/render-config"))
        .arg("--list-targets")
        .current_dir(root)
        .output()
        .expect("render-config must be executable");
    let listing = String::from_utf8(listing.stdout).expect("target names are UTF-8");
    let targets: Vec<String> = listing.split_whitespace().map(String::from).collect();
    assert!(
        targets.iter().any(|t| t == "local") && targets.iter().any(|t| t == "debian"),
        "expected at least the local and debian targets, got {targets:?}"
    );
    targets
        .into_iter()
        .map(|target| {
            // Unique per call as well as per process: several tests render
            // concurrently, and a shared directory let one delete another's
            // output mid-read.
            static RENDERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = RENDERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("fanctl-cfg-{target}-{}-{n}", std::process::id()));
            let text = render_target(&target, &dir);
            let _ = std::fs::remove_dir_all(&dir);
            (target, text)
        })
        .collect()
}
