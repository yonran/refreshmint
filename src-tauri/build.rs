use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    register_build_inputs();
    ensure_sidecar_placeholder();
    let builtin_out_dir = build_builtin_extensions();
    generate_builtin_extensions_source(&builtin_out_dir);
    tauri_build::build();
}

fn register_build_inputs() {
    println!("cargo:rerun-if-changed=../builtin-extensions");
    println!("cargo:rerun-if-changed=../scripts/build-extensions.mjs");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=../package-lock.json");
}

fn build_builtin_extensions() -> PathBuf {
    let out_dir = match std::env::var_os("OUT_DIR") {
        Some(value) => PathBuf::from(value),
        None => panic!("OUT_DIR missing"),
    };
    let builtin_out_dir = out_dir.join("builtin-extensions");
    let repo_root = match PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent() {
        Some(value) => value.to_path_buf(),
        None => panic!("src-tauri should have repo parent"),
    };
    let script_path = repo_root.join("scripts").join("build-extensions.mjs");
    let status = Command::new("node")
        .arg(&script_path)
        .arg("--builtin-out-dir")
        .arg(&builtin_out_dir)
        .current_dir(&repo_root)
        .status()
        .unwrap_or_else(|error| panic!("failed to run extension builder: {error}"));
    if !status.success() {
        panic!("builtin extension build failed");
    }
    builtin_out_dir
}

fn generate_builtin_extensions_source(builtin_out_dir: &Path) {
    let out_dir = match std::env::var_os("OUT_DIR") {
        Some(value) => PathBuf::from(value),
        None => panic!("OUT_DIR missing"),
    };
    let generated_path = out_dir.join("builtin_extensions_generated.rs");
    let mut source = String::from("const EXTENSIONS: &[BuiltinExtension] = &[\n");
    let mut extension_dirs = std::fs::read_dir(builtin_out_dir)
        .unwrap_or_else(|error| panic!("failed to read built builtin extensions: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("invalid builtin extension dir entry: {error}"))
                .path()
        })
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    extension_dirs.sort();

    for extension_dir in extension_dirs {
        let name = extension_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| panic!("builtin extension dir name should be utf-8"));
        let files = collect_files(&extension_dir);
        writeln!(&mut source, "    BuiltinExtension {{")
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
        writeln!(&mut source, "        name: {:?},", name)
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
        writeln!(&mut source, "        files: &[")
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
        for (relative_path, absolute_path) in files {
            writeln!(
                &mut source,
                "            ({:?}, include_str!(r#\"{}\"#)),",
                relative_path,
                absolute_path.display()
            )
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
        }
        writeln!(&mut source, "        ],")
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
        writeln!(&mut source, "    }},")
            .unwrap_or_else(|error| panic!("failed to write builtin extension source: {error}"));
    }

    source.push_str("];\n");
    std::fs::write(generated_path, source)
        .unwrap_or_else(|error| panic!("failed to write builtin extensions source: {error}"));
}

fn collect_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    collect_files_recursive(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect_files_recursive(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) {
    let mut entries = std::fs::read_dir(current)
        .unwrap_or_else(|error| panic!("failed to read builtin extension contents: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("invalid builtin extension file entry: {error}"))
                .path()
        })
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files_recursive(root, &path, files);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| panic!("built builtin file should be under root: {error}"))
            .to_string_lossy()
            .replace('\\', "/");
        files.push((relative, path));
    }
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
