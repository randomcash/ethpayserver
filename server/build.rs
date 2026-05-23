use std::process::Command;

fn main() {
    // Prefer an externally-set env var (CI can inject a specific SHA)
    // then fall back to `git rev-parse --short HEAD`.
    let sha = std::env::var("ETHPAYSERVER_BUILD_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        });

    println!("cargo:rustc-env=ETHPAYSERVER_BUILD_SHA={sha}");

    // Rebuild when the checked-out commit changes.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
    println!("cargo:rerun-if-env-changed=ETHPAYSERVER_BUILD_SHA");
}
