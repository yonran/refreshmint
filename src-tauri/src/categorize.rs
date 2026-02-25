//! Category suggestion for unposted account journal entries.
//!
//! Uses a from-scratch Multinomial Naïve Bayes (MNB) classifier trained on:
//! - A compile-time seed vocabulary (common merchant keywords and bank-category tags)
//! - User posting history extracted from `general.journal`
//!
//! Also detects amount/status drift for already-posted entries and performs
//! rule-based transfer auto-matching across login accounts.

use std::collections::HashMap;
use std::path::Path;

use crate::account_journal;
use crate::hledger;
use crate::ledger_open::run_hledger_print;
use crate::login_config;
use crate::transfer_detector;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Per-entry result from `suggest_categories`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryResult {
    /// Suggested counterpart account (only for unposted entries without a
    /// unique transfer match, and only when confidence ≥ 0.5).
    pub suggested: Option<String>,
    /// `true` if the entry's posting amount differs from the GL transaction amount.
    pub amount_changed: bool,
    /// `true` if the entry's status differs from the GL transaction status.
    pub status_changed: bool,
    /// Auto-detected transfer match (only set when a unique opposite-amount
    /// unposted entry exists within ±3 days across other login accounts).
    pub transfer_match: Option<TransferMatch>,
}

/// A uniquely matched transfer entry from another login account.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferMatch {
    pub account_locator: String,
    pub entry_id: String,
    pub matched_amount: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Class probability threshold below which the classifier abstains.
const CONFIDENCE_THRESHOLD: f64 = 0.5;

/// Number of per-account training examples at which per-account weight = 1.0.
const ACCOUNT_WARMUP_SIZE: f64 = 20.0;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Suggest counterpart categories for all entries in a login account.
///
/// Returns a `HashMap<entry_id, CategoryResult>`.
pub fn suggest_categories(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> Result<HashMap<String, CategoryResult>, Box<dyn std::error::Error + Send + Sync>> {
    // Load account journal entries.
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let entries = account_journal::read_journal_at_path(&journal_path)?;

    // Parse general.journal once (may not exist for new ledgers).
    let gl_journal_path = ledger_dir.join("general.journal");
    let gl_txns: Vec<hledger::Transaction> = if gl_journal_path.exists() {
        run_hledger_print(&gl_journal_path).unwrap_or_default()
    } else {
        vec![]
    };

    // Build id → Transaction index for O(1) lookup.
    let gl_by_id: HashMap<String, &hledger::Transaction> = gl_txns
        .iter()
        .filter_map(|txn| {
            txn.ttags
                .iter()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| (v.clone(), txn))
        })
        .collect();

    // Build MNB training data from user history.
    let source_locator = format!("logins/{login_name}/accounts/{label}");
    let (global_examples, account_examples) =
        build_training_examples(ledger_dir, &gl_txns, &source_locator)?;

    // Fit global and per-account classifiers.
    let global_model = MnbModel::fit(&global_examples, 1.0);
    let account_model = MnbModel::fit(&account_examples, 1.0);
    let account_sample_count = account_examples.len();

    // Collect unposted transfer candidates from other login accounts.
    let transfer_candidates = collect_transfer_candidates(ledger_dir, login_name, label)?;

    // Process each entry.
    let mut results = HashMap::new();
    for entry in &entries {
        let result = process_entry(
            entry,
            &gl_by_id,
            &source_locator,
            global_model.as_ref(),
            account_model.as_ref(),
            account_sample_count,
            &transfer_candidates,
        );
        results.insert(entry.id.clone(), result);
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Training data
// ---------------------------------------------------------------------------

/// A training example: `(tokens, counterpart_account)`.
type TrainingExample = (Vec<String>, String);

/// Build `(tokens, counterpart_account)` training examples from GL history.
///
/// Returns `(global_examples, per_account_examples)`.  Both start with the
/// compile-time seed vocabulary; the per-account set is then filtered to only
/// examples whose source locator matches the caller's login account.
fn build_training_examples(
    ledger_dir: &Path,
    gl_txns: &[hledger::Transaction],
    source_locator: &str,
) -> Result<(Vec<TrainingExample>, Vec<TrainingExample>), Box<dyn std::error::Error + Send + Sync>>
{
    let mut global = seed_examples();
    let mut account_specific: Vec<TrainingExample> = Vec::new();

    // Pre-load all login account journals into (locator, entry_id) → AccountEntry.
    let mut entry_map: HashMap<(String, String), account_journal::AccountEntry> = HashMap::new();
    if let Ok(logins) = login_config::list_logins(ledger_dir) {
        for login in &logins {
            let cfg = login_config::read_login_config(ledger_dir, login);
            for lbl in cfg.accounts.keys() {
                let jpath = account_journal::login_account_journal_path(ledger_dir, login, lbl);
                let loc = format!("logins/{login}/accounts/{lbl}");
                if let Ok(entries) = account_journal::read_journal_at_path(&jpath) {
                    for e in entries {
                        entry_map.insert((loc.clone(), e.id.clone()), e);
                    }
                }
            }
        }
    }

    for txn in gl_txns {
        // Only process transactions we generated.
        let is_ours = txn
            .ttags
            .iter()
            .any(|(k, v)| k == "generated-by" && v == "refreshmint-post");
        if !is_ours {
            continue;
        }

        // Gather source tags (key == "source").
        let sources: Vec<&str> = txn
            .ttags
            .iter()
            .filter(|(k, _)| k == "source")
            .map(|(_, v)| v.as_str())
            .collect();

        // Only single-source transactions (not transfers).
        if sources.len() != 1 {
            continue;
        }

        // Parse "locator:entry_id" — split at last colon.
        let src = sources[0];
        let Some(colon_pos) = src.rfind(':') else {
            continue;
        };
        let locator = &src[..colon_pos];
        let entry_id = &src[colon_pos + 1..];
        if locator.is_empty() || entry_id.is_empty() {
            continue;
        }

        // Look up the account entry for tokens.
        let Some(entry) = entry_map.get(&(locator.to_string(), entry_id.to_string())) else {
            continue;
        };

        // Counterpart is the last posting in our generated GL format.
        let Some(counterpart_posting) = txn.tpostings.last() else {
            continue;
        };
        let counterpart_account = counterpart_posting.paccount.clone();
        if counterpart_account.is_empty() {
            continue;
        }

        let tokens = tokenize_entry(entry);
        let example = (tokens, counterpart_account);
        if locator == source_locator {
            account_specific.push(example.clone());
        }
        global.push(example);
    }

    Ok((global, account_specific))
}

/// Compile-time seed vocabulary: common merchant keywords and bank category tags.
fn seed_examples() -> Vec<(Vec<String>, String)> {
    let raw: &[(&str, &str)] = &[
        // Bank category tag tokens
        ("category:Groceries", "Expenses:Groceries"),
        ("category:Dining", "Expenses:Dining"),
        ("category:Gas", "Expenses:Gas"),
        ("category:Shopping", "Expenses:Shopping"),
        ("category:Entertainment", "Expenses:Entertainment"),
        ("category:Travel", "Expenses:Travel"),
        ("category:Healthcare", "Expenses:Healthcare"),
        ("category:Utilities", "Expenses:Utilities"),
        ("category:Rent", "Expenses:Rent"),
        ("category:Insurance", "Expenses:Insurance"),
        // Merchant keywords
        ("SAFEWAY", "Expenses:Groceries"),
        ("KROGER", "Expenses:Groceries"),
        ("WHOLE", "Expenses:Groceries"),
        ("TRADER", "Expenses:Groceries"),
        ("STARBUCKS", "Expenses:Dining"),
        ("CHIPOTLE", "Expenses:Dining"),
        ("MCDONALDS", "Expenses:Dining"),
        ("DOORDASH", "Expenses:Dining"),
        ("GRUBHUB", "Expenses:Dining"),
        ("SHELL", "Expenses:Gas"),
        ("CHEVRON", "Expenses:Gas"),
        ("EXXON", "Expenses:Gas"),
        ("ARCO", "Expenses:Gas"),
        ("AMAZON", "Expenses:Shopping"),
        ("WALMART", "Expenses:Shopping"),
        ("TARGET", "Expenses:Shopping"),
        ("COSTCO", "Expenses:Shopping"),
        ("NETFLIX", "Expenses:Entertainment"),
        ("SPOTIFY", "Expenses:Entertainment"),
        ("HULU", "Expenses:Entertainment"),
        ("PAYROLL", "Income:Salary"),
        ("DEPOSIT", "Income:Salary"),
    ];
    raw.iter()
        .map(|(token, account)| (vec![token.to_string()], account.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Tokenisation
// ---------------------------------------------------------------------------

/// Tokenise an account journal entry into uppercase alphabetic words plus
/// `"key:value"` strings for each entry tag.
pub(crate) fn tokenize_entry(entry: &account_journal::AccountEntry) -> Vec<String> {
    let mut tokens = tokenize_text(&entry.description);
    for (k, v) in &entry.tags {
        if v.is_empty() {
            tokens.push(k.clone());
        } else {
            tokens.push(format!("{k}:{v}"));
        }
    }
    tokens
}

/// Split free text into uppercase alphabetic tokens (length ≥ 2).
pub(crate) fn tokenize_text(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_uppercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Transfer matching
// ---------------------------------------------------------------------------

/// Pre-loaded unposted entry from another login account.
struct TransferCandidate {
    locator: String,
    entry_id: String,
    date: String,
    amount_f64: f64,
    commodity: String,
}

fn collect_transfer_candidates(
    ledger_dir: &Path,
    exclude_login: &str,
    exclude_label: &str,
) -> Result<Vec<TransferCandidate>, Box<dyn std::error::Error + Send + Sync>> {
    let mut candidates = Vec::new();
    let logins = login_config::list_logins(ledger_dir)?;
    for login in &logins {
        let cfg = login_config::read_login_config(ledger_dir, login);
        for lbl in cfg.accounts.keys() {
            if login == exclude_login && lbl == exclude_label {
                continue;
            }
            let jpath = account_journal::login_account_journal_path(ledger_dir, login, lbl);
            let locator = format!("logins/{login}/accounts/{lbl}");
            if let Ok(entries) = account_journal::read_journal_at_path(&jpath) {
                for e in entries {
                    if e.posted.is_some() || !e.posted_postings.is_empty() {
                        continue;
                    }
                    let Some(first_posting) = e.postings.first() else {
                        continue;
                    };
                    let Some(amt) = &first_posting.amount else {
                        continue;
                    };
                    let amount_f64: f64 = amt.quantity.trim().parse().unwrap_or(f64::NAN);
                    candidates.push(TransferCandidate {
                        locator: locator.clone(),
                        entry_id: e.id.clone(),
                        date: e.date.clone(),
                        amount_f64,
                        commodity: amt.commodity.clone(),
                    });
                }
            }
        }
    }
    Ok(candidates)
}

/// Find a unique transfer match for an unposted entry.
///
/// Returns `Some(TransferMatch)` only when EXACTLY ONE candidate has the
/// opposite amount (sum ≈ 0), same commodity, and a date within ±3 days.
/// Returns `None` when there are 0 or 2+ matches.
fn find_transfer_match(
    entry: &account_journal::AccountEntry,
    candidates: &[TransferCandidate],
) -> Option<TransferMatch> {
    let first_posting = entry.postings.first()?;
    let amt = first_posting.amount.as_ref()?;
    let entry_amount: f64 = amt.quantity.trim().parse().unwrap_or(f64::NAN);
    if entry_amount.is_nan() {
        return None;
    }
    let entry_date = parse_date(&entry.date)?;

    let matches: Vec<&TransferCandidate> = candidates
        .iter()
        .filter(|c| {
            c.commodity == amt.commodity
                && !c.amount_f64.is_nan()
                && (entry_amount + c.amount_f64).abs() < 0.005
                && parse_date(&c.date)
                    .map(|cd| (entry_date - cd).num_days().abs() <= 3)
                    .unwrap_or(false)
        })
        .collect();

    if matches.len() == 1 {
        let m = matches[0];
        Some(TransferMatch {
            account_locator: m.locator.clone(),
            entry_id: m.entry_id.clone(),
            matched_amount: format!("{} {}", m.amount_f64, m.commodity),
        })
    } else {
        None
    }
}

fn parse_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

// ---------------------------------------------------------------------------
// Amount / status drift detection
// ---------------------------------------------------------------------------

fn entry_status_str(status: &account_journal::EntryStatus) -> &'static str {
    match status {
        account_journal::EntryStatus::Cleared => "Cleared",
        account_journal::EntryStatus::Pending => "Pending",
        account_journal::EntryStatus::Unmarked => "Unmarked",
    }
}

fn gl_status_str(status: &hledger::Status) -> &'static str {
    match status {
        hledger::Status::Cleared => "Cleared",
        hledger::Status::Pending => "Pending",
        hledger::Status::Unmarked => "Unmarked",
    }
}

// ---------------------------------------------------------------------------
// Per-entry processing
// ---------------------------------------------------------------------------

fn process_entry(
    entry: &account_journal::AccountEntry,
    gl_by_id: &HashMap<String, &hledger::Transaction>,
    _source_locator: &str,
    global_model: Option<&MnbModel>,
    account_model: Option<&MnbModel>,
    account_sample_count: usize,
    transfer_candidates: &[TransferCandidate],
) -> CategoryResult {
    // --- Amount / status drift (posted entries only) ---
    let (amount_changed, status_changed) = if let Some(gl_ref) = &entry.posted {
        let gl_txn_id = gl_ref.strip_prefix("general.journal:").unwrap_or(gl_ref);
        if let Some(txn) = gl_by_id.get(gl_txn_id) {
            let real_account = entry
                .postings
                .first()
                .map(|p| p.account.as_str())
                .unwrap_or("");

            let gl_posting = txn.tpostings.iter().find(|p| p.paccount == real_account);

            let amount_changed = if let (Some(entry_amt), Some(gl_post)) = (
                entry.postings.first().and_then(|p| p.amount.as_ref()),
                gl_posting,
            ) {
                // Use the pre-computed floating_point field from hledger JSON.
                if let Some(gl_amount) = gl_post.pamount.first() {
                    let entry_f64: f64 = entry_amt.quantity.trim().parse().unwrap_or(f64::NAN);
                    let gl_f64 = gl_amount.aquantity.floating_point;
                    entry_amt.commodity != gl_amount.acommodity
                        || (!entry_f64.is_nan() && (entry_f64 - gl_f64).abs() >= 1e-6)
                } else {
                    false
                }
            } else {
                false
            };

            let status_changed = entry_status_str(&entry.status) != gl_status_str(&txn.tstatus);

            (amount_changed, status_changed)
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    // --- Transfer detection + category suggestion (unposted entries only) ---
    let (transfer_match, suggested) = if entry.posted.is_none() {
        let is_probable_transfer = transfer_detector::is_probable_transfer(&entry.description)
            || entry
                .tags
                .iter()
                .any(|(k, v)| k == "isTransfer" && v == "true");

        let transfer_match = if is_probable_transfer {
            find_transfer_match(entry, transfer_candidates)
        } else {
            None
        };

        let suggested = if transfer_match.is_none() {
            suggest_category(entry, global_model, account_model, account_sample_count)
        } else {
            None
        };

        (transfer_match, suggested)
    } else {
        (None, None)
    };

    CategoryResult {
        suggested,
        amount_changed,
        status_changed,
        transfer_match,
    }
}

fn suggest_category(
    entry: &account_journal::AccountEntry,
    global_model: Option<&MnbModel>,
    account_model: Option<&MnbModel>,
    account_sample_count: usize,
) -> Option<String> {
    let tokens = tokenize_entry(entry);
    let global_proba = global_model?.predict_proba(&tokens);
    let alpha = (account_sample_count as f64 / ACCOUNT_WARMUP_SIZE).min(1.0);

    // Combine global and per-account probabilities.
    let mut combined: HashMap<&str, f64> = HashMap::new();
    for (prob, class) in &global_proba {
        *combined.entry(class).or_insert(0.0) += prob;
    }
    if alpha > 0.0 {
        if let Some(acct_model) = account_model {
            for (prob, class) in &acct_model.predict_proba(&tokens) {
                *combined.entry(class).or_insert(0.0) += alpha * prob;
            }
        }
    }

    // Normalise and apply threshold.
    let total: f64 = combined.values().sum();
    if total == 0.0 {
        return None;
    }
    combined
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(|(class, prob)| {
            if prob / total >= CONFIDENCE_THRESHOLD {
                Some(class.to_string())
            } else {
                None
            }
        })
}

// ---------------------------------------------------------------------------
// Multinomial Naïve Bayes (from scratch, no external ML deps)
// ---------------------------------------------------------------------------

struct MnbModel {
    classes: Vec<String>,
    /// log P(class_i)
    log_priors: Vec<f64>,
    /// log_likelihoods[class_i][vocab_j] = log P(token_j | class_i)
    log_likelihoods: Vec<Vec<f64>>,
    vocab: HashMap<String, usize>,
    vocab_size: usize,
}

impl MnbModel {
    /// Fit from `(tokens, class_label)` pairs with Laplace smoothing `alpha`.
    ///
    /// Returns `None` if fewer than 2 distinct classes are present.
    fn fit(examples: &[(Vec<String>, String)], alpha: f64) -> Option<Self> {
        // Build vocabulary (insertion-order index).
        let mut vocab: HashMap<String, usize> = HashMap::new();
        for (tokens, _) in examples {
            for token in tokens {
                let n = vocab.len();
                vocab.entry(token.clone()).or_insert(n);
            }
        }
        let vocab_size = vocab.len();

        // Group examples by class (preserve insertion order for determinism).
        let mut class_order: Vec<String> = Vec::new();
        let mut class_examples: HashMap<String, Vec<&[String]>> = HashMap::new();
        for (tokens, class) in examples {
            let e = class_examples.entry(class.clone()).or_default();
            if e.is_empty() {
                class_order.push(class.clone());
            }
            e.push(tokens.as_slice());
        }

        if class_order.len() < 2 {
            return None;
        }

        let total = examples.len() as f64;
        let mut log_priors = Vec::with_capacity(class_order.len());
        let mut log_likelihoods = Vec::with_capacity(class_order.len());

        for class in &class_order {
            let class_exs = &class_examples[class];
            log_priors.push((class_exs.len() as f64 / total).ln());

            let mut token_counts = vec![0.0_f64; vocab_size];
            for tokens in class_exs {
                for token in *tokens {
                    if let Some(&idx) = vocab.get(token) {
                        token_counts[idx] += 1.0;
                    }
                }
            }

            let total_count: f64 = token_counts.iter().sum::<f64>() + alpha * vocab_size as f64;
            let log_probs: Vec<f64> = token_counts
                .iter()
                .map(|&count| ((count + alpha) / total_count).ln())
                .collect();
            log_likelihoods.push(log_probs);
        }

        Some(MnbModel {
            classes: class_order,
            log_priors,
            log_likelihoods,
            vocab,
            vocab_size,
        })
    }

    /// Compute softmax class probabilities for the given token sequence.
    fn predict_proba<'a>(&'a self, tokens: &[String]) -> Vec<(f64, &'a str)> {
        let mut counts = vec![0_usize; self.vocab_size];
        for token in tokens {
            if let Some(&idx) = self.vocab.get(token) {
                counts[idx] += 1;
            }
        }

        // Compute log-score for each class.
        let log_scores: Vec<f64> = self
            .classes
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let mut score = self.log_priors[i];
                for (j, &count) in counts.iter().enumerate() {
                    if count > 0 {
                        score += count as f64 * self.log_likelihoods[i][j];
                    }
                }
                score
            })
            .collect();

        // Numerically stable softmax.
        let max_score = log_scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let exp_scores: Vec<f64> = log_scores.iter().map(|&s| (s - max_score).exp()).collect();
        let sum: f64 = exp_scores.iter().sum();

        self.classes
            .iter()
            .zip(exp_scores.iter())
            .map(|(class, &exp_score)| (exp_score / sum, class.as_str()))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::account_journal::{AccountEntry, EntryPosting, EntryStatus, SimpleAmount};

    fn make_entry(id: &str, desc: &str, tags: Vec<(String, String)>) -> AccountEntry {
        AccountEntry {
            id: id.to_string(),
            date: "2024-01-15".to_string(),
            status: EntryStatus::Cleared,
            description: desc.to_string(),
            comment: String::new(),
            evidence: vec![],
            postings: vec![EntryPosting {
                account: "Assets:Checking".to_string(),
                amount: Some(SimpleAmount {
                    commodity: "USD".to_string(),
                    quantity: "-21.32".to_string(),
                }),
            }],
            tags,
            extracted_by: None,
            posted: None,
            posted_postings: vec![],
        }
    }

    // --- Tokenisation ---

    #[test]
    fn tokenize_text_produces_uppercase_words() {
        let tokens = tokenize_text("Shell Oil 123 & Gas");
        assert!(tokens.contains(&"SHELL".to_string()));
        assert!(tokens.contains(&"OIL".to_string()));
        assert!(tokens.contains(&"GAS".to_string()));
        // Numbers and single-char tokens skipped.
        assert!(!tokens.iter().any(|t| t.chars().any(|c| c.is_ascii_digit())));
    }

    #[test]
    fn tokenize_entry_includes_tags() {
        let entry = make_entry(
            "e1",
            "Grocery Store",
            vec![("category".to_string(), "Groceries".to_string())],
        );
        let tokens = tokenize_entry(&entry);
        assert!(tokens.contains(&"GROCERY".to_string()));
        assert!(tokens.contains(&"category:Groceries".to_string()));
    }

    // --- MNB model ---

    #[test]
    fn mnb_fit_returns_none_for_single_class() {
        let examples = vec![
            (
                vec!["SAFEWAY".to_string()],
                "Expenses:Groceries".to_string(),
            ),
            (vec!["KROGER".to_string()], "Expenses:Groceries".to_string()),
        ];
        assert!(MnbModel::fit(&examples, 1.0).is_none());
    }

    #[test]
    fn mnb_suggests_known_token_from_seeds() {
        // Even with only seed examples, SAFEWAY should rank Groceries highest.
        let examples = seed_examples();
        let model = MnbModel::fit(&examples, 1.0).unwrap();
        let tokens = vec!["SAFEWAY".to_string()];
        let proba = model.predict_proba(&tokens);
        let best = proba
            .into_iter()
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (_, best_class) = best.unwrap();
        assert_eq!(best_class, "Expenses:Groceries");
    }

    #[test]
    fn mnb_abstains_on_unknown_token() {
        // With only seed examples, an unknown token should spread probability
        // across all classes uniformly → max prob < threshold.
        let examples = seed_examples();
        let model = MnbModel::fit(&examples, 1.0).unwrap();
        let tokens = vec!["ZZZZUNKNOWNMERCHANT".to_string()];
        let proba = model.predict_proba(&tokens);
        let best_prob = proba.into_iter().map(|(p, _)| p).fold(0.0_f64, f64::max);
        // With Laplace smoothing the unknown token is spread evenly; best
        // class probability should be well below 0.5 for unseen tokens.
        assert!(
            best_prob < CONFIDENCE_THRESHOLD,
            "expected abstain, got prob={best_prob}"
        );
    }

    // --- Transfer matching ---

    fn make_candidate(
        locator: &str,
        entry_id: &str,
        date: &str,
        amount: f64,
        commodity: &str,
    ) -> TransferCandidate {
        TransferCandidate {
            locator: locator.to_string(),
            entry_id: entry_id.to_string(),
            date: date.to_string(),
            amount_f64: amount,
            commodity: commodity.to_string(),
        }
    }

    #[test]
    fn find_transfer_match_unique_candidate() {
        let entry = make_entry("e1", "Transfer out", vec![]);
        // Entry amount is -21.32 USD; candidate is +21.32 USD, same date.
        let candidates = vec![make_candidate(
            "logins/boa/accounts/savings",
            "txn-b",
            "2024-01-15",
            21.32,
            "USD",
        )];
        let result = find_transfer_match(&entry, &candidates);
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.entry_id, "txn-b");
    }

    #[test]
    fn find_transfer_match_two_candidates_returns_none() {
        let entry = make_entry("e1", "Transfer out", vec![]);
        let candidates = vec![
            make_candidate(
                "logins/boa/accounts/savings",
                "txn-b",
                "2024-01-15",
                21.32,
                "USD",
            ),
            make_candidate(
                "logins/boa/accounts/checking",
                "txn-c",
                "2024-01-15",
                21.32,
                "USD",
            ),
        ];
        assert!(find_transfer_match(&entry, &candidates).is_none());
    }

    #[test]
    fn find_transfer_match_different_commodity_returns_none() {
        let entry = make_entry("e1", "Transfer out", vec![]);
        let candidates = vec![make_candidate(
            "logins/boa/accounts/savings",
            "txn-b",
            "2024-01-15",
            21.32,
            "EUR", // wrong commodity
        )];
        assert!(find_transfer_match(&entry, &candidates).is_none());
    }

    #[test]
    fn find_transfer_match_outside_date_window_returns_none() {
        let entry = make_entry("e1", "Transfer out", vec![]);
        let candidates = vec![make_candidate(
            "logins/boa/accounts/savings",
            "txn-b",
            "2024-01-19", // 4 days later → outside ±3
            21.32,
            "USD",
        )];
        assert!(find_transfer_match(&entry, &candidates).is_none());
    }

    // --- suggest_category integration ---

    #[test]
    fn suggest_category_returns_groceries_for_safeway() {
        // Build a denser training set so the model is confident (prob >= 0.5).
        let mut examples: Vec<(Vec<String>, String)> = Vec::new();
        for _ in 0..20 {
            examples.push((
                vec!["SAFEWAY".to_string()],
                "Expenses:Groceries".to_string(),
            ));
        }
        for _ in 0..5 {
            examples.push((vec!["STARBUCKS".to_string()], "Expenses:Dining".to_string()));
        }
        let model = MnbModel::fit(&examples, 1.0).unwrap();
        let entry = make_entry("e1", "SAFEWAY #123", vec![]);
        let result = suggest_category(&entry, Some(&model), None, 0);
        assert_eq!(result.as_deref(), Some("Expenses:Groceries"));
    }

    #[test]
    fn suggest_category_returns_none_for_unknown_merchant() {
        let examples = seed_examples();
        let model = MnbModel::fit(&examples, 1.0).unwrap();
        let entry = make_entry("e1", "ZZMYSTERYMERCHANT", vec![]);
        let result = suggest_category(&entry, Some(&model), None, 0);
        // Should abstain when confidence is low.
        assert!(result.is_none(), "expected None, got {result:?}");
    }
}
