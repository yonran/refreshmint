use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    let gl_account = require_non_empty("gl_account", input.gl_account)?;
    let statement_start_date = normalize_optional_date(input.statement_start_date)?;
    let statement_end_date = require_date("statement_end_date", input.statement_end_date)?;
    let reconciled_txn_ids = normalize_ids(input.reconciled_txn_ids);
    validate_reconciliation_membership(
        ledger_dir,
        &gl_account,
        statement_start_date.as_deref(),
        &statement_end_date,
        &reconciled_txn_ids,
    )?;
    let session = ReconciliationSession {
        id: uuid::Uuid::new_v4().to_string(),
        gl_account,
        statement_start_date,
        statement_end_date,
        statement_starting_balance: normalize_optional_string(input.statement_starting_balance),
        statement_ending_balance: require_non_empty(
            "statement_ending_balance",
            input.statement_ending_balance,
        )?,
        currency: normalize_optional_string(input.currency),
        status: ReconciliationSessionStatus::Draft,
        reconciled_txn_ids,
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
    let gl_account = require_non_empty("gl_account", input.gl_account)?;
    let statement_start_date = normalize_optional_date(input.statement_start_date)?;
    let statement_end_date = require_date("statement_end_date", input.statement_end_date)?;
    let reconciled_txn_ids = normalize_ids(input.reconciled_txn_ids);
    validate_reconciliation_membership(
        ledger_dir,
        &gl_account,
        statement_start_date.as_deref(),
        &statement_end_date,
        &reconciled_txn_ids,
    )?;
    existing.gl_account = gl_account;
    existing.statement_start_date = statement_start_date;
    existing.statement_end_date = statement_end_date;
    existing.statement_starting_balance =
        normalize_optional_string(input.statement_starting_balance);
    existing.statement_ending_balance =
        require_non_empty("statement_ending_balance", input.statement_ending_balance)?;
    existing.currency = normalize_optional_string(input.currency);
    existing.reconciled_txn_ids = reconciled_txn_ids;
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
    let session =
        read_required_json::<ReconciliationSession>(&reconciliation_session_path(ledger_dir, id))?;
    validate_reconciliation_membership(
        ledger_dir,
        &session.gl_account,
        session.statement_start_date.as_deref(),
        &session.statement_end_date,
        &session.reconciled_txn_ids,
    )?;
    validate_finalized_reconciliation_overlap(
        ledger_dir,
        &session.id,
        &session.gl_account,
        &session.reconciled_txn_ids,
    )?;
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
    let gl_txn_index = if matches!(input.left_ref.kind, TypedRefKind::GlTxn)
        || matches!(input.right_ref.kind, TypedRefKind::GlTxn)
    {
        Some(GlTxnIndex::load(ledger_dir)?)
    } else {
        None
    };
    validate_typed_ref("left_ref", &input.left_ref, gl_txn_index.as_ref())?;
    validate_typed_ref("right_ref", &input.right_ref, gl_txn_index.as_ref())?;
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

pub fn query_reconciliation_candidates(
    ledger_dir: &Path,
    gl_account: &str,
    statement_start_date: Option<&str>,
    statement_end_date: &str,
) -> io::Result<Vec<crate::ledger_open::TransactionRow>> {
    let gl_account = require_non_empty("gl_account", gl_account.to_string())?;
    let statement_end_date = require_date("statement_end_date", statement_end_date.to_string())?;
    let statement_start_date = match statement_start_date {
        Some(value) => Some(require_date("statement_start_date", value.to_string())?),
        None => None,
    };
    let journal_path = ledger_dir.join("general.journal");
    let transactions = crate::ledger_open::run_hledger_print(&journal_path)?;
    ensure_all_transactions_have_gl_ids(&transactions)?;
    let filtered: Vec<_> = transactions
        .into_iter()
        .filter(|txn| {
            txn.tpostings
                .iter()
                .any(|posting| posting.paccount == gl_account)
        })
        .filter(|txn| {
            if let Some(start) = statement_start_date.as_deref() {
                if txn.tdate.as_str() < start {
                    return false;
                }
            }
            txn.tdate.as_str() <= statement_end_date.as_str()
        })
        .collect();
    crate::ledger_open::build_transaction_rows(ledger_dir, &filtered)
}

pub fn repair_gl_txn_refs_after_merge(
    ledger_dir: &Path,
    old_txn_ids: &[&str],
    new_txn_id: &str,
) -> io::Result<()> {
    let mut replacement_map = HashMap::new();
    let old_txn_ids: BTreeSet<String> = old_txn_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if old_txn_ids.is_empty() {
        return Ok(());
    }
    if old_txn_ids.contains(new_txn_id) {
        return Ok(());
    }
    for old_id in &old_txn_ids {
        replacement_map.insert(old_id.clone(), new_txn_id.to_string());
    }

    let gl_txn_index = GlTxnIndex::load(ledger_dir)?;
    let new_txn = gl_txn_index.records.get(new_txn_id).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing merged GL transaction: {new_txn_id}"),
        )
    })?;

    let sessions = list_reconciliation_sessions(ledger_dir)?;
    let mut affected_finalized_sessions = Vec::new();
    for session in &sessions {
        if matches!(session.status, ReconciliationSessionStatus::Finalized)
            && session
                .reconciled_txn_ids
                .iter()
                .any(|id| old_txn_ids.contains(id))
        {
            affected_finalized_sessions.push(session.id.clone());
            if !new_txn.accounts.contains(&session.gl_account) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "merged transaction {new_txn_id} does not touch reconciled account {} for session {}",
                        session.gl_account, session.id
                    ),
                ));
            }
            let session_start = session
                .statement_start_date
                .as_deref()
                .map(|s| {
                    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("statement_start_date must be YYYY-MM-DD: {err}"),
                        )
                    })
                })
                .transpose()?;
            let session_end =
                chrono::NaiveDate::parse_from_str(&session.statement_end_date, "%Y-%m-%d")
                    .map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("statement_end_date must be YYYY-MM-DD: {err}"),
                        )
                    })?;
            validate_reconciliation_date(new_txn.date, session_start, session_end, new_txn_id)?;
        }
    }
    affected_finalized_sessions.sort();
    if affected_finalized_sessions.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot merge GL transactions across multiple finalized reconciliation sessions: {}",
                affected_finalized_sessions.join(", ")
            ),
        ));
    }

    let mut updates = Vec::new();
    for mut session in sessions {
        let updated_ids =
            crate::gl_journal::replace_txn_ids(&session.reconciled_txn_ids, &replacement_map);
        if updated_ids != session.reconciled_txn_ids {
            let original = session.clone();
            session.reconciled_txn_ids = updated_ids;
            session.updated_at = crate::operations::now_timestamp();
            updates.push(BookkeepingUpdate::Session {
                original: Box::new(original),
                updated: Box::new(session),
            });
        }
    }

    for mut link in list_links(ledger_dir)? {
        let mut changed = false;
        if matches!(link.left_ref.kind, TypedRefKind::GlTxn) {
            if let Some(id) = link.left_ref.id.as_ref() {
                if let Some(replacement) = replacement_map.get(id) {
                    link.left_ref.id = Some(replacement.clone());
                    changed = true;
                }
            }
        }
        if matches!(link.right_ref.kind, TypedRefKind::GlTxn) {
            if let Some(id) = link.right_ref.id.as_ref() {
                if let Some(replacement) = replacement_map.get(id) {
                    link.right_ref.id = Some(replacement.clone());
                    changed = true;
                }
            }
        }
        if changed {
            let original = read_required_json::<LinkRecord>(&link_path(ledger_dir, &link.id))?;
            link.updated_at = crate::operations::now_timestamp();
            updates.push(BookkeepingUpdate::Link {
                original: Box::new(original),
                updated: Box::new(link),
            });
        }
    }

    for mut close in list_period_closes(ledger_dir)? {
        let updated_ids =
            crate::gl_journal::replace_txn_ids(&close.adjustment_txn_ids, &replacement_map);
        if updated_ids != close.adjustment_txn_ids {
            let original = close.clone();
            close.adjustment_txn_ids = updated_ids;
            updates.push(BookkeepingUpdate::PeriodClose {
                original: Box::new(original),
                updated: Box::new(close),
            });
        }
    }

    apply_bookkeeping_updates(ledger_dir, &updates)
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

fn validate_typed_ref(
    field_name: &str,
    value: &TypedRef,
    gl_txn_index: Option<&GlTxnIndex>,
) -> io::Result<()> {
    match value.kind {
        TypedRefKind::GlTxn => {
            let Some(id) = value.id.as_deref().filter(|id| !id.is_empty()) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.id is required for gl-txn refs"),
                ));
            };
            let Some(index) = gl_txn_index else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.id could not be validated"),
                ));
            };
            if !index.records.contains_key(id) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{field_name}.id does not reference an existing GL transaction: {id}"),
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

#[derive(Debug, Clone)]
struct GlTxnInfo {
    date: chrono::NaiveDate,
    accounts: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct GlTxnIndex {
    records: BTreeMap<String, GlTxnInfo>,
}

impl GlTxnIndex {
    fn load(ledger_dir: &Path) -> io::Result<Self> {
        let journal_path = ledger_dir.join("general.journal");
        let transactions = crate::ledger_open::run_hledger_print(&journal_path)?;
        ensure_all_transactions_have_gl_ids(&transactions)?;
        let mut records = BTreeMap::new();
        for txn in transactions {
            let id = gl_transaction_id(&txn).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "general.journal contains a transaction without an id: tag; run migrate_ledger first",
                )
            })?;
            let date =
                chrono::NaiveDate::parse_from_str(&txn.tdate, "%Y-%m-%d").map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid GL transaction date {}: {err}", txn.tdate),
                    )
                })?;
            records.insert(
                id,
                GlTxnInfo {
                    date,
                    accounts: txn
                        .tpostings
                        .iter()
                        .map(|posting| posting.paccount.clone())
                        .collect(),
                },
            );
        }
        Ok(Self { records })
    }
}

fn gl_transaction_id(txn: &crate::hledger::Transaction) -> Option<String> {
    crate::ledger_open::gl_transaction_id(txn).map(ToOwned::to_owned)
}

fn ensure_all_transactions_have_gl_ids(
    transactions: &[crate::hledger::Transaction],
) -> io::Result<()> {
    if transactions
        .iter()
        .all(|txn| gl_transaction_id(txn).is_some())
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "general.journal contains transaction(s) without stable id: tags; run migrate_ledger first",
        ))
    }
}

fn validate_reconciliation_membership(
    ledger_dir: &Path,
    gl_account: &str,
    statement_start_date: Option<&str>,
    statement_end_date: &str,
    txn_ids: &[String],
) -> io::Result<()> {
    let gl_txn_index = GlTxnIndex::load(ledger_dir)?;
    let start = statement_start_date
        .map(|s| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("statement_start_date must be YYYY-MM-DD: {err}"),
                )
            })
        })
        .transpose()?;
    let end = chrono::NaiveDate::parse_from_str(statement_end_date, "%Y-%m-%d").map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("statement_end_date must be YYYY-MM-DD: {err}"),
        )
    })?;
    for txn_id in txn_ids {
        let Some(record) = gl_txn_index.records.get(txn_id) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown GL transaction id in reconciliation session: {txn_id}"),
            ));
        };
        if !record.accounts.contains(gl_account) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "GL transaction {txn_id} does not touch reconciliation account {gl_account}"
                ),
            ));
        }
        validate_reconciliation_date(record.date, start, end, txn_id)?;
    }
    Ok(())
}

fn validate_reconciliation_date(
    txn_date: chrono::NaiveDate,
    statement_start_date: Option<chrono::NaiveDate>,
    statement_end_date: chrono::NaiveDate,
    txn_id: &str,
) -> io::Result<()> {
    if let Some(start) = statement_start_date {
        if txn_date < start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("GL transaction {txn_id} is before statement start date"),
            ));
        }
    }
    if txn_date > statement_end_date {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("GL transaction {txn_id} is after statement end date"),
        ));
    }
    Ok(())
}

fn validate_finalized_reconciliation_overlap(
    ledger_dir: &Path,
    session_id: &str,
    gl_account: &str,
    txn_ids: &[String],
) -> io::Result<()> {
    let requested: BTreeSet<&str> = txn_ids.iter().map(String::as_str).collect();
    if requested.is_empty() {
        return Ok(());
    }
    for session in list_reconciliation_sessions(ledger_dir)? {
        if session.id == session_id
            || !matches!(session.status, ReconciliationSessionStatus::Finalized)
            || session.gl_account != gl_account
        {
            continue;
        }
        let overlap: Vec<&str> = session
            .reconciled_txn_ids
            .iter()
            .map(String::as_str)
            .filter(|id| requested.contains(id))
            .collect();
        if !overlap.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "GL transaction(s) already reconciled in finalized session {}: {}",
                    session.id,
                    overlap.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

enum BookkeepingUpdate {
    Session {
        original: Box<ReconciliationSession>,
        updated: Box<ReconciliationSession>,
    },
    Link {
        original: Box<LinkRecord>,
        updated: Box<LinkRecord>,
    },
    PeriodClose {
        original: Box<PeriodClose>,
        updated: Box<PeriodClose>,
    },
}

impl BookkeepingUpdate {
    fn apply(&self, ledger_dir: &Path) -> io::Result<()> {
        match self {
            Self::Session { updated, .. } => write_json(
                &reconciliation_session_path(ledger_dir, &updated.id),
                updated.as_ref(),
            ),
            Self::Link { updated, .. } => {
                write_json(&link_path(ledger_dir, &updated.id), updated.as_ref())
            }
            Self::PeriodClose { updated, .. } => write_json(
                &period_close_path(ledger_dir, &updated.period_id),
                updated.as_ref(),
            ),
        }
    }

    fn rollback(&self, ledger_dir: &Path) -> io::Result<()> {
        match self {
            Self::Session { original, .. } => write_json(
                &reconciliation_session_path(ledger_dir, &original.id),
                original.as_ref(),
            ),
            Self::Link { original, .. } => {
                write_json(&link_path(ledger_dir, &original.id), original.as_ref())
            }
            Self::PeriodClose { original, .. } => write_json(
                &period_close_path(ledger_dir, &original.period_id),
                original.as_ref(),
            ),
        }
    }
}

fn apply_bookkeeping_updates(ledger_dir: &Path, updates: &[BookkeepingUpdate]) -> io::Result<()> {
    for (applied_count, update) in updates.iter().enumerate() {
        if let Err(err) = update.apply(ledger_dir) {
            for rollback in updates[..applied_count].iter().rev() {
                let _ = rollback.rollback(ledger_dir);
            }
            return Err(err);
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
        fs::write(dir.join("general.journal"), "").unwrap();
        dir
    }

    fn write_general_journal(root: &Path, content: &str) {
        fs::write(root.join("general.journal"), content).unwrap();
    }

    #[test]
    fn reconciliation_sessions_round_trip_and_status_transitions() {
        let root = temp_ledger_dir("reconciliation");
        write_general_journal(
            &root,
            "2026-03-03 Example  ; id: gl-1\n  Assets:Checking  -25 USD\n  Expenses:Food  25 USD\n\n2026-03-04 Example  ; id: gl-2\n  Assets:Checking  -10 USD\n  Expenses:Food  10 USD\n",
        );

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
        write_general_journal(
            &root,
            "2026-03-03 Example  ; id: gl-1\n  Assets:Checking  -25 USD\n  Expenses:Food  25 USD\n",
        );

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

    #[test]
    fn create_reconciliation_session_rejects_unknown_or_out_of_period_txns() {
        let root = temp_ledger_dir("reconciliation-validation");
        write_general_journal(
            &root,
            "2026-03-03 Example  ; id: gl-1\n  Assets:Checking  -25 USD\n  Expenses:Food  25 USD\n\n2026-04-03 Later  ; id: gl-2\n  Assets:Checking  -10 USD\n  Expenses:Food  10 USD\n",
        );

        let err = create_reconciliation_session(
            &root,
            NewReconciliationSessionInput {
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-01".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: None,
                statement_ending_balance: "0 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["missing".to_string()],
                notes: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown GL transaction id"));

        let err = create_reconciliation_session(
            &root,
            NewReconciliationSessionInput {
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-01".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: None,
                statement_ending_balance: "0 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["gl-2".to_string()],
                notes: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("after statement end date"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finalize_reconciliation_session_rejects_overlap() {
        let root = temp_ledger_dir("reconciliation-overlap");
        write_general_journal(
            &root,
            "2026-03-03 Example  ; id: gl-1\n  Assets:Checking  -25 USD\n  Expenses:Food  25 USD\n",
        );
        let first = create_reconciliation_session(
            &root,
            NewReconciliationSessionInput {
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-01".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: None,
                statement_ending_balance: "0 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["gl-1".to_string()],
                notes: None,
            },
        )
        .unwrap();
        finalize_reconciliation_session(&root, &first.id).unwrap();

        let second = create_reconciliation_session(
            &root,
            NewReconciliationSessionInput {
                gl_account: "Assets:Checking".to_string(),
                statement_start_date: Some("2026-03-01".to_string()),
                statement_end_date: "2026-03-31".to_string(),
                statement_starting_balance: None,
                statement_ending_balance: "0 USD".to_string(),
                currency: Some("USD".to_string()),
                reconciled_txn_ids: vec!["gl-1".to_string()],
                notes: None,
            },
        )
        .unwrap();
        let err = finalize_reconciliation_session(&root, &second.id).unwrap_err();
        assert!(err.to_string().contains("already reconciled"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn create_link_rejects_missing_gl_transaction_ref() {
        let root = temp_ledger_dir("link-validation");
        write_general_journal(
            &root,
            "2026-03-03 Example  ; id: gl-1\n  Assets:Checking  -25 USD\n  Expenses:Food  25 USD\n",
        );

        let err = create_link(
            &root,
            NewLinkRecordInput {
                kind: LinkKind::SettlementLink,
                left_ref: TypedRef {
                    kind: TypedRefKind::GlTxn,
                    id: Some("missing".to_string()),
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
                amount: None,
                notes: None,
            },
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("does not reference an existing GL transaction"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repair_gl_txn_refs_after_merge_rewrites_bookkeeping_refs() {
        let root = temp_ledger_dir("merge-repair");
        write_general_journal(
            &root,
            "2026-03-03 Merged  ; id: gl-new\n  Assets:Checking  -25 USD\n  Assets:Savings  25 USD\n",
        );
        let session = ReconciliationSession {
            id: "rec-1".to_string(),
            gl_account: "Assets:Checking".to_string(),
            statement_start_date: Some("2026-03-01".to_string()),
            statement_end_date: "2026-03-31".to_string(),
            statement_starting_balance: None,
            statement_ending_balance: "0 USD".to_string(),
            currency: Some("USD".to_string()),
            status: ReconciliationSessionStatus::Finalized,
            reconciled_txn_ids: vec!["old-a".to_string()],
            notes: None,
            created_at: "2026-04-08T00:00:00Z".to_string(),
            updated_at: "2026-04-08T00:00:00Z".to_string(),
        };
        write_json(&reconciliation_session_path(&root, &session.id), &session).unwrap();
        let link = LinkRecord {
            id: "link-1".to_string(),
            kind: LinkKind::SettlementLink,
            left_ref: TypedRef {
                kind: TypedRefKind::GlTxn,
                id: Some("old-a".to_string()),
                locator: None,
                entry_id: None,
                login_name: None,
                label: None,
                filename: None,
            },
            right_ref: TypedRef {
                kind: TypedRefKind::GlTxn,
                id: Some("old-b".to_string()),
                locator: None,
                entry_id: None,
                login_name: None,
                label: None,
                filename: None,
            },
            amount: None,
            notes: None,
            created_at: "2026-04-08T00:00:00Z".to_string(),
            updated_at: "2026-04-08T00:00:00Z".to_string(),
        };
        write_json(&link_path(&root, &link.id), &link).unwrap();
        let close = PeriodClose {
            period_id: "2026-03".to_string(),
            status: PeriodCloseStatus::SoftClosed,
            closed_at: Some("2026-04-08T00:00:00Z".to_string()),
            closed_by: Some("owner".to_string()),
            notes: None,
            reconciliation_session_ids: vec!["rec-1".to_string()],
            adjustment_txn_ids: vec!["old-a".to_string(), "other".to_string()],
        };
        write_json(&period_close_path(&root, &close.period_id), &close).unwrap();

        repair_gl_txn_refs_after_merge(&root, &["old-a", "old-b"], "gl-new").unwrap();

        let updated_session = read_required_json::<ReconciliationSession>(
            &reconciliation_session_path(&root, "rec-1"),
        )
        .unwrap();
        assert_eq!(
            updated_session.reconciled_txn_ids,
            vec!["gl-new".to_string()]
        );
        let updated_link = read_required_json::<LinkRecord>(&link_path(&root, "link-1")).unwrap();
        assert_eq!(updated_link.left_ref.id.as_deref(), Some("gl-new"));
        assert_eq!(updated_link.right_ref.id.as_deref(), Some("gl-new"));
        let updated_close =
            read_required_json::<PeriodClose>(&period_close_path(&root, "2026-03")).unwrap();
        assert_eq!(
            updated_close.adjustment_txn_ids,
            vec!["gl-new".to_string(), "other".to_string()]
        );

        let _ = fs::remove_dir_all(root);
    }
}
