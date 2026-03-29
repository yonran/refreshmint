use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

struct BuiltinExtension {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/builtin_extensions_generated.rs"));

static EXTRACTED: OnceLock<HashMap<&'static str, PathBuf>> = OnceLock::new();

fn extract_builtin_extension(
    base: &std::path::Path,
    ext: &BuiltinExtension,
) -> std::io::Result<PathBuf> {
    let dir = base.join(ext.name);
    std::fs::create_dir_all(&dir)?;
    for (fname, content) in ext.files {
        let path = dir.join(fname);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    Ok(dir)
}

fn ensure_extracted() -> &'static HashMap<&'static str, PathBuf> {
    EXTRACTED.get_or_init(|| {
        let base = std::env::temp_dir().join(format!("refreshmint-builtin-{}", std::process::id()));
        let mut map = HashMap::new();
        for ext in EXTENSIONS {
            if let Ok(dir) = extract_builtin_extension(&base, ext) {
                map.insert(ext.name, dir);
            } else {
                eprintln!("[builtin] failed to extract '{}'", ext.name);
            }
        }
        map
    })
}

fn has_runnable_driver(candidate: &std::path::Path) -> bool {
    let manifest = match crate::scrape::load_manifest(candidate) {
        Ok(manifest) => manifest,
        Err(_) => return false,
    };
    crate::scrape::resolve_driver_script_path(candidate, &manifest).is_file()
}

/// Return the directory for a built-in extension by name, or `None` if unknown.
///
/// In debug builds, prefers the live source tree via `CARGO_MANIFEST_DIR` so
/// edits to extension files are picked up without recompiling. Falls back to
/// embedded bytes extracted to a process-scoped temp directory if the source
/// path is absent (binary moved, different machine, or release build).
pub fn resolve_dir(name: &str) -> Option<PathBuf> {
    // Debug: try the live source tree first (edits reflected without recompile)
    #[cfg(debug_assertions)]
    {
        let source_root = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../builtin-extensions"
        ));
        let candidate = source_root.join(name);
        if has_runnable_driver(&candidate) {
            eprintln!(
                "[builtin] using source tree for '{name}': {}",
                candidate.display()
            );
            return Some(candidate);
        }
        eprintln!(
            "[builtin] source tree path not found for '{name}' ({}), falling back to embedded",
            candidate.display()
        );
    }

    // Embedded extraction fallback (always used in release builds)
    let dir = ensure_extracted().get(name).cloned();
    #[cfg(debug_assertions)]
    if let Some(ref p) = dir {
        eprintln!(
            "[builtin] using embedded extraction for '{name}': {}",
            p.display()
        );
    }
    dir
}

/// Return the names of all built-in extensions.
pub fn names() -> impl Iterator<Item = &'static str> {
    EXTENSIONS.iter().map(|e| e.name)
}

#[cfg(test)]
mod tests {
    use super::{extract_builtin_extension, BuiltinExtension};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|err| panic!("system clock error: {err}"))
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|err| {
            panic!("failed to create temp dir {}: {err}", dir.display());
        });
        dir
    }

    #[test]
    fn extract_builtin_extension_creates_parent_directories_for_nested_files() {
        let root = create_temp_dir("builtin-extract");
        let ext = BuiltinExtension {
            name: "demo",
            files: &[
                ("manifest.json", "{}"),
                ("dist/driver.mjs", "console.log('driver');"),
                ("dist/extract.mjs", "console.log('extract');"),
            ],
        };

        let dir = extract_builtin_extension(&root, &ext).unwrap_or_else(|err| {
            panic!("extract_builtin_extension failed: {err}");
        });

        assert_eq!(
            fs::read_to_string(dir.join("dist/driver.mjs")).unwrap_or_else(|err| {
                panic!("failed to read extracted driver: {err}");
            }),
            "console.log('driver');"
        );
        assert_eq!(
            fs::read_to_string(dir.join("dist/extract.mjs")).unwrap_or_else(|err| {
                panic!("failed to read extracted extract script: {err}");
            }),
            "console.log('extract');"
        );

        let _ = fs::remove_dir_all(root);
    }
}
