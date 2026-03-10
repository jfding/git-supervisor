// Set APP_VERSION from repo root VERSION file when present; otherwise fallback.
fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let version_file_root = manifest_dir.join("..").join("VERSION");
    let version_file_local = manifest_dir.join("VERSION");
    let version_file = if version_file_local.exists() {
        version_file_local
    } else {
        version_file_root
    };
    let version = std::fs::read_to_string(&version_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0.0.0-dev".to_string());
    println!("cargo:rustc-env=APP_VERSION={}", version);
}
