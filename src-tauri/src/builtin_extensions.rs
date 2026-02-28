use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

struct BuiltinExtension {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

const EXTENSIONS: &[BuiltinExtension] = &[
    BuiltinExtension {
        name: "bankofamerica",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/bankofamerica/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/bankofamerica/driver.mjs"),
            ),
        ],
    },
    BuiltinExtension {
        name: "chase",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/chase/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/chase/driver.mjs"),
            ),
        ],
    },
    BuiltinExtension {
        name: "citi",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/citi/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/citi/driver.mjs"),
            ),
        ],
    },
    BuiltinExtension {
        name: "paypal",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/paypal/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/paypal/driver.mjs"),
            ),
        ],
    },
    BuiltinExtension {
        name: "providentcu",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/providentcu/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/providentcu/driver.mjs"),
            ),
            (
                "extract.mjs",
                include_str!("../../builtin-extensions/providentcu/extract.mjs"),
            ),
        ],
    },
    BuiltinExtension {
        name: "target",
        files: &[
            (
                "manifest.json",
                include_str!("../../builtin-extensions/target/manifest.json"),
            ),
            (
                "driver.mjs",
                include_str!("../../builtin-extensions/target/driver.mjs"),
            ),
        ],
    },
];

static EXTRACTED: OnceLock<HashMap<&'static str, PathBuf>> = OnceLock::new();

fn ensure_extracted() -> &'static HashMap<&'static str, PathBuf> {
    EXTRACTED.get_or_init(|| {
        let base = std::env::temp_dir().join(format!("refreshmint-builtin-{}", std::process::id()));
        let mut map = HashMap::new();
        for ext in EXTENSIONS {
            let dir = base.join(ext.name);
            if std::fs::create_dir_all(&dir).is_ok() {
                for (fname, content) in ext.files {
                    let _ = std::fs::write(dir.join(fname), content);
                }
                map.insert(ext.name, dir);
            }
        }
        map
    })
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
        if candidate.join("driver.mjs").is_file() {
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
