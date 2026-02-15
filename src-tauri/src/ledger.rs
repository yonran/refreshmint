use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_LEDGER_DIR: &str = "accounting.refreshmint";
pub(crate) const GIT_USER_NAME: &str = "Refreshmint";
pub(crate) const GIT_USER_EMAIL: &str = "refreshmint@noreply.example.com";
pub(crate) const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

#[derive(Serialize, Deserialize)]
pub(crate) struct RefreshmintConfig {
    pub(crate) version: String,
}

pub fn default_ledger_dir_from_documents(documents_dir: PathBuf) -> PathBuf {
    documents_dir.join(DEFAULT_LEDGER_DIR)
}

pub fn ensure_refreshmint_extension(path: PathBuf) -> io::Result<PathBuf> {
    let mut updated = path;
    if updated.set_extension("refreshmint") {
        Ok(updated)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid ledger path",
        ))
    }
}

pub(crate) fn require_refreshmint_extension(path: &Path) -> io::Result<()> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("refreshmint") => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ledger directory must end with .refreshmint",
        )),
    }
}

pub fn new_ledger_at_dir(target_dir: &Path) -> io::Result<()> {
    create_ledger_dir(target_dir)?;
    enable_bundle_attr_if_supported(target_dir)?;
    write_refreshmint_json(target_dir)?;
    create_general_journal(target_dir)?;
    init_git_repo(target_dir)?;
    Ok(())
}

pub(crate) fn commit_general_journal(dir: &Path, message: &str) -> io::Result<()> {
    if !dir.join(".git").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "ledger git repository not found",
        ));
    }
    run_git(
        dir,
        &[OsString::from("add"), OsString::from("general.journal")],
    )?;
    run_git(
        dir,
        &[
            OsString::from("-c"),
            OsString::from(format!("user.name={GIT_USER_NAME}")),
            OsString::from("-c"),
            OsString::from(format!("user.email={GIT_USER_EMAIL}")),
            OsString::from("commit"),
            OsString::from("-m"),
            OsString::from(message),
        ],
    )?;
    Ok(())
}

fn create_ledger_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir(dir)
}

fn write_refreshmint_json(dir: &Path) -> io::Result<()> {
    let path = dir.join("refreshmint.json");
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let config = RefreshmintConfig {
        version: crate::version::APP_VERSION.to_string(),
    };
    serde_json::to_writer(&mut file, &config).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn read_refreshmint_config(dir: &Path) -> io::Result<RefreshmintConfig> {
    let path = dir.join("refreshmint.json");
    let file = OpenOptions::new().read(true).open(path)?;
    serde_json::from_reader(file).map_err(io::Error::other)
}

fn create_general_journal(dir: &Path) -> io::Result<()> {
    let path = dir.join("general.journal");
    OpenOptions::new().create_new(true).write(true).open(path)?;
    Ok(())
}

fn init_git_repo(dir: &Path) -> io::Result<()> {
    run_git(
        dir,
        &[
            OsString::from("-c"),
            OsString::from("init.defaultBranch=main"),
            OsString::from("init"),
        ],
    )?;

    run_git(
        dir,
        &[
            OsString::from("add"),
            OsString::from("general.journal"),
            OsString::from("refreshmint.json"),
        ],
    )?;

    run_git(
        dir,
        &[
            OsString::from("-c"),
            OsString::from(format!("user.name={GIT_USER_NAME}")),
            OsString::from("-c"),
            OsString::from(format!("user.email={GIT_USER_EMAIL}")),
            OsString::from("commit"),
            OsString::from("-m"),
            OsString::from("Initial commit"),
        ],
    )?;

    Ok(())
}

fn run_git(dir: &Path, args: &[OsString]) -> io::Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_QUARANTINE_PATH")
        .env("GIT_AUTHOR_NAME", GIT_USER_NAME)
        .env("GIT_AUTHOR_EMAIL", GIT_USER_EMAIL)
        .env("GIT_COMMITTER_NAME", GIT_USER_NAME)
        .env("GIT_COMMITTER_EMAIL", GIT_USER_EMAIL)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "git command failed with status {status}"
        )))
    }
}

#[cfg(target_os = "macos")]
fn enable_bundle_attr_if_supported(dir: &Path) -> io::Result<()> {
    const ATTR_NAME: &str = "com.apple.FinderInfo";
    let existing = match xattr::get(dir, ATTR_NAME) {
        Ok(existing) => existing,
        Err(err) => {
            if is_xattr_unsupported(&err) {
                return Ok(());
            }
            return Err(err);
        }
    };

    let mut finder_info = existing.unwrap_or_else(|| vec![0u8; 32]);
    if finder_info.len() < 32 {
        finder_info.resize(32, 0);
    } else if finder_info.len() > 32 {
        finder_info.truncate(32);
    }

    // FolderInfo flags at offset 8. Set kHasBundle (0x2000).
    finder_info[8] |= 0x20;

    match xattr::set(dir, ATTR_NAME, &finder_info) {
        Ok(()) => Ok(()),
        Err(err) => {
            if is_xattr_unsupported(&err) {
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn is_xattr_unsupported(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    match err.raw_os_error() {
        Some(code) => code == libc::ENOTSUP || code == libc::EOPNOTSUPP,
        None => false,
    }
}

#[cfg(not(target_os = "macos"))]
fn enable_bundle_attr_if_supported(_dir: &Path) -> io::Result<()> {
    Ok(())
}
