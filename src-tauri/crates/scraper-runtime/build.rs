use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("scraper-runtime should be nested under src-tauri/crates"))
        .to_path_buf();
    let extensions_dir = repo_root.join("builtin-extensions");
    println!("cargo:rerun-if-changed={}", extensions_dir.display());
    println!(
        "cargo:rustc-env=REFRESHMINT_BUILTIN_EXTENSIONS_DIR={}",
        extensions_dir.display()
    );

    let out_dir =
        PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_else(|| panic!("OUT_DIR missing")));
    let builtin_out_dir = out_dir.join("builtin-extensions");
    let status = Command::new("node")
        .arg(repo_root.join("scripts/build-extensions.mjs"))
        .arg("--builtin-out-dir")
        .arg(&builtin_out_dir)
        .current_dir(&repo_root)
        .status()
        .unwrap_or_else(|error| panic!("failed to run extension builder: {error}"));
    if !status.success() {
        panic!("builtin extension build failed");
    }

    let mut source = String::from("const EXTENSIONS: &[BuiltinExtension] = &[\n");
    let mut extension_dirs = std::fs::read_dir(&builtin_out_dir)
        .unwrap_or_else(|error| panic!("failed to read built extensions: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("invalid entry: {error}"))
                .path()
        })
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    extension_dirs.sort();
    for extension_dir in extension_dirs {
        let name = extension_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| panic!("extension name should be utf-8"));
        writeln!(&mut source, "    BuiltinExtension {{")
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
        writeln!(&mut source, "        name: {:?},", name)
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
        writeln!(&mut source, "        files: &[")
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
        let mut files = Vec::new();
        collect_files(&extension_dir, &extension_dir, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        for (relative, absolute) in files {
            writeln!(
                &mut source,
                "            ({:?}, include_str!(r#\"{}\"#)),",
                relative,
                absolute.display()
            )
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
        }
        writeln!(&mut source, "        ],")
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
        writeln!(&mut source, "    }},")
            .unwrap_or_else(|error| panic!("failed to build source: {error}"));
    }
    source.push_str("];\n");
    std::fs::write(out_dir.join("builtin_extensions_generated.rs"), source)
        .unwrap_or_else(|error| panic!("failed to write builtins source: {error}"));
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) {
    let mut entries = std::fs::read_dir(current)
        .unwrap_or_else(|error| panic!("failed to read extension contents: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("invalid entry: {error}"))
                .path()
        })
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|error| panic!("built file should be under root: {error}"))
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
}
