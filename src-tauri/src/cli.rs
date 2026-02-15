use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use std::error::Error;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

const DEFAULT_LEDGER_DIR: &str = "accounting.refreshmint";
const GIT_USER_NAME: &str = "Refreshmint";
const GIT_USER_EMAIL: &str = "refreshmint@noreply.example.com";
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

#[derive(Serialize)]
struct RefreshmintConfig<'a> {
    version: &'a str,
}

#[derive(Parser)]
#[command(name = "refreshmint", version = crate::version::APP_VERSION)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    New(NewArgs),
}

#[derive(Args)]
struct NewArgs {
    #[arg(long)]
    ledger: Option<PathBuf>,
}

pub fn run(context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::New(args)) => run_new(args, context),
        None => crate::run_with_context(context),
    }
}

fn run_new(args: NewArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    match args.ledger {
        Some(path) => run_new_with_ledger_path(path),
        None => {
            let target_dir = default_ledger_dir(context)?;
            run_new_at_dir(&target_dir)
        }
    }
}

fn run_new_with_ledger_path(path: PathBuf) -> Result<(), Box<dyn Error>> {
    let target_dir = ensure_refreshmint_extension(path)?;
    run_new_at_dir(&target_dir)
}

fn run_new_at_dir(target_dir: &Path) -> Result<(), Box<dyn Error>> {
    create_ledger_dir(target_dir)?;
    enable_bundle_attr_if_supported(target_dir)?;
    write_refreshmint_json(target_dir)?;
    create_general_journal(target_dir)?;
    init_git_repo(target_dir)?;
    Ok(())
}

fn ensure_refreshmint_extension(path: PathBuf) -> io::Result<PathBuf> {
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

fn default_ledger_dir(context: tauri::Context<tauri::Wry>) -> Result<PathBuf, Box<dyn Error>> {
    let app = tauri::Builder::default().build(context)?;
    let documents_dir = app.path().document_dir()?;
    Ok(documents_dir.join(DEFAULT_LEDGER_DIR))
}

fn create_ledger_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir(dir)
}

fn write_refreshmint_json(dir: &Path) -> io::Result<()> {
    let path = dir.join("refreshmint.json");
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let config = RefreshmintConfig {
        version: crate::version::APP_VERSION,
    };
    serde_json::to_writer(&mut file, &config).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{ensure_refreshmint_extension, run_new_with_ledger_path};
    use serde_json::Value;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ensure_refreshmint_extension_replaces_or_adds() {
        let no_extension = PathBuf::from("ledger");
        assert_eq!(
            expect_ok(ensure_refreshmint_extension(no_extension), "no extension"),
            PathBuf::from("ledger.refreshmint")
        );

        let other_extension = PathBuf::from("ledger.journal");
        assert_eq!(
            expect_ok(
                ensure_refreshmint_extension(other_extension),
                "other extension"
            ),
            PathBuf::from("ledger.refreshmint")
        );

        let already_refreshmint = PathBuf::from("ledger.refreshmint");
        assert_eq!(
            expect_ok(
                ensure_refreshmint_extension(already_refreshmint),
                "refreshmint extension"
            ),
            PathBuf::from("ledger.refreshmint")
        );
    }

    #[test]
    fn ensure_refreshmint_extension_rejects_empty_path() {
        let empty = PathBuf::from("");
        assert!(ensure_refreshmint_extension(empty).is_err());
    }

    #[test]
    fn new_command_creates_ledger_dir_and_git_repo() {
        if Command::new("git").arg("--version").status().is_err() {
            return;
        }

        let base_dir = create_temp_dir();
        let ledger_path = base_dir.join("ledger.journal");

        if let Err(err) = run_new_with_ledger_path(ledger_path) {
            panic!("run_new_with_ledger_path failed: {err}");
        }

        let ledger_dir = base_dir.join("ledger.refreshmint");
        if !ledger_dir.is_dir() {
            panic!("ledger directory was not created");
        }

        let refreshmint_json = ledger_dir.join("refreshmint.json");
        if !refreshmint_json.is_file() {
            panic!("refreshmint.json was not created");
        }

        let journal_path = ledger_dir.join("general.journal");
        if !journal_path.is_file() {
            panic!("general.journal was not created");
        }

        let json_contents = match fs::read_to_string(&refreshmint_json) {
            Ok(contents) => contents,
            Err(err) => {
                panic!("failed to read refreshmint.json: {err}");
            }
        };
        let json: Value = match serde_json::from_str(&json_contents) {
            Ok(json) => json,
            Err(err) => {
                panic!("failed to parse refreshmint.json: {err}");
            }
        };
        let version = match json.get("version").and_then(Value::as_str) {
            Some(version) => version,
            None => {
                panic!("refreshmint.json missing version");
            }
        };
        if version != crate::version::APP_VERSION {
            panic!(
                "refreshmint.json version {version} does not match {}",
                crate::version::APP_VERSION
            );
        }

        if !ledger_dir.join(".git").is_dir() {
            panic!(".git was not created");
        }

        let commit_subject = match git_output(&ledger_dir, &["log", "-1", "--pretty=%s"]) {
            Ok(output) => output,
            Err(err) => {
                panic!("git log failed: {err}");
            }
        };
        if commit_subject.trim() != "Initial commit" {
            panic!("unexpected git commit subject: {commit_subject}");
        }

        if let Err(err) = fs::remove_dir_all(&base_dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    fn expect_ok<T, E: std::fmt::Display>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(err) => {
                panic!("expected Ok for {label}, got error: {err}");
            }
        }
    }

    fn create_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir_name = format!("refreshmint-test-{}-{nanos}", std::process::id());
        let mut dir = std::env::temp_dir();
        dir.push(dir_name);
        if let Err(err) = fs::create_dir(&dir) {
            panic!("failed to create temp dir: {err}");
        }
        dir
    }

    fn git_output(dir: &Path, args: &[&str]) -> Result<String, io::Error> {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", super::NULL_DEVICE)
            .env("GIT_CONFIG_SYSTEM", super::NULL_DEVICE)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", super::GIT_USER_NAME)
            .env("GIT_AUTHOR_EMAIL", super::GIT_USER_EMAIL)
            .env("GIT_COMMITTER_NAME", super::GIT_USER_NAME)
            .env("GIT_COMMITTER_EMAIL", super::GIT_USER_EMAIL)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}
