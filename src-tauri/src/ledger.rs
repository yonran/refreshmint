use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
    commit_paths(dir, &[Path::new("general.journal")], message)
}

/// Commit general.journal plus a login account journal after a single-entry post.
pub(crate) fn commit_post_changes(
    dir: &Path,
    login_name: &str,
    label: &str,
    message: &str,
) -> io::Result<()> {
    let acct_rel = PathBuf::from("logins")
        .join(login_name)
        .join("accounts")
        .join(label)
        .join("account.journal");
    commit_paths(dir, &[Path::new("general.journal"), &acct_rel], message)
}

/// Commit general.journal plus two login account journals after a transfer post.
pub(crate) fn commit_transfer_changes(
    dir: &Path,
    login_name1: &str,
    label1: &str,
    login_name2: &str,
    label2: &str,
    message: &str,
) -> io::Result<()> {
    let acct_rel1 = PathBuf::from("logins")
        .join(login_name1)
        .join("accounts")
        .join(label1)
        .join("account.journal");
    let acct_rel2 = PathBuf::from("logins")
        .join(login_name2)
        .join("accounts")
        .join(label2)
        .join("account.journal");
    commit_paths(
        dir,
        &[Path::new("general.journal"), &acct_rel1, &acct_rel2],
        message,
    )
}

fn commit_paths(dir: &Path, paths: &[&Path], message: &str) -> io::Result<()> {
    let repo = git2::Repository::open(dir).map_err(|e| io::Error::other(e.to_string()))?;
    let mut index = repo.index().map_err(|e| io::Error::other(e.to_string()))?;
    for path in paths {
        index
            .add_path(path)
            .map_err(|e| io::Error::other(e.to_string()))?;
    }
    index.write().map_err(|e| io::Error::other(e.to_string()))?;
    let tree_oid = index
        .write_tree()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let sig = git2::Signature::now(GIT_USER_NAME, GIT_USER_EMAIL)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let head = repo.head().map_err(|e| io::Error::other(e.to_string()))?;
    let parent = head
        .peel_to_commit()
        .map_err(|e| io::Error::other(e.to_string()))?;
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .map_err(|e| io::Error::other(e.to_string()))?;
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
    let repo = git2::Repository::init(dir).map_err(|e| io::Error::other(e.to_string()))?;

    // Set default branch to main
    repo.config()
        .and_then(|mut cfg| cfg.set_str("init.defaultBranch", "main"))
        .map_err(|e| io::Error::other(e.to_string()))?;
    repo.set_head("refs/heads/main")
        .map_err(|e| io::Error::other(e.to_string()))?;

    // Stage files
    let mut index = repo.index().map_err(|e| io::Error::other(e.to_string()))?;
    index
        .add_path(Path::new("general.journal"))
        .map_err(|e| io::Error::other(e.to_string()))?;
    index
        .add_path(Path::new("refreshmint.json"))
        .map_err(|e| io::Error::other(e.to_string()))?;
    index.write().map_err(|e| io::Error::other(e.to_string()))?;
    let tree_oid = index
        .write_tree()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| io::Error::other(e.to_string()))?;

    // Create initial commit (no parents)
    let sig = git2::Signature::now(GIT_USER_NAME, GIT_USER_EMAIL)
        .map_err(|e| io::Error::other(e.to_string()))?;
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .map_err(|e| io::Error::other(e.to_string()))?;

    Ok(())
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
