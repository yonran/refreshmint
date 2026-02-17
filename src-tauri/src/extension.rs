use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn load_extension_from_source(
    ledger_dir: &Path,
    source: &Path,
    replace: bool,
) -> io::Result<String> {
    if source.is_dir() {
        let source_root = resolve_extension_root(source)?;
        return load_extension_from_directory(ledger_dir, &source_root, replace);
    }

    if source.is_file() {
        let is_zip = source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if !is_zip {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("source file must be a .zip archive: {}", source.display()),
            ));
        }

        let extracted = ExtractedZip::from_path(source)?;
        let source_root = resolve_extension_root(extracted.path())?;
        return load_extension_from_directory(ledger_dir, &source_root, replace);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("source path not found: {}", source.display()),
    ))
}

pub fn validate_extension_name(name: &str) -> io::Result<()> {
    if name.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "extension name cannot be empty",
        ));
    }

    if name == "." || name == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "extension name cannot be '.' or '..'",
        ));
    }

    if name.ends_with(' ') || name.ends_with('.') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "extension name cannot end with space or dot",
        ));
    }

    // Keep path segments portable across NTFS, Samba, and HFS+ by rejecting
    // characters disallowed by any of them.
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("extension name contains invalid character: {ch}"),
            ));
        }
    }

    if is_windows_reserved_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("extension name is reserved on NTFS: {name}"),
        ));
    }

    if name.len() > 255 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "extension name is too long",
        ));
    }

    Ok(())
}

fn load_extension_from_directory(
    ledger_dir: &Path,
    source_root: &Path,
    replace: bool,
) -> io::Result<String> {
    let name = read_extension_name(source_root)?;
    validate_extension_name(&name)?;

    let extensions_dir = ledger_dir.join("extensions");
    fs::create_dir_all(&extensions_dir)?;
    let target_dir = extensions_dir.join(&name);

    if target_dir.exists() {
        if !replace {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "extension '{}' already exists at {} (use --replace to overwrite)",
                    name,
                    target_dir.display()
                ),
            ));
        }
        remove_path(&target_dir)?;
    }

    copy_directory(source_root, &target_dir)?;
    Ok(name)
}

fn resolve_extension_root(base: &Path) -> io::Result<PathBuf> {
    let manifest = base.join("manifest.json");
    if manifest.is_file() {
        return Ok(base.to_path_buf());
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if path.join("manifest.json").is_file() {
            candidates.push(path);
        }
    }

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("manifest.json not found under {}", base.display()),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "multiple extension directories found under {} (ambiguous manifest.json)",
                base.display()
            ),
        )),
    }
}

fn read_extension_name(extension_root: &Path) -> io::Result<String> {
    #[derive(Deserialize)]
    struct Manifest {
        name: String,
    }

    let manifest_path = extension_root.join("manifest.json");
    let contents = fs::read_to_string(&manifest_path)?;
    let manifest: Manifest = serde_json::from_str(&contents).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid {}: {error}", manifest_path.display()),
        )
    })?;

    if manifest.name.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("manifest name is missing in {}", manifest_path.display()),
        ));
    }

    Ok(manifest.name)
}

fn is_windows_reserved_name(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();

    matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    let source_meta = fs::symlink_metadata(source)?;
    if !source_meta.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source is not a directory: {}", source.display()),
        ));
    }

    fs::create_dir(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_directory(&entry_path, &target)?;
            continue;
        }

        if file_type.is_file() {
            fs::copy(&entry_path, &target)?;
            continue;
        }

        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported file type in extension contents: {}",
                entry_path.display()
            ),
        ));
    }

    Ok(())
}

struct ExtractedZip {
    path: PathBuf,
}

impl ExtractedZip {
    fn from_path(zip_path: &Path) -> io::Result<Self> {
        let extract_dir = create_unique_temp_dir("refreshmint-extension")?;
        let extract_result = (|| -> io::Result<()> {
            let file = fs::File::open(zip_path)?;
            let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;

            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).map_err(io::Error::other)?;
                let Some(relative_path) = entry.enclosed_name() else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("zip contains invalid path entry: {}", entry.name()),
                    ));
                };
                if relative_path.as_os_str().is_empty() {
                    continue;
                }

                let output_path = extract_dir.join(relative_path);
                if entry.name().ends_with('/') {
                    fs::create_dir_all(&output_path)?;
                    continue;
                }

                if let Some(mode) = entry.unix_mode() {
                    // Reject symlinks in archives to avoid unexpected indirection.
                    if (mode & 0o170000) == 0o120000 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("zip contains symlink entry: {}", entry.name()),
                        ));
                    }
                }

                if let Some(parent) = output_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&output_path)?;
                io::copy(&mut entry, &mut output)?;
            }

            Ok(())
        })();

        if let Err(error) = extract_result {
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(error);
        }

        Ok(Self { path: extract_dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExtractedZip {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn create_unique_temp_dir(prefix: &str) -> io::Result<PathBuf> {
    for attempt in 0..100u32 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{now}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to create a unique temporary directory",
    ))
}

#[cfg(test)]
mod tests {
    use super::{load_extension_from_source, validate_extension_name};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&path).unwrap_or_else(|err| {
            panic!("failed to create temp dir {}: {err}", path.display());
        });
        path
    }

    fn write_manifest(dir: &Path, name: &str) {
        let manifest = format!("{{\"name\":\"{name}\"}}\n");
        fs::write(dir.join("manifest.json"), manifest).unwrap_or_else(|err| {
            panic!("failed to write manifest: {err}");
        });
    }

    #[test]
    fn loads_extension_from_directory() {
        let root = create_temp_dir("refreshmint-ext-dir");
        let ledger_dir = root.join("ledger.refreshmint");
        let source_dir = root.join("source-ext");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| {
            panic!("failed to create ledger dir: {err}");
        });
        fs::create_dir_all(&source_dir).unwrap_or_else(|err| {
            panic!("failed to create source dir: {err}");
        });
        write_manifest(&source_dir, "bank-sync");
        fs::write(source_dir.join("driver.mjs"), "// driver\n").unwrap_or_else(|err| {
            panic!("failed to write driver: {err}");
        });

        let loaded =
            load_extension_from_source(&ledger_dir, &source_dir, false).unwrap_or_else(|err| {
                panic!("load extension failed: {err}");
            });

        assert_eq!(loaded, "bank-sync");
        assert!(ledger_dir
            .join("extensions")
            .join("bank-sync")
            .join("driver.mjs")
            .is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn loads_extension_from_zip() {
        let root = create_temp_dir("refreshmint-ext-zip");
        let ledger_dir = root.join("ledger.refreshmint");
        let zip_path = root.join("extension.zip");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| {
            panic!("failed to create ledger dir: {err}");
        });

        let zip_file = fs::File::create(&zip_path).unwrap_or_else(|err| {
            panic!("failed to create zip file: {err}");
        });
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = SimpleFileOptions::default();
        zip.add_directory("bundle/", options)
            .unwrap_or_else(|err| panic!("failed to add directory: {err}"));
        zip.start_file("bundle/manifest.json", options)
            .unwrap_or_else(|err| panic!("failed to start manifest file: {err}"));
        zip.write_all(br#"{"name":"bank-sync"}"#)
            .unwrap_or_else(|err| panic!("failed to write manifest: {err}"));
        zip.start_file("bundle/driver.mjs", options)
            .unwrap_or_else(|err| panic!("failed to start driver file: {err}"));
        zip.write_all(b"// driver\n")
            .unwrap_or_else(|err| panic!("failed to write driver: {err}"));
        zip.finish()
            .unwrap_or_else(|err| panic!("failed to finalize zip file: {err}"));

        let loaded =
            load_extension_from_source(&ledger_dir, &zip_path, false).unwrap_or_else(|err| {
                panic!("load extension from zip failed: {err}");
            });
        assert_eq!(loaded, "bank-sync");
        assert!(ledger_dir
            .join("extensions")
            .join("bank-sync")
            .join("manifest.json")
            .is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn load_requires_replace_when_destination_exists() {
        let root = create_temp_dir("refreshmint-ext-replace");
        let ledger_dir = root.join("ledger.refreshmint");
        let source_dir = root.join("source-ext");
        let destination = ledger_dir.join("extensions").join("bank-sync");
        fs::create_dir_all(&source_dir).unwrap_or_else(|err| {
            panic!("failed to create source dir: {err}");
        });
        fs::create_dir_all(&destination).unwrap_or_else(|err| {
            panic!("failed to create destination dir: {err}");
        });
        write_manifest(&source_dir, "bank-sync");
        fs::write(source_dir.join("driver.mjs"), "// new\n").unwrap_or_else(|err| {
            panic!("failed to write source driver: {err}");
        });
        fs::write(destination.join("driver.mjs"), "// old\n").unwrap_or_else(|err| {
            panic!("failed to write destination driver: {err}");
        });

        let error = load_extension_from_source(&ledger_dir, &source_dir, false).err();
        assert!(error.is_some());

        load_extension_from_source(&ledger_dir, &source_dir, true).unwrap_or_else(|err| {
            panic!("replace load failed: {err}");
        });
        let driver = fs::read_to_string(destination.join("driver.mjs")).unwrap_or_else(|err| {
            panic!("failed to read replaced driver: {err}");
        });
        assert_eq!(driver, "// new\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validates_extension_names() {
        validate_extension_name("Bank.Sync-1").unwrap_or_else(|err| {
            panic!("valid name rejected: {err}");
        });
        assert!(validate_extension_name("CON").is_err());
        assert!(validate_extension_name("bad/name").is_err());
        assert!(validate_extension_name("bad*name").is_err());
        assert!(validate_extension_name("bad.").is_err());
        assert!(validate_extension_name("  ").is_err());
    }
}
