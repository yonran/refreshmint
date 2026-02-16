use std::path::Path;

fn main() {
    ensure_sidecar_placeholder();
    tauri_build::build();
}

/// Create an empty placeholder sidecar binary so that `tauri build` does not
/// fail during development when the real hledger binary has not been downloaded.
fn ensure_sidecar_placeholder() {
    let target_triple = std::env::var("TARGET").unwrap_or_default();
    if target_triple.is_empty() {
        return;
    }

    let ext = if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    };

    let name = format!("binaries/hledger-{target_triple}{ext}");
    let path = Path::new(&name);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, b"");
    }
}
