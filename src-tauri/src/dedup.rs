use crate::account_journal::{AccountEntry, EntryStatus, SimpleAmount};
use crate::bookkeeping::{TypedRef, TypedRefKind};
use crate::extract::ExtractedTransaction;
use crate::operations;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Default)]
struct AttachmentIndex {
    by_key: BTreeMap<String, Vec<String>>,
}

fn attachment_index_from_documents(docs: &[crate::extract::DocumentWithInfo]) -> AttachmentIndex {
    let mut by_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for doc in docs {
        let Some(info) = &doc.info else {
            continue;
        };
        let Some(key) = info.metadata.get("attachmentKey").and_then(|v| v.as_str()) else {
            continue;
        };
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        by_key
            .entry(trimmed.to_string())
            .or_default()
            .insert(doc.filename.clone());
    }

    let by_key = by_key
        .into_iter()
        .map(|(key, files)| (key, files.into_iter().collect()))
        .collect();
    AttachmentIndex { by_key }
}

fn build_attachment_index_for_account(ledger_dir: &Path, account_name: &str) -> AttachmentIndex {
    match crate::extract::list_documents(ledger_dir, account_name) {
        Ok(docs) => attachment_index_from_documents(&docs),
        Err(err) => {
            eprintln!("warning: failed to index account attachments: {err}");
            AttachmentIndex::default()
        }
    }
}

fn build_attachment_index_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> AttachmentIndex {
    match crate::extract::list_documents_for_login_account(ledger_dir, login_name, label) {
        Ok(docs) => attachment_index_from_documents(&docs),
        Err(err) => {
            eprintln!("warning: failed to index login account attachments: {err}");
            AttachmentIndex::default()
        }
    }
}

fn add_attachment_evidence_refs(
    entry: &mut AccountEntry,
    txn: &ExtractedTransaction,
    index: &AttachmentIndex,
) {
    let keys = attachment_keys_with_variants(txn);

    for key in keys {
        let Some(files) = index.by_key.get(&key) else {
            continue;
        };
        for filename in files {
            entry.add_evidence(format!("{filename}#attachment"));
        }
    }
}

fn attachment_keys_with_variants(txn: &ExtractedTransaction) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for key in txn.attachment_keys() {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        keys.insert(trimmed.to_string());
        if let Some(flip) = check_key_sign_flip(trimmed) {
            keys.insert(flip);
        }
    }
    keys
}

fn check_key_sign_flip(key: &str) -> Option<String> {
    // check:<checkNumber>|<YYYY-MM-DD>|<amount>
    if !key.starts_with("check:") {
        return None;
    }
    let mut parts = key.split('|');
    let left = parts.next()?;
    let middle = parts.next()?;
    let amount = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if amount.trim().is_empty() {
        return None;
    }
    let flipped = if let Some(stripped) = amount.strip_prefix('-') {
        stripped.to_string()
    } else {
        format!("-{amount}")
    };
    Some(format!("{left}|{middle}|{flipped}"))
}

/// Result of processing a single proposed transaction through the dedup engine.
#[derive(Debug)]
pub enum DedupResult {
    /// Matched an existing entry by exact evidence reference (same document + row).
    SameEvidence {
        existing_index: usize,
        updated: bool,
    },
    /// Matched an existing entry by bankId across documents.
    BankIdMatch { existing_index: usize },
    /// Fuzzy matched an existing entry (date ±1 day, same amount, similar description).
    FuzzyMatch { existing_index: usize },
    /// Explicitly matched by a durable same-source resolution.
    ResolutionMatch { existing_index: usize },
    /// Pending→finalized transition.
    PendingToFinalized { existing_index: usize },
    /// New transaction, no match found.
    New,
    /// Ambiguous: multiple candidates found, needs human review.
    Ambiguous { candidate_indices: Vec<usize> },
}

/// Tolerance settings for dedup matching.
pub struct DedupConfig {
    /// Maximum number of days difference for fuzzy date matching.
    pub date_tolerance_days: i64,
    /// Amount tolerance for pending→finalized (absolute).
    pub pending_finalized_amount_abs: f64,
    /// Amount tolerance for pending→finalized (relative, e.g. 0.20 = 20%).
    pub pending_finalized_amount_pct: f64,
}

#[derive(Default)]
pub struct DedupPolicy {
    same_source: BTreeSet<(String, String)>,
    not_same_source: BTreeSet<(String, String)>,
}

impl DedupPolicy {
    fn force_match(&mut self, entry_id: &str, evidence_ref: &str) {
        self.same_source
            .insert((entry_id.to_string(), evidence_ref.to_string()));
    }

    fn prevent_match(&mut self, entry_id: &str, evidence_ref: &str) {
        self.not_same_source
            .insert((entry_id.to_string(), evidence_ref.to_string()));
    }

    fn forces_match(&self, entry_id: &str, evidence_refs: &[String]) -> bool {
        evidence_refs.iter().any(|evidence_ref| {
            self.same_source
                .contains(&(entry_id.to_string(), evidence_ref.clone()))
        })
    }

    fn prevents_match(&self, entry_id: &str, evidence_refs: &[String]) -> bool {
        evidence_refs.iter().any(|evidence_ref| {
            self.not_same_source
                .contains(&(entry_id.to_string(), evidence_ref.clone()))
        })
    }
}

fn login_entry_id(subject: &TypedRef, login_name: &str, label: &str) -> Option<String> {
    if subject.kind != TypedRefKind::LoginEntry {
        return None;
    }
    if subject.login_name.as_deref() != Some(login_name) {
        return None;
    }
    if subject.label.as_deref() != Some(label) {
        return None;
    }
    subject.entry_id.clone()
}

fn evidence_ref_values(subject: &TypedRef) -> Vec<String> {
    if subject.kind == TypedRefKind::EvidenceRow {
        return [subject.locator.as_deref(), subject.id.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect();
    }

    // Backward compatibility for resolutions created before evidence-row refs
    // existed: Pipeline stored evidence locators as document refs with the same
    // value in locator/id/filename.
    [
        subject.locator.as_deref(),
        subject.id.as_deref(),
        subject.filename.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToString::to_string)
    .collect()
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            date_tolerance_days: 1,
            pending_finalized_amount_abs: 5.0,
            pending_finalized_amount_pct: 0.20,
        }
    }
}

/// Run dedup on a set of proposed transactions against existing account journal entries.
///
/// Returns a list of `DedupAction` describing what to do for each proposed transaction.
pub fn run_dedup(
    existing: &[AccountEntry],
    proposed: &[ExtractedTransaction],
    source_document: &str,
    config: &DedupConfig,
) -> Vec<DedupAction> {
    run_dedup_with_policy(
        existing,
        proposed,
        source_document,
        config,
        &DedupPolicy::default(),
    )
}

pub fn run_dedup_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    existing: &[AccountEntry],
    proposed: &[ExtractedTransaction],
    source_document: &str,
    config: &DedupConfig,
) -> Vec<DedupAction> {
    let policy =
        DedupPolicy::from_resolutions(ledger_dir, login_name, label).unwrap_or_else(|err| {
            eprintln!("warning: failed to load dedup resolutions: {err}");
            DedupPolicy::default()
        });
    run_dedup_with_policy(existing, proposed, source_document, config, &policy)
}

pub fn run_dedup_with_policy(
    existing: &[AccountEntry],
    proposed: &[ExtractedTransaction],
    source_document: &str,
    config: &DedupConfig,
    policy: &DedupPolicy,
) -> Vec<DedupAction> {
    let mut actions = Vec::new();
    // Track which existing entries have been matched (one-time consumption).
    let mut matched_existing: Vec<bool> = vec![false; existing.len()];

    for txn in proposed {
        let result = match_proposed(
            existing,
            txn,
            source_document,
            config,
            &matched_existing,
            policy,
        );
        match &result {
            DedupResult::SameEvidence { existing_index, .. }
            | DedupResult::BankIdMatch { existing_index }
            | DedupResult::FuzzyMatch { existing_index }
            | DedupResult::ResolutionMatch { existing_index }
            | DedupResult::PendingToFinalized { existing_index } => {
                matched_existing[*existing_index] = true;
            }
            DedupResult::New | DedupResult::Ambiguous { .. } => {}
        }
        actions.push(DedupAction {
            proposed: txn.clone(),
            source_document: source_document.to_string(),
            result,
        });
    }

    actions
}

impl DedupPolicy {
    fn from_resolutions(ledger_dir: &Path, login_name: &str, label: &str) -> std::io::Result<Self> {
        let mut policy = DedupPolicy::default();
        for resolution in crate::automation::list_resolutions(ledger_dir)?
            .into_iter()
            .filter(|resolution| resolution.status == crate::automation::ResolutionStatus::Active)
        {
            if !matches!(
                resolution.kind,
                crate::automation::ResolutionKind::SameSource
                    | crate::automation::ResolutionKind::NotSameSource
            ) {
                continue;
            }
            let entry_ids = resolution
                .subject_refs
                .iter()
                .filter_map(|subject| login_entry_id(subject, login_name, label))
                .collect::<Vec<_>>();
            let evidence_refs = resolution
                .subject_refs
                .iter()
                .flat_map(evidence_ref_values)
                .collect::<Vec<_>>();
            for entry_id in &entry_ids {
                for evidence_ref in &evidence_refs {
                    match resolution.kind {
                        crate::automation::ResolutionKind::SameSource => {
                            policy.force_match(entry_id, evidence_ref);
                        }
                        crate::automation::ResolutionKind::NotSameSource => {
                            policy.prevent_match(entry_id, evidence_ref);
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(policy)
    }
}

/// A dedup action: the proposed transaction paired with its match result.
pub struct DedupAction {
    pub proposed: ExtractedTransaction,
    pub source_document: String,
    pub result: DedupResult,
}

/// Apply dedup actions to update the account journal entries.
///
/// Returns the updated list of entries.
pub fn apply_dedup_actions(
    ledger_dir: &Path,
    account_name: &str,
    entries: Vec<AccountEntry>,
    actions: &[DedupAction],
    default_account: &str,
    staging_account: &str,
    extracted_by: Option<&str>,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let attachment_index = build_attachment_index_for_account(ledger_dir, account_name);
    // Legacy accounts/<name> path files no anomalies (no login/label context).
    apply_dedup_actions_with_logger(
        entries,
        actions,
        default_account,
        staging_account,
        extracted_by,
        Some(&attachment_index),
        None,
        |op| operations::append_account_operation(ledger_dir, account_name, op),
    )
}

/// Apply dedup actions for a login account journal.
pub fn apply_dedup_actions_for_login_account(
    ledger_dir: &Path,
    login_account: (&str, &str),
    entries: Vec<AccountEntry>,
    actions: &[DedupAction],
    default_account: &str,
    staging_account: &str,
    extracted_by: Option<&str>,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let (login_name, label) = login_account;
    let attachment_index = build_attachment_index_for_login_account(ledger_dir, login_name, label);
    create_ambiguous_dedup_anomalies(ledger_dir, login_name, label, &entries, actions)?;
    let anomaly_ctx = AnomalyContext {
        ledger_dir,
        login_name,
        label,
    };
    apply_dedup_actions_with_logger(
        entries,
        actions,
        default_account,
        staging_account,
        extracted_by,
        Some(&attachment_index),
        Some(&anomaly_ctx),
        |op| operations::append_login_account_operation(ledger_dir, login_name, label, op),
    )
}

pub fn apply_coverage_lifecycle_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    document_name: &str,
    proposed: &[ExtractedTransaction],
    mut entries: Vec<AccountEntry>,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let Some((coverage_start, coverage_end)) =
        document_coverage_for_login_account(ledger_dir, login_name, label, document_name)?
    else {
        // No coverage info sidecar: the disappeared-entry lifecycle can't run for
        // this document. Keep the entries unchanged, but file an anomaly so the
        // silent self-disabling is visible instead of vanishing. Dedup key is
        // (kind, login, label, source_entry_id, coverage_document); with a fixed
        // source_entry_id sentinel it dedups per document across repeated imports.
        crate::bookkeeping::create_import_anomaly(
            ledger_dir,
            crate::bookkeeping::NewImportAnomalyInput {
                kind: crate::bookkeeping::ImportAnomalyKind::CoverageInfoMissing,
                login_name: login_name.to_string(),
                label: label.to_string(),
                source_entry_id: "(coverage-info)".to_string(),
                gl_txn_id: None,
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                amount: None,
                description: format!(
                    "document '{document_name}' has no coverage info; disappeared-entry detection skipped"
                ),
                evidence: Vec::new(),
                coverage_document: document_name.to_string(),
                safe_to_retire: false,
                safety_reasons: vec![
                    "no coverage window is known for this document".to_string(),
                ],
                notes: None,
            },
        )?;
        return Ok(entries);
    };

    let mut retained = Vec::with_capacity(entries.len());
    for entry in entries.drain(..) {
        if !entry_date_is_covered(&entry.date, coverage_start, coverage_end) {
            retained.push(entry);
            continue;
        }
        if proposed.iter().any(|txn| entry_matches_txn(&entry, txn)) {
            retained.push(entry);
            continue;
        }
        if entry
            .evidence
            .iter()
            .any(|ev| entry_is_from_same_document_evidence(ev, document_name))
        {
            retained.push(entry);
            continue;
        }

        match entry.status {
            EntryStatus::Pending if entry.posted.is_none() && entry.posted_postings.is_empty() => {
                let op = operations::AccountOperation::EntryRetired {
                    entry_id: entry.id.clone(),
                    reason: format!(
                        "pending source entry absent from covered document {document_name}"
                    ),
                    timestamp: operations::now_timestamp(),
                };
                operations::append_login_account_operation(ledger_dir, login_name, label, &op)?;
            }
            EntryStatus::Pending => {
                create_missing_import_anomaly(
                    ledger_dir,
                    MissingImportAnomaly {
                        login_name,
                        label,
                        document_name,
                        entry: &entry,
                        kind: crate::bookkeeping::ImportAnomalyKind::UnsafePendingRetirement,
                        safe_to_retire: false,
                        safety_reasons: vec![
                            "pending entry is already posted or partially posted".to_string()
                        ],
                    },
                )?;
                retained.push(entry);
            }
            EntryStatus::Cleared => {
                create_missing_import_anomaly(
                    ledger_dir,
                    MissingImportAnomaly {
                        login_name,
                        label,
                        document_name,
                        entry: &entry,
                        kind:
                            crate::bookkeeping::ImportAnomalyKind::FinalizedMissingFromCoveredExport,
                        safe_to_retire: false,
                        safety_reasons: vec![
                            "bank-cleared source entries are authoritative".to_string()
                        ],
                    },
                )?;
                retained.push(entry);
            }
            EntryStatus::Unmarked => retained.push(entry),
        }
    }

    Ok(retained)
}

fn create_ambiguous_dedup_anomalies(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entries: &[AccountEntry],
    actions: &[DedupAction],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for action in actions {
        let DedupResult::Ambiguous { candidate_indices } = &action.result else {
            continue;
        };
        for candidate_index in candidate_indices {
            let Some(entry) = entries.get(*candidate_index) else {
                continue;
            };
            crate::bookkeeping::create_import_anomaly(
                ledger_dir,
                crate::bookkeeping::NewImportAnomalyInput {
                    kind: crate::bookkeeping::ImportAnomalyKind::DuplicateImportRepairSkipped,
                    login_name: login_name.to_string(),
                    label: label.to_string(),
                    source_entry_id: entry.id.clone(),
                    gl_txn_id: entry.posted.as_deref().map(gl_ref_txn_id),
                    date: action.proposed.tdate.clone(),
                    amount: txn_primary_simple_amount(&action.proposed)
                        .map(|amount| format!("{} {}", amount.quantity, amount.commodity)),
                    description: action.proposed.tdescription.clone(),
                    evidence: action.proposed.evidence_refs(),
                    coverage_document: action.source_document.clone(),
                    safe_to_retire: false,
                    safety_reasons: vec![
                        "multiple existing source entries match this extracted row".to_string(),
                    ],
                    notes: Some(format!(
                        "candidate source entry {}; choose same-source or not-same-source",
                        entry.id
                    )),
                },
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_dedup_actions_with_logger<F>(
    mut entries: Vec<AccountEntry>,
    actions: &[DedupAction],
    default_account: &str,
    staging_account: &str,
    extracted_by: Option<&str>,
    attachment_index: Option<&AttachmentIndex>,
    anomaly_ctx: Option<&AnomalyContext<'_>>,
    mut log_operation: F,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&operations::AccountOperation) -> std::io::Result<()>,
{
    for action in actions {
        match &action.result {
            DedupResult::SameEvidence {
                existing_index,
                updated,
            } => {
                if *updated {
                    let old_amounts = entry_posting_amounts(&entries[*existing_index]);
                    update_entry_from_proposed(&mut entries[*existing_index], &action.proposed);
                    maybe_record_posted_leg_amount_drift(
                        anomaly_ctx,
                        &entries[*existing_index],
                        action,
                        &old_amounts,
                    )?;
                }
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
                if let Some(index) = attachment_index {
                    add_attachment_evidence_refs(
                        &mut entries[*existing_index],
                        &action.proposed,
                        index,
                    );
                }
            }
            DedupResult::BankIdMatch { existing_index }
            | DedupResult::FuzzyMatch { existing_index }
            | DedupResult::ResolutionMatch { existing_index } => {
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
                if let Some(index) = attachment_index {
                    add_attachment_evidence_refs(
                        &mut entries[*existing_index],
                        &action.proposed,
                        index,
                    );
                }
                if is_more_finalized(&action.proposed.status(), &entries[*existing_index].status) {
                    entries[*existing_index].status = action.proposed.status();
                }
                if !action.proposed.tcomment.is_empty()
                    && entries[*existing_index].comment.is_empty()
                {
                    entries[*existing_index].comment = action.proposed.tcomment.clone();
                }
                if !amounts_equal(
                    &entry_primary_amount(&entries[*existing_index]),
                    &txn_primary_amount(&action.proposed),
                ) {
                    let old_amounts = entry_posting_amounts(&entries[*existing_index]);
                    update_entry_amount_from_proposed(
                        &mut entries[*existing_index],
                        &action.proposed,
                    );
                    maybe_record_posted_leg_amount_drift(
                        anomaly_ctx,
                        &entries[*existing_index],
                        action,
                        &old_amounts,
                    )?;
                }
            }
            DedupResult::PendingToFinalized { existing_index } => {
                entries[*existing_index].status = EntryStatus::Cleared;
                let old_amounts = entry_posting_amounts(&entries[*existing_index]);
                update_entry_from_proposed(&mut entries[*existing_index], &action.proposed);
                maybe_record_posted_leg_amount_drift(
                    anomaly_ctx,
                    &entries[*existing_index],
                    action,
                    &old_amounts,
                )?;
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
                if let Some(index) = attachment_index {
                    add_attachment_evidence_refs(
                        &mut entries[*existing_index],
                        &action.proposed,
                        index,
                    );
                }
            }
            DedupResult::New => {
                let mut entry = action
                    .proposed
                    .to_account_entry(default_account, staging_account);
                if let Some(eb) = extracted_by {
                    entry.extracted_by = Some(eb.to_string());
                }
                if let Some(index) = attachment_index {
                    add_attachment_evidence_refs(&mut entry, &action.proposed, index);
                }

                let op = operations::AccountOperation::EntryCreated {
                    entry_id: entry.id.clone(),
                    evidence: entry.evidence.clone(),
                    date: entry.date.clone(),
                    amount: entry
                        .postings
                        .first()
                        .and_then(|p| p.amount.as_ref())
                        .map(|a| a.quantity.clone())
                        .unwrap_or_default(),
                    tags: entry.tags.clone(),
                    timestamp: operations::now_timestamp(),
                };
                log_operation(&op)?;
                entries.push(entry);
            }
            DedupResult::Ambiguous { .. } => {
                eprintln!(
                    "Ambiguous match for transaction: {} {}",
                    action.proposed.tdate, action.proposed.tdescription
                );
            }
        }
    }

    Ok(entries)
}

fn match_proposed(
    existing: &[AccountEntry],
    txn: &ExtractedTransaction,
    source_document: &str,
    config: &DedupConfig,
    matched: &[bool],
    policy: &DedupPolicy,
) -> DedupResult {
    let evidence_refs = txn.evidence_refs();

    // Step 0: Explicit source-relationship resolutions.
    let forced_candidates = existing
        .iter()
        .enumerate()
        .filter(|(i, entry)| !matched[*i] && policy.forces_match(&entry.id, &evidence_refs))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if forced_candidates.len() == 1 {
        return DedupResult::ResolutionMatch {
            existing_index: forced_candidates[0],
        };
    }
    if forced_candidates.len() > 1 {
        return DedupResult::Ambiguous {
            candidate_indices: forced_candidates,
        };
    }

    // Step 1: Same-evidence match
    for (i, entry) in existing.iter().enumerate() {
        if matched[i] || policy.prevents_match(&entry.id, &evidence_refs) {
            continue;
        }
        for ev in &evidence_refs {
            if entry.has_evidence(ev) {
                let updated = has_content_changed(entry, txn);
                return DedupResult::SameEvidence {
                    existing_index: i,
                    updated,
                };
            }
        }
    }

    // Step 2: Exact match by bankId (across other documents)
    if let Some(bank_id) = txn.bank_id() {
        let mut candidates = Vec::new();
        for (i, entry) in existing.iter().enumerate() {
            if matched[i] || policy.prevents_match(&entry.id, &evidence_refs) {
                continue;
            }
            // Only match across different documents
            if entry_is_from_same_document(entry, source_document) {
                continue;
            }
            if entry.bank_id() == Some(bank_id) {
                candidates.push(i);
            }
        }
        if candidates.len() == 1 {
            return DedupResult::BankIdMatch {
                existing_index: candidates[0],
            };
        }
        if candidates.len() > 1 {
            return DedupResult::Ambiguous {
                candidate_indices: candidates,
            };
        }
    }

    // Step 3: Pending→finalized. Run before generic fuzzy matching so the
    // apply step updates the provisional source entry to the finalized bank row.
    let txn_amount = txn_primary_amount(txn);
    if txn.status() == EntryStatus::Cleared {
        let mut pending_candidates = Vec::new();
        for (i, entry) in existing.iter().enumerate() {
            if matched[i] || policy.prevents_match(&entry.id, &evidence_refs) {
                continue;
            }
            if entry.status != EntryStatus::Pending {
                continue;
            }
            if entry_is_from_same_document(entry, source_document) {
                continue;
            }
            if !dates_within_tolerance(&entry.date, &txn.tdate, config.date_tolerance_days) {
                continue;
            }
            if amounts_within_tolerance(
                &entry_primary_amount(entry),
                &txn_amount,
                config.pending_finalized_amount_abs,
                config.pending_finalized_amount_pct,
            ) && descriptions_similar(&entry.description, &txn.tdescription)
            {
                pending_candidates.push(i);
            }
        }
        if pending_candidates.len() == 1 {
            return DedupResult::PendingToFinalized {
                existing_index: pending_candidates[0],
            };
        }
    }

    // Step 4: Fuzzy match (across other documents)
    let mut fuzzy_candidates = Vec::new();

    for (i, entry) in existing.iter().enumerate() {
        if matched[i] || policy.prevents_match(&entry.id, &evidence_refs) {
            continue;
        }
        if entry_is_from_same_document(entry, source_document) {
            continue;
        }
        if !dates_within_tolerance(&entry.date, &txn.tdate, config.date_tolerance_days) {
            continue;
        }
        let entry_amount = entry_primary_amount(entry);
        if amounts_equal(&entry_amount, &txn_amount)
            && descriptions_similar(&entry.description, &txn.tdescription)
        {
            fuzzy_candidates.push(i);
        }
    }

    if fuzzy_candidates.len() == 1 {
        return DedupResult::FuzzyMatch {
            existing_index: fuzzy_candidates[0],
        };
    }

    // Step 5: Ambiguous (multiple fuzzy candidates)
    if fuzzy_candidates.len() > 1 {
        return DedupResult::Ambiguous {
            candidate_indices: fuzzy_candidates,
        };
    }

    // Step 6: New transaction
    DedupResult::New
}

fn entry_is_from_same_document(entry: &AccountEntry, source_document: &str) -> bool {
    entry
        .evidence
        .iter()
        .any(|ev| entry_is_from_same_document_evidence(ev, source_document))
}

fn entry_is_from_same_document_evidence(evidence_ref: &str, source_document: &str) -> bool {
    evidence_ref.starts_with(source_document)
        && evidence_ref
            .get(source_document.len()..)
            .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
            .unwrap_or(false)
}

fn document_coverage_for_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    document_name: &str,
) -> Result<Option<(chrono::NaiveDate, chrono::NaiveDate)>, Box<dyn std::error::Error + Send + Sync>>
{
    let document = crate::extract::list_documents_for_login_account(ledger_dir, login_name, label)?
        .into_iter()
        .find(|doc| doc.filename == document_name);
    let Some(info) = document.and_then(|doc| doc.info) else {
        return Ok(None);
    };
    let end_raw = info
        .date_range_end
        .as_deref()
        .unwrap_or(&info.coverage_end_date);
    let end = chrono::NaiveDate::parse_from_str(end_raw, "%Y-%m-%d")?;
    let start = match info.date_range_start.as_deref() {
        Some(start_raw) => chrono::NaiveDate::parse_from_str(start_raw, "%Y-%m-%d")?,
        None => end,
    };
    Ok(Some((start, end)))
}

fn entry_date_is_covered(
    entry_date: &str,
    coverage_start: chrono::NaiveDate,
    coverage_end: chrono::NaiveDate,
) -> bool {
    let Ok(date) = chrono::NaiveDate::parse_from_str(entry_date, "%Y-%m-%d") else {
        return false;
    };
    date >= coverage_start && date <= coverage_end
}

fn entry_matches_txn(entry: &AccountEntry, txn: &ExtractedTransaction) -> bool {
    entry.date == txn.tdate
        && amounts_equal(&entry_primary_amount(entry), &txn_primary_amount(txn))
        && descriptions_similar(&entry.description, &txn.tdescription)
}

struct MissingImportAnomaly<'a> {
    login_name: &'a str,
    label: &'a str,
    document_name: &'a str,
    entry: &'a AccountEntry,
    kind: crate::bookkeeping::ImportAnomalyKind,
    safe_to_retire: bool,
    safety_reasons: Vec<String>,
}

fn create_missing_import_anomaly(
    ledger_dir: &Path,
    anomaly: MissingImportAnomaly<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let amount = anomaly
        .entry
        .postings
        .first()
        .and_then(|posting| posting.amount.as_ref())
        .map(|amount| format!("{} {}", amount.quantity, amount.commodity));
    let gl_txn_id = anomaly.entry.posted.as_deref().map(gl_ref_txn_id);
    crate::bookkeeping::create_import_anomaly(
        ledger_dir,
        crate::bookkeeping::NewImportAnomalyInput {
            kind: anomaly.kind,
            login_name: anomaly.login_name.to_string(),
            label: anomaly.label.to_string(),
            source_entry_id: anomaly.entry.id.clone(),
            gl_txn_id,
            date: anomaly.entry.date.clone(),
            amount,
            description: anomaly.entry.description.clone(),
            evidence: anomaly.entry.evidence.clone(),
            coverage_document: anomaly.document_name.to_string(),
            safe_to_retire: anomaly.safe_to_retire,
            safety_reasons: anomaly.safety_reasons,
            notes: None,
        },
    )?;
    Ok(())
}

/// Login-account context needed to file import anomalies. Only the
/// `apply_dedup_actions_for_login_account` path supplies it; the legacy
/// `apply_dedup_actions` path passes `None`.
struct AnomalyContext<'a> {
    ledger_dir: &'a Path,
    login_name: &'a str,
    label: &'a str,
}

/// If a dedup update changed any leg's amount on an entry that already has
/// per-leg posted postings, record a PostedLegAmountDrift anomaly. The bank
/// amounts are still updated (bank data is truth), but the GL cannot be
/// auto-synced for posting-indexed sources, so the drift must be surfaced
/// rather than swallowed. Per-leg-posted entries are by definition multi-leg
/// splits, so every leg is compared — not just the primary.
fn maybe_record_posted_leg_amount_drift(
    ctx: Option<&AnomalyContext<'_>>,
    entry: &AccountEntry,
    action: &DedupAction,
    old_amounts: &[Option<f64>],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(ctx) = ctx else {
        return Ok(());
    };
    if entry.posted_postings.is_empty() {
        return Ok(());
    }
    let new_amounts = entry_posting_amounts(entry);
    let any_leg_changed = old_amounts.len() != new_amounts.len()
        || old_amounts
            .iter()
            .zip(new_amounts.iter())
            .any(|(old, new)| !amounts_equal(old, new));
    if !any_leg_changed {
        return Ok(());
    }
    crate::bookkeeping::create_import_anomaly(
        ctx.ledger_dir,
        crate::bookkeeping::NewImportAnomalyInput {
            kind: crate::bookkeeping::ImportAnomalyKind::PostedLegAmountDrift,
            login_name: ctx.login_name.to_string(),
            label: ctx.label.to_string(),
            source_entry_id: entry.id.clone(),
            gl_txn_id: entry
                .posted_postings
                .first()
                .map(|(_, gl_ref)| gl_ref_txn_id(gl_ref)),
            date: action.proposed.tdate.clone(),
            amount: txn_primary_simple_amount(&action.proposed)
                .map(|amount| format!("{} {}", amount.quantity, amount.commodity)),
            description: action.proposed.tdescription.clone(),
            evidence: action.proposed.evidence_refs(),
            coverage_document: action.source_document.clone(),
            safe_to_retire: false,
            safety_reasons: vec![
                "amount changed on a per-leg-posted entry; GL not auto-synced — unpost and re-post the split to update"
                    .to_string(),
            ],
            notes: None,
        },
    )?;
    Ok(())
}

fn gl_ref_txn_id(posted: &str) -> String {
    posted
        .strip_prefix("general.journal:")
        .unwrap_or(posted)
        .to_string()
}

fn has_content_changed(entry: &AccountEntry, txn: &ExtractedTransaction) -> bool {
    if entry.description != txn.tdescription {
        return true;
    }
    if entry.status != txn.status() {
        return true;
    }
    let proposed_amount = txn_primary_amount(txn);
    if proposed_amount.is_some() && !amounts_equal(&entry_primary_amount(entry), &proposed_amount) {
        return true;
    }
    false
}

fn is_more_finalized(new_status: &EntryStatus, old_status: &EntryStatus) -> bool {
    matches!(
        (old_status, new_status),
        (
            EntryStatus::Unmarked,
            EntryStatus::Pending | EntryStatus::Cleared
        ) | (EntryStatus::Pending, EntryStatus::Cleared)
    )
}

fn update_entry_from_proposed(entry: &mut AccountEntry, txn: &ExtractedTransaction) {
    entry.description = txn.tdescription.clone();
    entry.status = txn.status();
    if !txn.tcomment.is_empty() {
        entry.comment = txn.tcomment.clone();
    }
    update_entry_amount_from_proposed(entry, txn);
}

fn update_entry_amount_from_proposed(entry: &mut AccountEntry, txn: &ExtractedTransaction) {
    if let Some(ref postings) = txn.tpostings {
        for (entry_posting, proposed_posting) in entry.postings.iter_mut().zip(postings.iter()) {
            entry_posting.amount = proposed_posting
                .pamount
                .as_ref()
                .and_then(|amounts| amounts.first())
                .map(|amount| SimpleAmount {
                    commodity: amount.acommodity.clone(),
                    quantity: amount.aquantity.clone(),
                });
        }
        return;
    }

    let Some(primary_amount) = txn_primary_simple_amount(txn) else {
        return;
    };

    if let Some(first) = entry.postings.first_mut() {
        first.amount = Some(primary_amount.clone());
    }
    if entry.postings.len() == 2 && crate::staging::is_staging_account(&entry.postings[1].account) {
        let negated = SimpleAmount {
            commodity: primary_amount.commodity,
            quantity: negate_quantity(&primary_amount.quantity),
        };
        entry.postings[1].amount = Some(negated);
    }
}

fn txn_primary_simple_amount(txn: &ExtractedTransaction) -> Option<SimpleAmount> {
    if let Some(ref postings) = txn.tpostings {
        if let Some(first) = postings.first() {
            if let Some(ref amounts) = first.pamount {
                if let Some(first_amount) = amounts.first() {
                    return Some(SimpleAmount {
                        commodity: first_amount.acommodity.clone(),
                        quantity: first_amount.aquantity.clone(),
                    });
                }
            }
        }
    }

    for (key, value) in &txn.ttags {
        if key == "amount" {
            let mut parts = value.split_whitespace();
            let quantity = parts.next().unwrap_or(value).to_string();
            let commodity = parts.next().unwrap_or("").to_string();
            return Some(SimpleAmount {
                commodity,
                quantity,
            });
        }
    }
    None
}

fn negate_quantity(quantity: &str) -> String {
    if let Some(stripped) = quantity.strip_prefix('-') {
        stripped.to_string()
    } else if let Some(stripped) = quantity.strip_prefix('+') {
        format!("-{stripped}")
    } else {
        format!("-{quantity}")
    }
}

pub(crate) fn dates_within_tolerance(date_a: &str, date_b: &str, tolerance_days: i64) -> bool {
    let Ok(a) = chrono::NaiveDate::parse_from_str(date_a, "%Y-%m-%d") else {
        return false;
    };
    let Ok(b) = chrono::NaiveDate::parse_from_str(date_b, "%Y-%m-%d") else {
        return false;
    };
    let diff = (a - b).num_days().abs();
    diff <= tolerance_days
}

fn txn_primary_amount(txn: &ExtractedTransaction) -> Option<f64> {
    // Try explicit postings first
    if let Some(ref postings) = txn.tpostings {
        if let Some(first) = postings.first() {
            if let Some(ref amounts) = first.pamount {
                if let Some(first_amount) = amounts.first() {
                    return first_amount.aquantity.parse().ok();
                }
            }
        }
    }
    // Try amount tag
    for (key, value) in &txn.ttags {
        if key == "amount" {
            let qty = value.split_whitespace().next().unwrap_or(value);
            return qty.parse().ok();
        }
    }
    None
}

fn entry_primary_amount(entry: &AccountEntry) -> Option<f64> {
    entry
        .postings
        .first()
        .and_then(|p| p.amount.as_ref())
        .and_then(|a| a.quantity.parse().ok())
}

/// Every leg's parsed amount, in posting order (None for amountless legs).
fn entry_posting_amounts(entry: &AccountEntry) -> Vec<Option<f64>> {
    entry
        .postings
        .iter()
        .map(|p| p.amount.as_ref().and_then(|a| a.quantity.parse().ok()))
        .collect()
}

fn amounts_equal(a: &Option<f64>, b: &Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 0.005,
        (None, None) => true,
        _ => false,
    }
}

fn amounts_within_tolerance(
    a: &Option<f64>,
    b: &Option<f64>,
    abs_tolerance: f64,
    pct_tolerance: f64,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            let max_abs = a.abs().max(b.abs());
            diff <= abs_tolerance || (max_abs > 0.0 && diff / max_abs <= pct_tolerance)
        }
        (None, None) => true,
        _ => false,
    }
}

pub(crate) fn descriptions_similar(a: &str, b: &str) -> bool {
    let na = normalize_description(a);
    let nb = normalize_description(b);
    if na == nb {
        return true;
    }
    // Check if one contains the other (for truncation cases)
    if na.contains(&nb) || nb.contains(&na) {
        return true;
    }
    // Simple Jaccard-like word overlap
    let words_a: std::collections::HashSet<&str> = na.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = nb.split_whitespace().collect();
    if words_a.is_empty() || words_b.is_empty() {
        return false;
    }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    if union == 0 {
        return false;
    }
    let similarity = intersection as f64 / union as f64;
    similarity >= 0.5
}

fn normalize_description(desc: &str) -> String {
    desc.to_ascii_uppercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::account_journal::{EntryPosting, SimpleAmount};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_entry(
        id: &str,
        date: &str,
        desc: &str,
        status: EntryStatus,
        amount: &str,
        evidence: &[&str],
    ) -> AccountEntry {
        AccountEntry {
            id: id.to_string(),
            date: date.to_string(),
            status,
            description: desc.to_string(),
            comment: String::new(),
            evidence: evidence.iter().map(|e| e.to_string()).collect(),
            postings: vec![
                EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: amount.to_string(),
                    }),
                },
                EntryPosting {
                    account: "Equity:Staging".to_string(),
                    amount: None,
                },
            ],
            tags: vec![],
            extracted_by: None,
            posted: None,
            posted_postings: Vec::new(),
        }
    }

    fn make_txn(date: &str, desc: &str, status: &str, evidence: &str) -> ExtractedTransaction {
        ExtractedTransaction {
            tdate: date.to_string(),
            tstatus: status.to_string(),
            tdescription: desc.to_string(),
            tcomment: String::new(),
            ttags: vec![("evidence".to_string(), evidence.to_string())],
            tpostings: None,
        }
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-dedup-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn same_evidence_matches() {
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Test",
            EntryStatus::Cleared,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];
        let proposed = vec![make_txn("2024-01-01", "Test", "Cleared", "doc-a.csv:1:1")];

        let actions = run_dedup(&existing, &proposed, "doc-a.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0].result,
            DedupResult::SameEvidence {
                existing_index: 0,
                updated: false
            }
        ));
    }

    #[test]
    fn new_transaction_when_no_match() {
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Other",
            EntryStatus::Cleared,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];
        let proposed = vec![make_txn(
            "2024-02-15",
            "New txn",
            "Cleared",
            "doc-b.csv:1:1",
        )];

        let actions = run_dedup(&existing, &proposed, "doc-b.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0].result, DedupResult::New));
    }

    #[test]
    fn policy_same_source_forces_match_to_evidence_ref() {
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Different",
            EntryStatus::Cleared,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];
        let proposed = vec![make_txn("2024-02-01", "Other", "Cleared", "doc-b.csv:9:1")];
        let mut policy = DedupPolicy::default();
        policy.force_match("e1", "doc-b.csv:9:1");

        let actions = run_dedup_with_policy(
            &existing,
            &proposed,
            "doc-b.csv",
            &DedupConfig::default(),
            &policy,
        );

        assert!(matches!(
            actions[0].result,
            DedupResult::ResolutionMatch { existing_index: 0 }
        ));
    }

    #[test]
    fn evidence_row_resolution_forces_login_dedup_match() {
        let root = temp_dir("evidence-row-resolution");
        crate::automation::create_resolution(
            &root,
            crate::automation::NewResolutionInput {
                kind: crate::automation::ResolutionKind::SameSource,
                subject_refs: vec![
                    TypedRef {
                        kind: TypedRefKind::LoginEntry,
                        id: None,
                        locator: Some("logins/bank/accounts/checking".to_string()),
                        entry_id: Some("e1".to_string()),
                        login_name: Some("bank".to_string()),
                        label: Some("checking".to_string()),
                        filename: None,
                    },
                    TypedRef {
                        kind: TypedRefKind::EvidenceRow,
                        id: None,
                        locator: Some("doc-b.csv:9:1".to_string()),
                        entry_id: None,
                        login_name: None,
                        label: None,
                        filename: None,
                    },
                ],
                parts: Vec::new(),
                notes: None,
            },
        )
        .expect("create resolution");
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Different",
            EntryStatus::Cleared,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];
        let proposed = vec![make_txn("2024-02-01", "Other", "Cleared", "doc-b.csv:9:1")];

        let actions = run_dedup_for_login_account(
            &root,
            "bank",
            "checking",
            &existing,
            &proposed,
            "doc-b.csv",
            &DedupConfig::default(),
        );

        assert!(matches!(
            actions[0].result,
            DedupResult::ResolutionMatch { existing_index: 0 }
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn policy_not_same_source_blocks_heuristic_match() {
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Test",
            EntryStatus::Cleared,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];
        let mut proposed = make_txn("2024-01-01", "Test", "Cleared", "doc-b.csv:9:1");
        proposed
            .ttags
            .push(("amount".to_string(), "-10.00 USD".to_string()));
        let mut policy = DedupPolicy::default();
        policy.prevent_match("e1", "doc-b.csv:9:1");

        let actions = run_dedup_with_policy(
            &existing,
            &[proposed],
            "doc-b.csv",
            &DedupConfig::default(),
            &policy,
        );

        assert!(matches!(actions[0].result, DedupResult::New));
    }

    #[test]
    fn cross_document_fuzzy_match() {
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "SHELL OIL 12345",
            EntryStatus::Cleared,
            "-21.32",
            &["doc-a.csv:1:1"],
        )];

        let mut txn = make_txn("2024-01-01", "SHELL OIL 12345", "Cleared", "doc-b.csv:1:1");
        txn.ttags
            .push(("amount".to_string(), "-21.32 USD".to_string()));

        let actions = run_dedup(&existing, &[txn], "doc-b.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0].result,
            DedupResult::FuzzyMatch { existing_index: 0 }
        ));
    }

    #[test]
    fn no_within_document_merging() {
        // Two identical transactions from the same document should both be New
        let existing = vec![];
        let proposed = vec![
            make_txn("2024-01-01", "Amazon", "Cleared", "doc-a.csv:1:1"),
            make_txn("2024-01-01", "Amazon", "Cleared", "doc-a.csv:2:1"),
        ];

        let actions = run_dedup(&existing, &proposed, "doc-a.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0].result, DedupResult::New));
        assert!(matches!(actions[1].result, DedupResult::New));
    }

    #[test]
    fn descriptions_similar_basic() {
        assert!(descriptions_similar("SHELL OIL 12345", "SHELL OIL 12345"));
        assert!(descriptions_similar("shell oil 12345", "SHELL OIL 12345"));
        assert!(descriptions_similar("SHELL OIL", "SHELL OIL 12345"));
        assert!(!descriptions_similar("SHELL OIL", "WALMART"));
    }

    #[test]
    fn descriptions_similar_treats_punctuation_as_separators() {
        assert!(descriptions_similar(
            "LEMONADE-METROMILE INS, +18447338666, NY",
            "LEMONADE-METROMILE INS   WWW.LEMONADE.NY",
        ));
    }

    #[test]
    fn cleared_card_row_finalizes_prior_pending_card_row() {
        let existing = vec![make_entry(
            "e1",
            "2026-03-23",
            "LEMONADE-METROMILE INS, +18447338666, NY",
            EntryStatus::Pending,
            "-57.97",
            &["pending.csv:2:1"],
        )];

        let mut txn = make_txn(
            "2026-03-23",
            "LEMONADE-METROMILE INS   WWW.LEMONADE.NY",
            "Cleared",
            "cleared.csv:2:1",
        );
        txn.ttags
            .push(("amount".to_string(), "-57.97 USD".to_string()));

        let actions = run_dedup(&existing, &[txn], "cleared.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0].result,
            DedupResult::PendingToFinalized { existing_index: 0 }
        ));
    }

    #[test]
    fn dates_within_tolerance_basic() {
        assert!(dates_within_tolerance("2024-01-01", "2024-01-01", 1));
        assert!(dates_within_tolerance("2024-01-01", "2024-01-02", 1));
        assert!(!dates_within_tolerance("2024-01-01", "2024-01-03", 1));
    }

    #[test]
    fn same_evidence_amount_change_updates_existing_entry() {
        let root = temp_dir("same-evidence-amount-change");
        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Coffee",
            EntryStatus::Pending,
            "-10.00",
            &["doc-a.csv:1:1"],
        )];

        let mut proposed = make_txn("2024-01-01", "Coffee", "Cleared", "doc-a.csv:1:1");
        proposed
            .ttags
            .push(("amount".to_string(), "-11.50 USD".to_string()));

        let actions = run_dedup(
            &existing,
            &[proposed.clone()],
            "doc-a.csv",
            &DedupConfig::default(),
        );
        let updated = apply_dedup_actions(
            &root,
            "test-acct",
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("test:latest"),
        )
        .expect("apply_dedup_actions");

        let updated_amount = updated[0]
            .postings
            .first()
            .and_then(|p| p.amount.as_ref())
            .map(|a| a.quantity.clone())
            .expect("first posting amount");
        assert_eq!(updated_amount, "-11.50");
        assert_eq!(updated[0].status, EntryStatus::Cleared);

        let _ = fs::remove_dir_all(&root);
    }

    fn write_login_document_info(
        root: &Path,
        login: &str,
        label: &str,
        filename: &str,
        start: &str,
        end: &str,
    ) {
        let docs_dir = root
            .join("logins")
            .join(login)
            .join("accounts")
            .join(label)
            .join("documents");
        fs::create_dir_all(&docs_dir).expect("create docs dir");
        fs::write(docs_dir.join(filename), b"Date,Description,Amount\n").expect("write doc");
        let info = crate::scrape::DocumentInfo {
            mime_type: "text/csv".to_string(),
            original_url: None,
            scraped_at: "2026-04-01T00:00:00Z".to_string(),
            extension_name: "providentcu".to_string(),
            login_name: login.to_string(),
            label: label.to_string(),
            scrape_session_id: "sess-1".to_string(),
            coverage_end_date: end.to_string(),
            date_range_start: Some(start.to_string()),
            date_range_end: Some(end.to_string()),
            metadata: std::collections::BTreeMap::new(),
            extraction_error: None,
            extraction_attempts: 0,
        };
        fs::write(
            docs_dir.join(format!("{filename}-info.json")),
            serde_json::to_string_pretty(&info).expect("serialize sidecar"),
        )
        .expect("write sidecar");
    }

    #[test]
    fn coverage_lifecycle_retires_missing_unposted_pending_entry() {
        let root = temp_dir("coverage-retires-pending");
        write_login_document_info(
            &root,
            "provident",
            "card",
            "activity.csv",
            "2026-03-01",
            "2026-03-31",
        );
        let existing = vec![make_entry(
            "pending-1",
            "2026-03-23",
            "PENDING HOLD",
            EntryStatus::Pending,
            "-57.97",
            &["old.csv:2:1"],
        )];

        let updated = apply_coverage_lifecycle_for_login_account(
            &root,
            "provident",
            "card",
            "activity.csv",
            &[],
            existing,
        )
        .expect("apply lifecycle");

        assert!(updated.is_empty());
        let ops =
            operations::read_login_account_operations(&root, "provident", "card").expect("ops");
        assert!(matches!(
            ops.as_slice(),
            [operations::AccountOperation::EntryRetired { entry_id, .. }]
                if entry_id == "pending-1"
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn coverage_lifecycle_flags_missing_cleared_entry() {
        let root = temp_dir("coverage-flags-cleared");
        write_login_document_info(
            &root,
            "provident",
            "card",
            "activity.csv",
            "2026-03-01",
            "2026-03-31",
        );
        let existing = vec![make_entry(
            "cleared-1",
            "2026-03-23",
            "CLEARED PURCHASE",
            EntryStatus::Cleared,
            "-57.97",
            &["old.csv:2:1"],
        )];

        let updated = apply_coverage_lifecycle_for_login_account(
            &root,
            "provident",
            "card",
            "activity.csv",
            &[],
            existing,
        )
        .expect("apply lifecycle");

        assert_eq!(updated.len(), 1);
        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(anomalies.len(), 1);
        assert!(matches!(
            anomalies[0].kind,
            crate::bookkeeping::ImportAnomalyKind::FinalizedMissingFromCoveredExport
        ));
        assert_eq!(anomalies[0].source_entry_id, "cleared-1");

        let _ = fs::remove_dir_all(&root);
    }

    fn reimport_txn(date: &str, desc: &str, evidence: &str, amount: &str) -> ExtractedTransaction {
        ExtractedTransaction {
            tdate: date.to_string(),
            tstatus: "Cleared".to_string(),
            tdescription: desc.to_string(),
            tcomment: String::new(),
            ttags: vec![
                ("evidence".to_string(), evidence.to_string()),
                ("amount".to_string(), amount.to_string()),
            ],
            tpostings: None,
        }
    }

    #[test]
    fn dedup_records_posted_leg_amount_drift_anomaly() {
        let root = temp_dir("posted-leg-drift");
        let mut existing_entry = make_entry(
            "e1",
            "2024-03-01",
            "Grocery",
            EntryStatus::Cleared,
            "-25.00",
            &["activity.csv:2:1"],
        );
        existing_entry.posted_postings = vec![(0, "general.journal:gl-x".to_string())];
        let existing = vec![existing_entry];

        // Re-import the same row (same evidence) with a changed amount.
        let proposed = vec![reimport_txn(
            "2024-03-01",
            "Grocery",
            "activity.csv:2:1",
            "-30.00 USD",
        )];
        let actions = run_dedup(
            &existing,
            &proposed,
            "activity.csv",
            &DedupConfig::default(),
        );
        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions");

        // Bank data is truth: the amount is updated.
        assert_eq!(
            updated[0]
                .postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| a.quantity.as_str()),
            Some("-30.00"),
        );
        // ...but the drift on a per-leg-posted entry is surfaced as an anomaly.
        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(anomalies.len(), 1);
        assert!(matches!(
            anomalies[0].kind,
            crate::bookkeeping::ImportAnomalyKind::PostedLegAmountDrift
        ));
        assert_eq!(anomalies[0].source_entry_id, "e1");

        // A further drift from the same document dedups (kind, login, label,
        // source_entry_id, coverage_document) rather than piling up.
        let proposed2 = vec![reimport_txn(
            "2024-03-01",
            "Grocery",
            "activity.csv:2:1",
            "-35.00 USD",
        )];
        let actions2 = run_dedup(
            &updated,
            &proposed2,
            "activity.csv",
            &DedupConfig::default(),
        );
        let updated2 = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            updated,
            &actions2,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions again");
        assert_eq!(
            updated2[0]
                .postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| a.quantity.as_str()),
            Some("-35.00"),
        );
        let anomalies2 = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(anomalies2.len(), 1, "drift anomaly must not duplicate");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dedup_records_drift_when_only_non_primary_leg_changes() {
        // Per-leg-posted entries are by definition multi-leg splits, so drift
        // detection must compare every leg, not just the primary. The
        // pending→cleared transition updates all legs from tpostings.
        let root = temp_dir("posted-leg-drift-nonprimary");
        let mut existing_entry = make_entry(
            "e1",
            "2024-03-01",
            "Paycheck",
            EntryStatus::Pending,
            "-50.00",
            &["activity.csv:2:1"],
        );
        existing_entry.postings[1].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "30.00".to_string(),
        });
        existing_entry.posted_postings = vec![(1, "general.journal:gl-x".to_string())];
        let existing = vec![existing_entry];

        // Same evidence, finalized status, primary leg unchanged, leg 2 drifted.
        let mut proposed_txn =
            reimport_txn("2024-03-01", "Paycheck", "activity.csv:2:1", "-50.00 USD");
        proposed_txn.tpostings = Some(vec![
            crate::extract::ExtractedPosting {
                paccount: "Assets:Checking".to_string(),
                pamount: Some(vec![crate::extract::ExtractedAmount {
                    acommodity: "USD".to_string(),
                    aquantity: "-50.00".to_string(),
                }]),
            },
            crate::extract::ExtractedPosting {
                paccount: "Equity:Staging".to_string(),
                pamount: Some(vec![crate::extract::ExtractedAmount {
                    acommodity: "USD".to_string(),
                    aquantity: "35.00".to_string(),
                }]),
            },
        ]);
        let actions = run_dedup(
            &existing,
            &[proposed_txn],
            "activity.csv",
            &DedupConfig::default(),
        );
        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions");

        assert_eq!(
            updated[0].postings[1]
                .amount
                .as_ref()
                .map(|a| a.quantity.as_str()),
            Some("35.00"),
            "bank data is truth; leg 2 must be updated"
        );
        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(
            anomalies.len(),
            1,
            "a non-primary-leg drift on a per-leg-posted entry must be surfaced"
        );
        assert!(matches!(
            anomalies[0].kind,
            crate::bookkeeping::ImportAnomalyKind::PostedLegAmountDrift
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dedup_records_no_drift_anomaly_for_unposted_entry() {
        // Amount changes on entries without per-leg posts stay on the existing
        // whole-entry drift/sync path; no anomaly is filed.
        let root = temp_dir("unposted-no-drift-anomaly");
        let existing = vec![make_entry(
            "e1",
            "2024-03-01",
            "Grocery",
            EntryStatus::Cleared,
            "-25.00",
            &["activity.csv:2:1"],
        )];
        let proposed = vec![reimport_txn(
            "2024-03-01",
            "Grocery",
            "activity.csv:2:1",
            "-30.00 USD",
        )];
        let actions = run_dedup(
            &existing,
            &proposed,
            "activity.csv",
            &DedupConfig::default(),
        );
        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions");

        assert_eq!(
            updated[0]
                .postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| a.quantity.as_str()),
            Some("-30.00"),
        );
        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert!(
            anomalies.is_empty(),
            "no drift anomaly for entries without per-leg posts"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn coverage_lifecycle_records_anomaly_when_info_missing() {
        let root = temp_dir("coverage-info-missing");
        // No document info sidecar exists, so disappeared-entry detection
        // self-disables; that skip must be surfaced as an anomaly.
        let entries = vec![make_entry(
            "e1",
            "2024-01-05",
            "Grocery",
            EntryStatus::Cleared,
            "-10.00",
            &["statement.pdf:1:1"],
        )];
        let out = apply_coverage_lifecycle_for_login_account(
            &root,
            "chase",
            "checking",
            "statement.pdf",
            &[],
            entries,
        )
        .expect("apply lifecycle");
        assert_eq!(out.len(), 1, "entries must be unchanged");

        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(anomalies.len(), 1);
        assert!(matches!(
            anomalies[0].kind,
            crate::bookkeeping::ImportAnomalyKind::CoverageInfoMissing
        ));
        assert_eq!(anomalies[0].coverage_document, "statement.pdf");

        // A second import of the same document dedups rather than piling up.
        let out2 = apply_coverage_lifecycle_for_login_account(
            &root,
            "chase",
            "checking",
            "statement.pdf",
            &[],
            out,
        )
        .expect("apply lifecycle again");
        assert_eq!(out2.len(), 1);
        let anomalies2 = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");
        assert_eq!(
            anomalies2.len(),
            1,
            "coverage-info anomaly must not duplicate"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_dedup_actions_for_login_account_links_attachment_docs() {
        let root = temp_dir("attachment-link");
        let docs_dir = root
            .join("logins")
            .join("chase")
            .join("accounts")
            .join("checking")
            .join("documents");
        fs::create_dir_all(&docs_dir).expect("create docs dir");

        let attachment_file = "2026-02-01-check-123-front.png";
        let sidecar_path = docs_dir.join(format!("{attachment_file}-info.json"));
        let info = crate::scrape::DocumentInfo {
            mime_type: "image/png".to_string(),
            original_url: None,
            scraped_at: "2026-02-01T00:00:00Z".to_string(),
            extension_name: "providentcu".to_string(),
            login_name: "chase".to_string(),
            label: "checking".to_string(),
            scrape_session_id: "sess-1".to_string(),
            coverage_end_date: "2026-02-01".to_string(),
            date_range_start: None,
            date_range_end: None,
            metadata: std::collections::BTreeMap::from([(
                "attachmentKey".to_string(),
                serde_json::Value::String("check:123|2026-02-01|-25.00".to_string()),
            )]),
            extraction_error: None,
            extraction_attempts: 0,
        };
        fs::write(docs_dir.join(attachment_file), b"img").expect("write attachment doc");
        fs::write(
            sidecar_path,
            serde_json::to_string_pretty(&info).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let mut proposed = make_txn("2026-02-01", "CHECK 123", "Cleared", "activity.csv:2:1");
        proposed.ttags.push((
            "attachmentKey".to_string(),
            "check:123|2026-02-01|-25.00".to_string(),
        ));
        let actions = run_dedup(&[], &[proposed], "activity.csv", &DedupConfig::default());
        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            vec![],
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions");

        assert_eq!(updated.len(), 1);
        assert!(
            updated[0]
                .evidence
                .iter()
                .any(|ev| ev == "2026-02-01-check-123-front.png#attachment"),
            "attachment evidence should be linked from attachmentKey"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_dedup_actions_for_login_account_links_check_attachment_with_sign_mismatch() {
        let root = temp_dir("attachment-link-sign-mismatch");
        let docs_dir = root
            .join("logins")
            .join("chase")
            .join("accounts")
            .join("checking")
            .join("documents");
        fs::create_dir_all(&docs_dir).expect("create docs dir");

        let attachment_file = "2026-02-01-check-123-single.png";
        let sidecar_path = docs_dir.join(format!("{attachment_file}-info.json"));
        let info = crate::scrape::DocumentInfo {
            mime_type: "image/png".to_string(),
            original_url: None,
            scraped_at: "2026-02-01T00:00:00Z".to_string(),
            extension_name: "providentcu".to_string(),
            login_name: "chase".to_string(),
            label: "checking".to_string(),
            scrape_session_id: "sess-1".to_string(),
            coverage_end_date: "2026-02-01".to_string(),
            date_range_start: None,
            date_range_end: None,
            metadata: std::collections::BTreeMap::from([(
                "attachmentKey".to_string(),
                serde_json::Value::String("check:123|2026-02-01|25.00".to_string()),
            )]),
            extraction_error: None,
            extraction_attempts: 0,
        };
        fs::write(docs_dir.join(attachment_file), b"img").expect("write attachment doc");
        fs::write(
            sidecar_path,
            serde_json::to_string_pretty(&info).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let mut proposed = make_txn("2026-02-01", "CHECK 123", "Cleared", "activity.csv:2:1");
        proposed.ttags.push((
            "attachmentKey".to_string(),
            "check:123|2026-02-01|-25.00".to_string(),
        ));
        let actions = run_dedup(&[], &[proposed], "activity.csv", &DedupConfig::default());
        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("chase", "checking"),
            vec![],
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("providentcu:latest"),
        )
        .expect("apply login dedup actions");

        assert_eq!(updated.len(), 1);
        assert!(
            updated[0]
                .evidence
                .iter()
                .any(|ev| ev == "2026-02-01-check-123-single.png#attachment"),
            "attachment evidence should be linked when only sign differs in check attachmentKey"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ambiguous_bank_id_match_requires_review() {
        let mut existing = vec![
            make_entry(
                "e1",
                "2024-01-01",
                "Transfer A",
                EntryStatus::Cleared,
                "-10.00",
                &["doc-a.csv:1:1"],
            ),
            make_entry(
                "e2",
                "2024-01-01",
                "Transfer B",
                EntryStatus::Cleared,
                "-10.00",
                &["doc-c.csv:1:1"],
            ),
        ];
        existing[0]
            .tags
            .push(("bankId".to_string(), "FIT-123".to_string()));
        existing[1]
            .tags
            .push(("bankId".to_string(), "FIT-123".to_string()));

        let mut proposed = make_txn("2024-01-01", "Transfer", "Cleared", "doc-b.csv:1:1");
        proposed
            .ttags
            .push(("bankId".to_string(), "FIT-123".to_string()));

        let actions = run_dedup(&existing, &[proposed], "doc-b.csv", &DedupConfig::default());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0].result, DedupResult::Ambiguous { .. }));
    }

    #[test]
    fn ambiguous_login_dedup_creates_review_anomalies() {
        let root = temp_dir("ambiguous-dedup-anomaly");
        let mut existing = vec![
            make_entry(
                "e1",
                "2024-01-01",
                "Transfer A",
                EntryStatus::Cleared,
                "-10.00",
                &["doc-a.csv:1:1"],
            ),
            make_entry(
                "e2",
                "2024-01-01",
                "Transfer B",
                EntryStatus::Cleared,
                "-10.00",
                &["doc-c.csv:1:1"],
            ),
        ];
        existing[0]
            .tags
            .push(("bankId".to_string(), "FIT-123".to_string()));
        existing[1]
            .tags
            .push(("bankId".to_string(), "FIT-123".to_string()));
        let mut proposed = make_txn("2024-01-01", "Transfer", "Cleared", "doc-b.csv:1:1");
        proposed
            .ttags
            .push(("bankId".to_string(), "FIT-123".to_string()));
        let actions = run_dedup(&existing, &[proposed], "doc-b.csv", &DedupConfig::default());

        let updated = apply_dedup_actions_for_login_account(
            &root,
            ("bank", "checking"),
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("test:latest"),
        )
        .expect("apply login dedup actions");
        let anomalies = crate::bookkeeping::list_import_anomalies(&root).expect("anomalies");

        assert_eq!(updated.len(), 2);
        assert_eq!(anomalies.len(), 2);
        assert!(anomalies.iter().all(|anomaly| {
            anomaly.kind == crate::bookkeeping::ImportAnomalyKind::DuplicateImportRepairSkipped
                && anomaly.coverage_document == "doc-b.csv"
                && anomaly.evidence == vec!["doc-b.csv:1:1".to_string()]
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_dedup_actions_merges_tcomment_on_fuzzy_match() {
        let root = temp_dir("dedup-tcomment-merge");

        let existing = vec![make_entry(
            "e1",
            "2024-01-01",
            "Zelle Alice",
            EntryStatus::Cleared,
            "-50.00",
            &["doc-a.csv:1:1"],
        )];

        let mut txn = make_txn("2024-01-01", "Zelle Alice", "Cleared", "doc-b.json:1001");
        txn.ttags
            .push(("amount".to_string(), "-50.00 USD".to_string()));
        txn.tcomment = "Pizza".to_string();

        let actions = run_dedup(&existing, &[txn], "doc-b.json", &DedupConfig::default());
        let updated = apply_dedup_actions(
            &root,
            "test-acct",
            existing,
            &actions,
            "Assets:Checking",
            "Equity:Staging:Checking",
            Some("test:latest"),
        )
        .expect("apply_dedup_actions");

        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].comment, "Pizza");

        let _ = fs::remove_dir_all(&root);
    }
}
