use crate::account_journal::{AccountEntry, EntryStatus, SimpleAmount};
use crate::extract::ExtractedTransaction;
use crate::operations;

use std::path::Path;

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
    /// Maximum number of days for pending→finalized transition.
    pub pending_finalized_days: i64,
    /// Amount tolerance for pending→finalized (absolute).
    pub pending_finalized_amount_abs: f64,
    /// Amount tolerance for pending→finalized (relative, e.g. 0.20 = 20%).
    pub pending_finalized_amount_pct: f64,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            date_tolerance_days: 1,
            pending_finalized_days: 7,
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
    let mut actions = Vec::new();
    // Track which existing entries have been matched (one-time consumption).
    let mut matched_existing: Vec<bool> = vec![false; existing.len()];

    for txn in proposed {
        let result = match_proposed(existing, txn, source_document, config, &matched_existing);
        match &result {
            DedupResult::SameEvidence { existing_index, .. }
            | DedupResult::BankIdMatch { existing_index }
            | DedupResult::FuzzyMatch { existing_index }
            | DedupResult::PendingToFinalized { existing_index } => {
                matched_existing[*existing_index] = true;
            }
            DedupResult::New | DedupResult::Ambiguous { .. } => {}
        }
        actions.push(DedupAction {
            proposed: txn.clone(),
            result,
        });
    }

    actions
}

/// A dedup action: the proposed transaction paired with its match result.
pub struct DedupAction {
    pub proposed: ExtractedTransaction,
    pub result: DedupResult,
}

/// Apply dedup actions to update the account journal entries.
///
/// Returns the updated list of entries.
pub fn apply_dedup_actions(
    ledger_dir: &Path,
    account_name: &str,
    mut entries: Vec<AccountEntry>,
    actions: &[DedupAction],
    default_account: &str,
    unreconciled_equity: &str,
    extracted_by: Option<&str>,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    for action in actions {
        match &action.result {
            DedupResult::SameEvidence {
                existing_index,
                updated,
            } => {
                if *updated {
                    // Update existing entry with new data
                    update_entry_from_proposed(&mut entries[*existing_index], &action.proposed);
                }
                // Even if not updated, ensure evidence is added
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
            }
            DedupResult::BankIdMatch { existing_index }
            | DedupResult::FuzzyMatch { existing_index } => {
                // Add evidence link from the new document
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
                // Update status if more finalized
                if is_more_finalized(&action.proposed.status(), &entries[*existing_index].status) {
                    entries[*existing_index].status = action.proposed.status();
                }
                if !amounts_equal(
                    &entry_primary_amount(&entries[*existing_index]),
                    &txn_primary_amount(&action.proposed),
                ) {
                    update_entry_amount_from_proposed(
                        &mut entries[*existing_index],
                        &action.proposed,
                    );
                }
            }
            DedupResult::PendingToFinalized { existing_index } => {
                // Update to finalized
                entries[*existing_index].status = EntryStatus::Cleared;
                // Update amount if postings have amounts
                update_entry_from_proposed(&mut entries[*existing_index], &action.proposed);
                for ev in action.proposed.evidence_refs() {
                    entries[*existing_index].add_evidence(ev);
                }
            }
            DedupResult::New => {
                let mut entry = action
                    .proposed
                    .to_account_entry(default_account, unreconciled_equity);
                if let Some(eb) = extracted_by {
                    entry.extracted_by = Some(eb.to_string());
                }

                // Log entry-created operation
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
                operations::append_account_operation(ledger_dir, account_name, &op)?;

                entries.push(entry);
            }
            DedupResult::Ambiguous { .. } => {
                // Skip ambiguous: needs human review
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
) -> DedupResult {
    let evidence_refs = txn.evidence_refs();

    // Step 1: Same-evidence match
    for (i, entry) in existing.iter().enumerate() {
        if matched[i] {
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
            if matched[i] {
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
    }

    // Step 3: Fuzzy match (across other documents)
    let mut fuzzy_candidates = Vec::new();
    let txn_amount = txn_primary_amount(txn);

    for (i, entry) in existing.iter().enumerate() {
        if matched[i] {
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

    // Step 4: Pending→finalized
    if txn.status() == EntryStatus::Cleared {
        let mut pending_candidates = Vec::new();
        for (i, entry) in existing.iter().enumerate() {
            if matched[i] {
                continue;
            }
            if entry.status != EntryStatus::Pending {
                continue;
            }
            if entry_is_from_same_document(entry, source_document) {
                continue;
            }
            if !dates_within_tolerance(&entry.date, &txn.tdate, config.pending_finalized_days) {
                continue;
            }
            if amounts_within_tolerance(
                &entry_primary_amount(entry),
                &txn_amount,
                config.pending_finalized_amount_abs,
                config.pending_finalized_amount_pct,
            ) {
                pending_candidates.push(i);
            }
        }
        if pending_candidates.len() == 1 {
            return DedupResult::PendingToFinalized {
                existing_index: pending_candidates[0],
            };
        }
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
    entry.evidence.iter().any(|ev| {
        ev.starts_with(source_document)
            && ev
                .get(source_document.len()..)
                .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
                .unwrap_or(false)
    })
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
    if entry.postings.len() == 2 && entry.postings[1].account.starts_with("Equity:Unreconciled") {
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

fn dates_within_tolerance(date_a: &str, date_b: &str, tolerance_days: i64) -> bool {
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

fn descriptions_similar(a: &str, b: &str) -> bool {
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
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
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
                    account: "Equity:Unreconciled".to_string(),
                    amount: None,
                },
            ],
            tags: vec![],
            extracted_by: None,
            reconciled: None,
            reconciled_postings: Vec::new(),
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
    fn dates_within_tolerance_basic() {
        assert!(dates_within_tolerance("2024-01-01", "2024-01-01", 1));
        assert!(dates_within_tolerance("2024-01-01", "2024-01-02", 1));
        assert!(!dates_within_tolerance("2024-01-01", "2024-01-03", 1));
    }
}
