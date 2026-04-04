use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const BOOKKEEPING_DIR: &str = "bookkeeping";
const RECONCILIATION_SESSIONS_DIR: &str = "reconciliation-sessions";
const LINKS_DIR: &str = "links";
const PERIOD_CLOSES_DIR: &str = "period-closes";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationSession {
    pub id: String,
    pub gl_account: String,
    pub statement_start_date: Option<String>,
    pub statement_end_date: String,
    pub statement_starting_balance: Option<String>,
    pub statement_ending_balance: String,
    pub currency: Option<String>,
    pub status: ReconciliationSessionStatus,
    pub reconciled_txn_ids: Vec<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReconciliationSessionStatus {
    Draft,
    Finalized,
    Reopened,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReconciliationSessionInput {
    pub gl_account: String,
    pub statement_start_date: Option<String>,
    pub statement_end_date: String,
    pub statement_starting_balance: Option<String>,
    pub statement_ending_balance: String,
    pub currency: Option<String>,
    pub reconciled_txn_ids: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReconciliationSessionInput {
    pub id: String,
    pub gl_account: String,
    pub statement_start_date: Option<String>,
    pub statement_end_date: String,
    pub statement_starting_balance: Option<String>,
    pub statement_ending_balance: String,
    pub currency: Option<String>,
    pub reconciled_txn_ids: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRecord {
    pub id: String,
    pub kind: LinkKind,
    pub left_ref: TypedRef,
    pub right_ref: TypedRef,
    pub amount: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    EvidenceLink,
    SettlementLink,
    SourceLink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedRef {
    pub kind: TypedRefKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

impl TypedRef {
    pub fn as_gl_txn_id(&self) -> Option<&str> {
        match self.kind {
            TypedRefKind::GlTxn => self.id.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TypedRefKind {
    GlTxn,
    LoginEntry,
    Document,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewLinkRecordInput {
    pub kind: LinkKind,
    pub left_ref: TypedRef,
    pub right_ref: TypedRef,
    pub amount: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodClose {
    pub period_id: String,
    pub status: PeriodCloseStatus,
    pub closed_at: Option<String>,
    pub closed_by: Option<String>,
    pub notes: Option<String>,
    pub reconciliation_session_ids: Vec<String>,
    pub adjustment_txn_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeriodCloseStatus {
    Draft,
    SoftClosed,
    Reopened,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertPeriodCloseInput {
    pub period_id: String,
    pub status: PeriodCloseStatus,
    pub closed_by: Option<String>,
    pub notes: Option<String>,
    pub reconciliation_session_ids: Vec<String>,
    pub adjustment_txn_ids: Vec<String>,
}

pub fn bookkeeping_dir(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join(BOOKKEEPING_DIR)
}

pub fn ensure_bookkeeping_layout(ledger_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(reconciliation_sessions_dir(ledger_dir))?;
    fs::create_dir_all(links_dir(ledger_dir))?;
    fs::create_dir_all(period_closes_dir(ledger_dir))?;
    Ok(())
}

pub fn list_reconciliation_sessions(ledger_dir: &Path) -> io::Result<Vec<ReconciliationSession>> {
    let mut sessions: Vec<ReconciliationSession> =
        read_json_objects_from_dir(&reconciliation_sessions_dir(ledger_dir))?;
    sessions.sort_by(|a, b| {
        b.statement_end_date
            .cmp(&a.statement_end_date)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(sessions)
}

pub fn create_reconciliation_session(
    ledger_dir: &Path,
    input: NewReconciliationSessionInput,
) -> io::Result<ReconciliationSession> {
    ensure_bookkeeping_layout(ledger_dir)?;
    let now = crate::operations::now_timestamp();
    let session = ReconciliationSession {
        id: uuid::Uuid::new_v4().to_string(),
        gl_account: require_non_empty("gl_account", input.gl_account)?,
        statement_start_date: normalize_optional_date(input.statement_start_date)?,
        statement_end_date: require_date("statement_end_date", input.statement_end_date)?,
        statement_starting_balance: normalize_optional_string(input.statement_starting_balance),
        statement_ending_balance: require_non_empty(
            "statement_ending_balance",
            input.statement_ending_balance,
        )?,
        currency: normalize_optional_string(input.currency),
        status: ReconciliationSessionStatus::Draft,
        reconciled_txn_ids: normalize_ids(input.reconciled_txn_ids),
        notes: normalize_optional_string(input.notes),
        created_at: now.clone(),
        updated_at: now,
    };
    write_json(
        &reconciliation_session_path(ledger_dir, &session.id),
        &session,
    )?;
    Ok(session)
}

pub fn update_reconciliation_session(
    ledger_dir: &Path,
    input: UpdateReconciliationSessionInput,
) -> io::Result<ReconciliationSession> {
    let mut existing = read_required_json::<ReconciliationSession>(&reconciliation_session_path(
        ledger_dir, &input.id,
    ))?;
    existing.gl_account = require_non_empty("gl_account", input.gl_account)?;
    existing.statement_start_date = normalize_optional_date(input.statement_start_date)?;
    existing.statement_end_date = require_date("statement_end_date", input.statement_end_date)?;
    existing.statement_starting_balance =
        normalize_optional_string(input.statement_starting_balance);
    existing.statement_ending_balance =
        require_non_empty("statement_ending_balance", input.statement_ending_balance)?;
    existing.currency = normalize_optional_string(input.currency);
    existing.reconciled_txn_ids = normalize_ids(input.reconciled_txn_ids);
    existing.notes = normalize_optional_string(input.notes);
    existing.updated_at = crate::operations::now_timestamp();
    write_json(
        &reconciliation_session_path(ledger_dir, &existing.id),
        &existing,
    )?;
    Ok(existing)
}

pub fn finalize_reconciliation_session(
    ledger_dir: &Path,
    id: &str,
) -> io::Result<ReconciliationSession> {
    set_reconciliation_status(ledger_dir, id, ReconciliationSessionStatus::Finalized)
}

pub fn reopen_reconciliation_session(
    ledger_dir: &Path,
    id: &str,
) -> io::Result<ReconciliationSession> {
    set_reconciliation_status(ledger_dir, id, ReconciliationSessionStatus::Reopened)
}

pub fn list_links(ledger_dir: &Path) -> io::Result<Vec<LinkRecord>> {
    let mut links: Vec<LinkRecord> = read_json_objects_from_dir(&links_dir(ledger_dir))?;
    links.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(links)
}

pub fn create_link(ledger_dir: &Path, input: NewLinkRecordInput) -> io::Result<LinkRecord> {
    ensure_bookkeeping_layout(ledger_dir)?;
    validate_typed_ref("left_ref", &input.left_ref)?;
    validate_typed_ref("right_ref", &input.right_ref)?;
    let now = crate::operations::now_timestamp();
    let link = LinkRecord {
        id: uuid::Uuid::new_v4().to_string(),
        kind: input.kind,
        left_ref: input.left_ref,
        right_ref: input.right_ref,
        amount: normalize_optional_string(input.amount),
        notes: normalize_optional_string(input.notes),
        created_at: now.clone(),
        updated_at: now,
    };
    write_json(&link_path(ledger_dir, &link.id), &link)?;
    Ok(link)
}

pub fn delete_link(ledger_dir: &Path, id: &str) -> io::Result<()> {
    let path = link_path(ledger_dir, id);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn list_period_closes(ledger_dir: &Path) -> io::Result<Vec<PeriodClose>> {
    let mut closes: Vec<PeriodClose> = read_json_objects_from_dir(&period_closes_dir(ledger_dir))?;
    closes.sort_by(|a, b| b.period_id.cmp(&a.period_id));
    Ok(closes)
}

pub fn upsert_period_close(
    ledger_dir: &Path,
    input: UpsertPeriodCloseInput,
) -> io::Result<PeriodClose> {
    ensure_bookkeeping_layout(ledger_dir)?;
    let period_id = require_period_id(input.period_id)?;
    let path = period_close_path(ledger_dir, &period_id);
    let existing = read_optional_json::<PeriodClose>(&path)?;
    let mut close = existing.unwrap_or(PeriodClose {
        period_id,
        status: PeriodCloseStatus::Draft,
        closed_at: None,
        closed_by: None,
        notes: None,
        reconciliation_session_ids: Vec::new(),
        adjustment_txn_ids: Vec::new(),
    });

    let should_stamp_closed_at = !matches!(close.status, PeriodCloseStatus::SoftClosed)
        && matches!(input.status, PeriodCloseStatus::SoftClosed);
    close.status = input.status;
    close.notes = normalize_optional_string(input.notes);
    close.reconciliation_session_ids = normalize_ids(input.reconciliation_session_ids);
    close.adjustment_txn_ids = normalize_ids(input.adjustment_txn_ids);
    if should_stamp_closed_at
        || (close.closed_at.is_none() && matches!(close.status, PeriodCloseStatus::SoftClosed))
    {
        close.closed_at = Some(crate::operations::now_timestamp());
        close.closed_by = normalize_optional_string(input.closed_by);
    } else if input.closed_by.is_some() {
        close.closed_by = normalize_optional_string(input.closed_by);
    }

    write_json(&path, &close)?;
    Ok(close)
}

pub fn reopen_period_close(ledger_dir: &Path, period_id: &str) -> io::Result<PeriodClose> {
    let normalized = require_period_id(period_id.to_string())?;
    let path = period_close_path(ledger_dir, &normalized);
    let mut close = read_required_json::<PeriodClose>(&path)?;
    close.status = PeriodCloseStatus::Reopened;
    write_json(&path, &close)?;
    Ok(close)
}

fn set_reconciliation_status(
    ledger_dir: &Path,
    id: &str,
    status: ReconciliationSessionStatus,
) -> io::Result<ReconciliationSession> {
    let path = reconciliation_session_path(ledger_dir, id);
    let mut session = read_required_json::<ReconciliationSession>(&path)?;
    session.status = status;
    session.updated_at = crate::operations::now_timestamp();
    write_json(&path, &session)?;
    Ok(session)
}

fn reconciliation_sessions_dir(ledger_dir: &Path) -> PathBuf {
    bookkeeping_dir(ledger_dir).join(RECONCILIATION_SESSIONS_DIR)
}

fn links_dir(ledger_dir: &Path) -> PathBuf {
    bookkeeping_dir(ledger_dir).join(LINKS_DIR)
}

fn period_closes_dir(ledger_dir: &Path) -> PathBuf {
    bookkeeping_dir(ledger_dir).join(PERIOD_CLOSES_DIR)
}

fn reconciliation_session_path(ledger_dir: &Path, id: &str) -> PathBuf {
    reconciliation_sessions_dir(ledger_dir).join(format!("{id}.json"))
}

fn link_path(ledger_dir: &Path, id: &str) -> PathBuf {
    links_dir(ledger_dir).join(format!("{id}.json"))
}

fn period_close_path(ledger_dir: &Path, period_id: &str) -> PathBuf {
    period_closes_dir(ledger_dir).join(format!("{period_id}.json"))
}

fn require_non_empty(field_name: &str, value: String) -> io::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{field_name} is required"),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn require_date(field_name: &str, value: String) -> io::Result<String> {
    let value = require_non_empty(field_name, value)?;
    chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{field_name} must be YYYY-MM-DD: {err}"),
        )
    })?;
    Ok(value)
}

fn normalize_optional_date(value: Option<String>) -> io::Result<Option<String>> {
    match normalize_optional_string(value) {
        Some(value) => require_date("date", value).map(Some),
        None => Ok(None),
    }
}

fn require_period_id(value: String) -> io::Result<String> {
    let value = require_non_empty("period_id", value)?;
    let candidate = format!("{value}-01");
    chrono::NaiveDate::parse_from_str(&candidate, "%Y-%m-%d").map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("period_id must be YYYY-MM: {err}"),
        )
    })?;
    Ok(value)
}

fn normalize_ids(ids: Vec<String>) -> Vec<String> {
    let mut ids: Vec<String> = ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn validate_typed_ref(field_name: &str, value: &TypedRef) -> io::Result<()> {
    match value.kind {
        TypedRefKind::GlTxn => {
            if value.id.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.id is required for gl-txn refs"),
                ));
            }
        }
        TypedRefKind::LoginEntry => {
            if value.locator.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.locator is required for login-entry refs"),
                ));
            }
            if value.entry_id.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.entryId is required for login-entry refs"),
                ));
            }
        }
        TypedRefKind::Document => {
            if value.login_name.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.loginName is required for document refs"),
                ));
            }
            if value.label.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.label is required for document refs"),
                ));
            }
            if value.filename.as_deref().map_or(true, str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.filename is required for document refs"),
                ));
            }
        }
    }
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{nanos}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bookkeeping.json"),
        std::process::id()
    ));

    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        serde_json::to_writer_pretty(&mut file, value).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }

    fs::rename(temp_path, path)?;
    Ok(())
}

fn read_required_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    read_optional_json(path)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing bookkeeping object: {}", path.display()),
        )
    })
}

fn read_optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = OpenOptions::new().read(true).open(path)?;
    let value = serde_json::from_reader(file).map_err(io::Error::other)?;
    Ok(Some(value))
}

fn read_json_objects_from_dir<T: for<'de> Deserialize<'de>>(dir: &Path) -> io::Result<Vec<T>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut values = Vec::with_capacity(paths.len());
    for path in paths {
        values.push(read_required_json(&path)?);
    }
    Ok(values)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_ledger_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-bookkeeping-{prefix}-{}-{now}.refreshmint",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reconciliation_sessions_round_trip_and_status_transitions() {
        let root = temp_ledger_dir("reconciliation");

        let created = create_reconciliation_session(
            &root,
            NewReconciliationSessionInput {
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-01".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: Some("100.00 USD".to_string()),
                statement_ending_balance: "75.00 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["gl-2".to_string(), "gl-1".to_string()],
                notes: Some("March statement".to_string()),
            },
        )
        .unwrap();
        assert!(matches!(created.status, ReconciliationSessionStatus::Draft));

        let updated = update_reconciliation_session(
            &root,
            UpdateReconciliationSessionInput {
                id: created.id.clone(),
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-02".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: Some("100.00 USD".to_string()),
                statement_ending_balance: "74.00 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["gl-1".to_string(), "gl-1".to_string()],
                notes: Some("Adjusted".to_string()),
            },
        )
        .unwrap();
        assert_eq!(updated.reconciled_txn_ids, vec!["gl-1".to_string()]);
        assert_eq!(updated.statement_start_date.as_deref(), Some("2026-03-02"));

        let finalized = finalize_reconciliation_session(&root, &created.id).unwrap();
        assert!(matches!(
            finalized.status,
            ReconciliationSessionStatus::Finalized
        ));

        let reopened = reopen_reconciliation_session(&root, &created.id).unwrap();
        assert!(matches!(
            reopened.status,
            ReconciliationSessionStatus::Reopened
        ));

        let listed = list_reconciliation_sessions(&root).unwrap();
        assert_eq!(listed.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn links_round_trip_and_delete() {
        let root = temp_ledger_dir("links");

        let created = create_link(
            &root,
            NewLinkRecordInput {
                kind: LinkKind::SettlementLink,
                left_ref: TypedRef {
                    kind: TypedRefKind::GlTxn,
                    id: Some("gl-1".to_string()),
                    locator: None,
                    entry_id: None,
                    login_name: None,
                    label: None,
                    filename: None,
                },
                right_ref: TypedRef {
                    kind: TypedRefKind::Document,
                    id: None,
                    locator: None,
                    entry_id: None,
                    login_name: Some("bank".to_string()),
                    label: Some("checking".to_string()),
                    filename: Some("stmt.pdf".to_string()),
                },
                amount: Some("25.00 USD".to_string()),
                notes: Some("Settles part of accrual".to_string()),
            },
        )
        .unwrap();
        let listed = list_links(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        delete_link(&root, &created.id).unwrap();
        assert!(list_links(&root).unwrap().is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn period_closes_round_trip_and_reopen() {
        let root = temp_ledger_dir("period-close");

        let closed = upsert_period_close(
            &root,
            UpsertPeriodCloseInput {
                period_id: "2026-03".to_string(),
                status: PeriodCloseStatus::SoftClosed,
                closed_by: Some("owner".to_string()),
                notes: Some("March is ready for review".to_string()),
                reconciliation_session_ids: vec!["rec-1".to_string()],
                adjustment_txn_ids: vec!["gl-adjust-1".to_string()],
            },
        )
        .unwrap();
        assert!(matches!(closed.status, PeriodCloseStatus::SoftClosed));
        assert!(closed.closed_at.is_some());

        let reopened = reopen_period_close(&root, "2026-03").unwrap();
        assert!(matches!(reopened.status, PeriodCloseStatus::Reopened));

        let listed = list_period_closes(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].period_id, "2026-03");

        let _ = fs::remove_dir_all(root);
    }
}
