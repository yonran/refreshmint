use std::io;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

const ALLOWED_COMMANDS: &[&str] = &[
    "balance",
    "balancesheet",
    "balancesheetequity",
    "cashflow",
    "incomestatement",
    "register",
    "aregister",
    "activity",
    "stats",
    "roi",
];

// Flags that control file I/O — must not be passed by the frontend
const BLOCKED_FLAG_PREFIXES: &[&str] = &["-f", "--file", "-o", "--output-file", "--output-format"];

#[derive(Debug, Serialize)]
pub struct ReportResult {
    /// Populated for CSV-output commands (all except stats/activity)
    pub rows: Vec<Vec<String>>,
    /// Populated for plain-text commands (stats, activity)
    pub text: Option<String>,
}

fn validate_args(command: &str, args: &[String]) -> io::Result<()> {
    if !ALLOWED_COMMANDS.contains(&command) {
        return Err(io::Error::other(format!(
            "Unknown report command: {command}"
        )));
    }
    for arg in args {
        for prefix in BLOCKED_FLAG_PREFIXES {
            if arg == prefix
                || arg.starts_with(&format!("{prefix}="))
                || arg.starts_with(&format!("{prefix} "))
            {
                return Err(io::Error::other(format!("Disallowed flag: {arg}")));
            }
        }
    }
    Ok(())
}

fn parse_csv_rows(bytes: &[u8]) -> io::Result<Vec<Vec<String>>> {
    let mut reader = csv::Reader::from_reader(bytes);
    let headers: Vec<String> = reader
        .headers()
        .map_err(io::Error::other)?
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut rows = vec![headers];
    for result in reader.records() {
        let record = result.map_err(io::Error::other)?;
        rows.push(record.iter().map(|s| s.to_string()).collect());
    }
    Ok(rows)
}

pub fn run_report(journal_path: &Path, command: &str, args: &[String]) -> io::Result<ReportResult> {
    validate_args(command, args)?;

    let text_commands = ["stats", "activity"];
    let use_text = text_commands.contains(&command);

    let mut cmd = Command::new(crate::binpath::hledger_path());
    cmd.arg(command)
        .arg("-f")
        .arg(journal_path)
        .env("GIT_CONFIG_GLOBAL", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1");

    if !use_text {
        cmd.arg("--output-format=csv");
    }

    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    if use_text {
        return Ok(ReportResult {
            rows: vec![],
            text: Some(String::from_utf8_lossy(&output.stdout).to_string()),
        });
    }

    Ok(ReportResult {
        rows: parse_csv_rows(&output.stdout)?,
        text: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // --- validate_args: command allowlist ---

    #[test]
    fn valid_commands_pass() {
        for cmd in ALLOWED_COMMANDS {
            assert!(
                validate_args(cmd, &[]).is_ok(),
                "expected {cmd} to be allowed"
            );
        }
    }

    #[test]
    fn unknown_command_is_rejected() {
        let err = validate_args("print", &[]).unwrap_err();
        assert!(
            err.to_string().contains("Unknown report command"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn empty_command_is_rejected() {
        let err = validate_args("", &[]).unwrap_err();
        assert!(err.to_string().contains("Unknown report command"));
    }

    // --- validate_args: blocked flags ---

    #[test]
    fn blocked_flag_exact_dash_f() {
        let err = validate_args("balance", &args(&["-f"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn blocked_flag_exact_double_dash_file() {
        let err = validate_args("balance", &args(&["--file"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn blocked_flag_equals_variant() {
        let err = validate_args("balance", &args(&["--file=other.journal"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn blocked_flag_output_file() {
        let err = validate_args("balance", &args(&["--output-file=out.csv"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn blocked_flag_output_format() {
        let err = validate_args("balance", &args(&["--output-format=json"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn blocked_flag_dash_o() {
        let err = validate_args("balance", &args(&["-o"])).unwrap_err();
        assert!(err.to_string().contains("Disallowed flag"));
    }

    #[test]
    fn allowed_flags_pass_validation() {
        // These look similar to blocked ones but are fine
        assert!(validate_args("balance", &args(&["-M", "-H", "--depth=2"])).is_ok());
        assert!(
            validate_args("register", &args(&["-b", "2024-01-01", "-e", "2024-12-31"])).is_ok()
        );
    }

    // --- parse_csv_rows ---

    #[test]
    fn parse_csv_empty_body() {
        let csv = b"account,balance\n";
        let rows = parse_csv_rows(csv).expect("parse failed");
        assert_eq!(rows, vec![vec!["account", "balance"]]);
    }

    #[test]
    fn parse_csv_with_data_rows() {
        let csv = b"account,balance\nAssets:Cash,\"1,000.00\"\nExpenses:Food,-42.00\n";
        let rows = parse_csv_rows(csv).expect("parse failed");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["account", "balance"]);
        assert_eq!(rows[1], vec!["Assets:Cash", "1,000.00"]);
        assert_eq!(rows[2], vec!["Expenses:Food", "-42.00"]);
    }

    #[test]
    fn parse_csv_multicolumn() {
        let csv = b"account,2024-01,2024-02,total\nExpenses:Food,10,20,30\n";
        let rows = parse_csv_rows(csv).expect("parse failed");
        assert_eq!(rows[0], vec!["account", "2024-01", "2024-02", "total"]);
        assert_eq!(rows[1], vec!["Expenses:Food", "10", "20", "30"]);
    }

    // -------------------------------------------------------------------------
    // Integration tests — require hledger on PATH.
    // Run with: cargo test report -- --ignored
    // -------------------------------------------------------------------------

    /// Write a temp journal with two months of transactions and return its path.
    /// The caller is responsible for deleting the file (Drop on the dir handles it).
    fn write_temp_journal() -> (tempdir::TempJournal, std::path::PathBuf) {
        let dir = tempdir::TempJournal::new();
        let path = dir.path.join("test.journal");
        std::fs::write(
            &path,
            "\
2024-01-15 Groceries
    Expenses:Food    $50.00
    Assets:Checking

2024-01-20 Salary
    Income:Salary   $-1000.00
    Assets:Checking

2024-02-10 Coffee
    Expenses:Food    $5.00
    Assets:Checking
",
        )
        .expect("write journal");
        (dir, path)
    }

    mod tempdir {
        use std::path::PathBuf;

        pub struct TempJournal {
            pub path: PathBuf,
        }

        impl TempJournal {
            pub fn new() -> Self {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir()
                    .join(format!("refreshmint-report-{}-{nanos}", std::process::id()));
                std::fs::create_dir_all(&path).expect("create temp dir");
                Self { path }
            }
        }

        impl Drop for TempJournal {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_balance_returns_csv_rows() {
        let (_dir, journal) = write_temp_journal();
        let result = run_report(&journal, "balance", &[]).expect("run_report failed");
        assert!(result.text.is_none());
        // Header row + at least one data row
        assert!(
            result.rows.len() >= 2,
            "expected at least 2 rows, got {}",
            result.rows.len()
        );
        assert_eq!(
            result.rows[0][0], "account",
            "first column header should be 'account'"
        );
        // All account names must be non-empty
        for row in result.rows.iter().skip(1) {
            assert!(!row[0].is_empty(), "account cell should not be empty");
        }
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_stats_returns_text() {
        let (_dir, journal) = write_temp_journal();
        let result = run_report(&journal, "stats", &[]).expect("run_report failed");
        assert!(result.rows.is_empty());
        let text = result.text.expect("stats should return text");
        assert!(
            text.contains("Txns"),
            "stats output should contain 'Txns'; got: {text}"
        );
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_monthly_balance_has_month_columns() {
        let (_dir, journal) = write_temp_journal();
        let result = run_report(&journal, "balance", &args(&["-M"])).expect("run_report failed");
        assert!(result.text.is_none());
        let header = &result.rows[0];
        // With -M and two months of data the header should contain a 2024-01 column
        assert!(
            header.iter().any(|h| h.contains("2024-01")),
            "expected a 2024-01 column; header = {header:?}"
        );
        assert!(
            header.iter().any(|h| h.contains("2024-02")),
            "expected a 2024-02 column; header = {header:?}"
        );
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_register_returns_running_total() {
        let (_dir, journal) = write_temp_journal();
        let result = run_report(&journal, "register", &args(&["Assets:Checking"]))
            .expect("run_report failed");
        assert!(result.text.is_none());
        // register CSV: txnidx, date, description, account, amount, balance
        assert!(result.rows.len() >= 2, "expected header + data rows");
        let header = &result.rows[0];
        assert!(
            header.iter().any(|h| h == "total"),
            "register CSV should have a 'total' column; header = {header:?}"
        );
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_incomestatement_has_income_and_expenses() {
        let (_dir, journal) = write_temp_journal();
        let result = run_report(&journal, "incomestatement", &[]).expect("run_report failed");
        assert!(result.text.is_none());
        assert!(result.rows.len() >= 2);
        // The account column should contain both Income and Expenses accounts
        let accounts: Vec<&str> = result.rows.iter().skip(1).map(|r| r[0].as_str()).collect();
        assert!(
            accounts.iter().any(|a| a.starts_with("Income")),
            "expected an Income row; accounts = {accounts:?}"
        );
        assert!(
            accounts.iter().any(|a| a.starts_with("Expenses")),
            "expected an Expenses row; accounts = {accounts:?}"
        );
    }

    #[test]
    #[ignore = "requires hledger on PATH"]
    fn integration_nonexistent_journal_returns_error() {
        let path = std::path::Path::new("/nonexistent/path/test.journal");
        let err = run_report(path, "balance", &[]).unwrap_err();
        assert!(
            !err.to_string().is_empty(),
            "expected a non-empty error message"
        );
    }
}
