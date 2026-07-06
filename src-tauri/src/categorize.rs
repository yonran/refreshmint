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
    /// Near-miss transfer candidates: when 2+ candidates match (post
    /// negative-memory filter), `transfer_match` stays `None` and this carries
    /// all of them in date-proximity order; empty when 0 or exactly 1 match.
    /// Keep in sync with `GlCategoryResult::transfer_candidates`.
    pub transfer_candidates: Vec<TransferMatch>,
    /// Counterpart account of the first matching active `CategoryRule` (see
    /// `automation::matching_rule_account`). Independent of `suggested`: rules are
    /// deterministic and get policy `Auto`, ML `suggested` stays `Review`. The
    /// three post paths use this before `Expenses:Unknown`; keep in sync with
    /// `GlCategoryResult::rule_account`.
    pub rule_account: Option<String>,
}

/// A uniquely matched transfer entry from another login account.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferMatch {
    pub account_locator: String,
    pub entry_id: String,
    pub matched_amount: String,
}

/// Per-GL-transaction result from `suggest_gl_categories`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlCategoryResult {
    /// ML-suggested replacement account for `Expenses:Unknown`, or `None` if
    /// confidence < 0.5 or a transfer match was found.
    pub suggested: Option<String>,
    /// Auto-detected transfer counterpart among other `Expenses:Unknown` GL
    /// transactions with opposite amount within ±3 days.
    pub transfer_match: Option<GlTransferMatch>,
    /// Near-miss transfer candidates: when 2+ candidates match (post
    /// negative-memory filter), `transfer_match` stays `None` and this carries
    /// all of them in date-proximity order; empty when 0 or exactly 1 match.
    /// Keep in sync with `CategoryResult::transfer_candidates`.
    pub transfer_candidates: Vec<GlTransferMatch>,
    /// Counterpart account of the first matching active `CategoryRule` (see
    /// `automation::matching_rule_account`). Independent of `suggested`; keep in
    /// sync with `CategoryResult::rule_account`.
    pub rule_account: Option<String>,
}

/// A matching `Expenses:Unknown` GL transaction that forms a transfer pair.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlTransferMatch {
    pub txn_id: String,
    pub description: String,
    pub date: String,
    pub matched_amount: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Class probability threshold below which the classifier abstains.
const CONFIDENCE_THRESHOLD: f64 = 0.5;

/// Default transfer matching date window (± days) when refreshmint.json does
/// not set `transferDateWindowDays`.
const DEFAULT_TRANSFER_DATE_WINDOW_DAYS: i64 = 3;

/// Ledger-configurable transfer matching settings, loaded from refreshmint.json
/// (`ledger::RefreshmintConfig`: `transferDateWindowDays`,
/// `extraTransferPatterns`). Threaded into both matchers by
/// suggest_categories / suggest_gl_categories.
pub struct TransferSettings {
    /// Date window (± days) for opposite-amount matching (default 3).
    pub date_window_days: i64,
    /// Additional case-insensitive substring patterns for the
    /// `is_probable_transfer` gate (default empty).
    pub extra_patterns: Vec<String>,
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            date_window_days: DEFAULT_TRANSFER_DATE_WINDOW_DAYS,
            extra_patterns: Vec::new(),
        }
    }
}

impl TransferSettings {
    /// Load from the ledger's refreshmint.json; a missing/unreadable file (or
    /// unset fields) yields the defaults.
    fn from_ledger(ledger_dir: &Path) -> Self {
        match crate::ledger::read_refreshmint_config(ledger_dir) {
            Ok(config) => Self {
                date_window_days: config
                    .transfer_date_window_days
                    .map(i64::from)
                    .unwrap_or(DEFAULT_TRANSFER_DATE_WINDOW_DAYS),
                extra_patterns: config.extra_transfer_patterns,
            },
            Err(_) => Self::default(),
        }
    }
}

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

    // Negative transfer memory (NotTransferLink resolutions). Filters blocked
    // candidates inside find_transfer_matches. See automation::TransferPolicy.
    let transfer_policy = crate::automation::TransferPolicy::from_resolutions(ledger_dir)?;

    // Ledger-configured transfer window/patterns (refreshmint.json).
    let transfer_settings = TransferSettings::from_ledger(ledger_dir);

    // Active category rules that apply to this login/account (global + scoped),
    // in newest-first match order. See automation::matching_rule_account.
    let rules =
        crate::automation::active_category_rules(ledger_dir, Some(login_name), Some(label))?;

    // Process each entry.
    let mut results = HashMap::new();
    for entry in &entries {
        let result = process_entry(
            entry,
            login_name,
            label,
            &gl_by_id,
            &source_locator,
            global_model.as_ref(),
            account_model.as_ref(),
            account_sample_count,
            &transfer_candidates,
            &transfer_policy,
            &transfer_settings,
            &rules,
        );
        results.insert(entry.id.clone(), result);
    }

    Ok(results)
}

/// Suggest categories and detect transfer pairs for all `Expenses:Unknown`
/// transactions already in `general.journal`.
///
/// Returns a `HashMap<txn_id, GlCategoryResult>`.
pub fn suggest_gl_categories(
    ledger_dir: &Path,
) -> Result<HashMap<String, GlCategoryResult>, Box<dyn std::error::Error + Send + Sync>> {
    let gl_journal_path = ledger_dir.join("general.journal");
    if !gl_journal_path.exists() {
        return Ok(HashMap::new());
    }
    let gl_txns = crate::ledger_open::run_hledger_print(&gl_journal_path).unwrap_or_default();

    // Find transactions that have an Expenses:Unknown posting.
    let unknown_txns: Vec<&crate::hledger::Transaction> = gl_txns
        .iter()
        .filter(|txn| {
            txn.tpostings
                .iter()
                .any(|p| p.paccount == "Expenses:Unknown")
        })
        .collect();

    if unknown_txns.is_empty() {
        return Ok(HashMap::new());
    }

    // Build ML model from GL transactions that already have real categories.
    let training_examples = build_gl_training_examples(&gl_txns);
    let global_model = MnbModel::fit(&training_examples, 1.0);

    // Build transfer candidates from the Expenses:Unknown set.
    let transfer_candidates = build_gl_transfer_candidates(&unknown_txns);

    // Negative transfer memory (NotTransferLink resolutions). Filters blocked
    // candidates inside find_gl_transfer_matches. See automation::TransferPolicy.
    let transfer_policy = crate::automation::TransferPolicy::from_resolutions(ledger_dir)?;

    // Ledger-configured transfer window (refreshmint.json). The GL matcher has
    // no description gate, so extra_patterns are unused here.
    let transfer_settings = TransferSettings::from_ledger(ledger_dir);

    // Global category rules apply to GL txns; per-txn scoping is refined below
    // from the txn's `source` tag when present. See automation::matching_rule_account
    // and CategoryResult::rule_account (kept in sync).
    let global_rules = crate::automation::active_category_rules(ledger_dir, None, None)?;
    // Scoped rules loaded once per distinct source account (global + that scope).
    let mut scoped_rules_cache: HashMap<(String, String), Vec<crate::automation::Resolution>> =
        HashMap::new();

    let mut results = HashMap::new();
    for txn in &unknown_txns {
        let txn_id = match txn.ttags.iter().find(|(k, _)| k == "id") {
            Some((_, v)) => v.clone(),
            None => continue,
        };

        // Transfer detection has priority over ML suggestion.
        let (transfer_match, near_miss_candidates) =
            unique_or_candidates(find_gl_transfer_matches(
                txn,
                &txn_id,
                &transfer_candidates,
                &transfer_policy,
                &transfer_settings,
            ));

        let suggested = if transfer_match.is_some() {
            None
        } else if let Some(model) = &global_model {
            // GL inference — MUST match GL training (build_gl_training_examples)
            // via the shared tokenize_description normalization.
            let tokens = tokenize_description(&txn.tdescription);
            let proba = model.predict_proba(&tokens);
            let total: f64 = proba.iter().map(|(p, _)| p).sum();
            if total > 0.0 {
                proba
                    .into_iter()
                    .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                    .and_then(|(prob, class)| {
                        if prob / total >= CONFIDENCE_THRESHOLD {
                            Some(class.to_string())
                        } else {
                            None
                        }
                    })
            } else {
                None
            }
        } else {
            None
        };

        // Category rule match (independent of ML `suggested`). Scope by the txn's
        // `source` tag when present, else global-only.
        let rules: &[crate::automation::Resolution] = match txn_source_login_label(txn) {
            Some((login, label)) => {
                // Propagate load errors instead of `unwrap_or_default()`, which
                // would silently drop ALL scoped rules for the account on an I/O
                // error (matching how the global load above propagates).
                let key = (login, label);
                if !scoped_rules_cache.contains_key(&key) {
                    let loaded = crate::automation::active_category_rules(
                        ledger_dir,
                        Some(&key.0),
                        Some(&key.1),
                    )?;
                    scoped_rules_cache.insert(key.clone(), loaded);
                }
                &scoped_rules_cache[&key]
            }
            None => &global_rules,
        };
        let rule_account =
            crate::automation::matching_rule_account(rules, &txn.tdescription, gl_txn_amount(txn));

        results.insert(
            txn_id,
            GlCategoryResult {
                suggested,
                transfer_match,
                transfer_candidates: near_miss_candidates,
                rule_account,
            },
        );
    }

    Ok(results)
}

/// Parse a generated GL txn's `source` tag
/// (`logins/{login}/accounts/{label}:{entry_id}`) into (login, label) for
/// CategoryRule scoping. Mirrors the GL format documented in post.rs.
fn txn_source_login_label(txn: &hledger::Transaction) -> Option<(String, String)> {
    let source = txn
        .ttags
        .iter()
        .find(|(k, _)| k == "source")
        .map(|(_, v)| v)?;
    let rest = source.strip_prefix("logins/")?;
    let (login, rest) = rest.split_once("/accounts/")?;
    let label = rest.rsplit_once(':').map_or(rest, |(label, _)| label);
    Some((login.to_string(), label.to_string()))
}

/// The signed amount of the first non-`Expenses:Unknown` posting of a generated GL
/// txn, for CategoryRule amount-bound matching. Mirrors `entry_signed_amount`.
fn gl_txn_amount(txn: &hledger::Transaction) -> Option<f64> {
    txn.tpostings
        .iter()
        .find(|posting| posting.paccount != "Expenses:Unknown")
        .and_then(|posting| posting.pamount.first())
        .map(|amount| amount.aquantity.floating_point)
}

/// Build training examples from GL transactions that have real (non-Unknown) categories.
///
/// Uses `tdescription` tokens and the last posting's account as the label.
fn build_gl_training_examples(gl_txns: &[crate::hledger::Transaction]) -> Vec<TrainingExample> {
    let mut examples = seed_examples();

    for txn in gl_txns {
        // Only refreshmint-post transactions.
        let is_post = txn
            .ttags
            .iter()
            .any(|(k, v)| k == "generated-by" && v == "refreshmint-post");
        if !is_post {
            continue;
        }

        // Only single-source (not transfers).
        let source_count = txn.ttags.iter().filter(|(k, _)| k == "source").count();
        if source_count != 1 {
            continue;
        }

        // Counterpart is the last posting.
        let counterpart = match txn.tpostings.last() {
            Some(p) => &p.paccount,
            None => continue,
        };

        // Skip uncategorized or empty. Mirrors build_training_examples.
        if counterpart == "Expenses:Unknown" || counterpart.is_empty() {
            continue;
        }

        // GL training — MUST match GL inference (suggest_gl_categories) via the
        // shared tokenize_description normalization.
        let tokens = tokenize_description(&txn.tdescription);
        examples.push((tokens, counterpart.clone()));
    }

    examples
}

/// Internal transfer candidate for GL-level transfer detection.
struct GlTransferCandidate {
    txn_id: String,
    description: String,
    date: String,
    amount_f64: f64,
    commodity: String,
    /// The candidate's source entry, parsed from its `source` tag. `None` for
    /// manual GL txns with no source tag (which can never carry negative memory).
    source: Option<crate::automation::TransferEntry>,
}

/// Build a list of transfer candidates from `Expenses:Unknown` GL transactions.
///
/// Each candidate's amount is taken from the non-`Expenses:Unknown` posting.
fn build_gl_transfer_candidates(
    unknown_txns: &[&crate::hledger::Transaction],
) -> Vec<GlTransferCandidate> {
    let mut candidates = Vec::new();
    for txn in unknown_txns {
        let txn_id = match txn.ttags.iter().find(|(k, _)| k == "id") {
            Some((_, v)) => v.clone(),
            None => continue,
        };
        // The explicit (non-Unknown) posting carries the amount.
        let posting = match txn
            .tpostings
            .iter()
            .find(|p| p.paccount != "Expenses:Unknown")
        {
            Some(p) => p,
            None => continue,
        };
        let amount = match posting.pamount.first() {
            Some(a) => a,
            None => continue,
        };
        candidates.push(GlTransferCandidate {
            txn_id,
            description: txn.tdescription.clone(),
            date: txn.tdate.clone(),
            amount_f64: amount.aquantity.floating_point,
            commodity: amount.acommodity.clone(),
            source: txn_source_triple(txn),
        });
    }
    candidates
}

/// Find all transfer-candidate matches for a GL `Expenses:Unknown` transaction,
/// in date-proximity order (closest date first): opposite amount (sum ≈ 0), same
/// commodity, date within ±3 days. The caller splits unique-match vs near-miss
/// semantics via [`unique_or_candidates`].
fn find_gl_transfer_matches(
    txn: &crate::hledger::Transaction,
    txn_id: &str,
    candidates: &[GlTransferCandidate],
    policy: &crate::automation::TransferPolicy,
    settings: &TransferSettings,
) -> Vec<GlTransferMatch> {
    // Get this transaction's explicit posting amount.
    let Some(posting) = txn
        .tpostings
        .iter()
        .find(|p| p.paccount != "Expenses:Unknown")
    else {
        return Vec::new();
    };
    let Some(amount) = posting.pamount.first() else {
        return Vec::new();
    };
    let amount_f64 = amount.aquantity.floating_point;
    if amount_f64.is_nan() {
        return Vec::new();
    }
    let Some(txn_date) = parse_date(&txn.tdate) else {
        return Vec::new();
    };
    // The subject txn's source entry (if any), for the same-account exclusion and
    // negative-memory filtering below.
    let subject_source = txn_source_triple(txn);

    let mut matches: Vec<(i64, &GlTransferCandidate)> = candidates
        .iter()
        .filter_map(|c| {
            let day_distance = parse_date(&c.date).map(|cd| (txn_date - cd).num_days().abs())?;
            (c.txn_id != txn_id
                && c.commodity == amount.acommodity
                && !c.amount_f64.is_nan()
                && (amount_f64 + c.amount_f64).abs() < crate::post::TRANSFER_CANCEL_EPSILON
                && day_distance <= settings.date_window_days
                // A candidate from the SAME login account is never a transfer leg
                // (mirrors the account-level cross-account rule: refund/charge
                // pairs must not collapse into self-transfers). Candidates without
                // a source tag are not excluded.
                && !match (&subject_source, &c.source) {
                    (Some((login_a, label_a, _)), Some((login_b, label_b, _))) => {
                        login_a == login_b && label_a == label_b
                    }
                    _ => false,
                }
                // Skip pairs marked not-a-transfer (both sides need a source tag).
                // Filtered BEFORE the exactly-one count so a blocked candidate does
                // not spoil uniqueness. See automation::TransferPolicy.
                && !match (&subject_source, &c.source) {
                    (Some(a), Some(b)) => policy.blocks(a, b),
                    _ => false,
                })
            .then_some((day_distance, c))
        })
        .collect();
    matches.sort_by_key(|(day_distance, _)| *day_distance);
    matches
        .into_iter()
        .map(|(_, m)| GlTransferMatch {
            txn_id: m.txn_id.clone(),
            description: m.description.clone(),
            date: m.date.clone(),
            matched_amount: format!("{} {}", m.amount_f64, m.commodity),
        })
        .collect()
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
        // Skip uncategorized or empty so the classifier never learns to predict
        // Expenses:Unknown as a class. Mirrors build_gl_training_examples.
        if counterpart_account.is_empty() || counterpart_account == "Expenses:Unknown" {
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
/// `"key:value"` strings for each entry tag. The description is payee-normalized
/// (see `tokenize_description`); tag tokens are left as-is.
pub(crate) fn tokenize_entry(entry: &account_journal::AccountEntry) -> Vec<String> {
    let mut tokens = tokenize_description(&entry.description);
    for (k, v) in &entry.tags {
        if v.is_empty() {
            tokens.push(k.clone());
        } else {
            tokens.push(format!("{k}:{v}"));
        }
    }
    tokens
}

/// Payee-normalize a merchant description (payee_normalize::normalize_payee) and
/// then tokenize it, so noisy variants of one merchant ("SQ *BLUE BOTTLE #12",
/// "BLUE BOTTLE") train and infer as the same tokens.
///
/// This is the single normalization point shared by ALL description tokenization —
/// account training/inference (tokenize_entry), GL training
/// (build_gl_training_examples), and GL inference (suggest_gl_categories). Routing
/// every site through here guarantees training and inference can never diverge.
fn tokenize_description(text: &str) -> Vec<String> {
    tokenize_text(&crate::payee_normalize::normalize_payee(text))
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

/// Find all transfer-candidate matches for an unposted entry, in date-proximity
/// order (closest date first): opposite amount (sum ≈ 0), same commodity, date
/// within ±3 days. The caller splits unique-match vs near-miss semantics via
/// [`unique_or_candidates`].
///
/// `policy` (negative transfer memory) filters blocked candidates BEFORE the
/// caller's exactly-one uniqueness count, so a blocked candidate cannot spoil
/// uniqueness for the remaining one. See `automation::TransferPolicy`.
fn find_transfer_matches(
    entry: &account_journal::AccountEntry,
    subject_login: &str,
    subject_label: &str,
    candidates: &[TransferCandidate],
    policy: &crate::automation::TransferPolicy,
    settings: &TransferSettings,
) -> Vec<TransferMatch> {
    let Some(first_posting) = entry.postings.first() else {
        return Vec::new();
    };
    let Some(amt) = first_posting.amount.as_ref() else {
        return Vec::new();
    };
    let entry_amount: f64 = amt.quantity.trim().parse().unwrap_or(f64::NAN);
    if entry_amount.is_nan() {
        return Vec::new();
    }
    let Some(entry_date) = parse_date(&entry.date) else {
        return Vec::new();
    };
    let subject = (
        subject_login.to_string(),
        subject_label.to_string(),
        entry.id.clone(),
    );

    let mut matches: Vec<(i64, &TransferCandidate)> = candidates
        .iter()
        .filter_map(|c| {
            let day_distance = parse_date(&c.date).map(|cd| (entry_date - cd).num_days().abs())?;
            (c.commodity == amt.commodity
                && !c.amount_f64.is_nan()
                && (entry_amount + c.amount_f64).abs() < crate::post::TRANSFER_CANCEL_EPSILON
                && day_distance <= settings.date_window_days
                && !candidate_is_blocked(policy, &subject, &c.locator, &c.entry_id))
            .then_some((day_distance, c))
        })
        .collect();
    matches.sort_by_key(|(day_distance, _)| *day_distance);
    matches
        .into_iter()
        .map(|(_, m)| TransferMatch {
            account_locator: m.locator.clone(),
            entry_id: m.entry_id.clone(),
            matched_amount: format!("{} {}", m.amount_f64, m.commodity),
        })
        .collect()
}

/// Split a matcher result into (unique match, near-miss candidates): exactly one
/// match → `(Some, [])`; 2+ → `(None, all)`; 0 → `(None, [])`. Shared by the
/// account-level and GL-level transfer detection so both report identical
/// near-miss semantics.
fn unique_or_candidates<T>(mut matches: Vec<T>) -> (Option<T>, Vec<T>) {
    if matches.len() == 1 {
        (matches.pop(), Vec::new())
    } else if matches.is_empty() {
        (None, Vec::new())
    } else {
        (None, matches)
    }
}

fn parse_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// Parse a `logins/{login}/accounts/{label}` locator. Mirrors
/// `automation::parse_login_account_locator`.
fn parse_login_account_locator(locator: &str) -> Option<(String, String)> {
    let rest = locator.strip_prefix("logins/")?;
    let (login, label) = rest.split_once("/accounts/")?;
    Some((login.to_string(), label.to_string()))
}

/// Whether `policy` marks the (subject, candidate) source pair not-a-transfer.
/// A candidate whose locator does not parse is never blocked. See
/// `automation::TransferPolicy`.
fn candidate_is_blocked(
    policy: &crate::automation::TransferPolicy,
    subject: &crate::automation::TransferEntry,
    candidate_locator: &str,
    candidate_entry_id: &str,
) -> bool {
    match parse_login_account_locator(candidate_locator) {
        Some((login, label)) => {
            policy.blocks(subject, &(login, label, candidate_entry_id.to_string()))
        }
        None => false,
    }
}

/// Parse a generated GL txn's `source` tag
/// (`logins/{login}/accounts/{label}:{entry_id}`) into a `TransferEntry`. Mirrors
/// `txn_source_login_label` (which drops the entry_id) and
/// `automation::parse_source_tag`.
fn txn_source_triple(txn: &hledger::Transaction) -> Option<crate::automation::TransferEntry> {
    let source = txn
        .ttags
        .iter()
        .find(|(k, _)| k == "source")
        .map(|(_, v)| v)?;
    let rest = source.strip_prefix("logins/")?;
    let (login, rest) = rest.split_once("/accounts/")?;
    let (label, entry_id) = rest.rsplit_once(':')?;
    Some((login.to_string(), label.to_string(), entry_id.to_string()))
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

#[allow(clippy::too_many_arguments)]
fn process_entry(
    entry: &account_journal::AccountEntry,
    login_name: &str,
    label: &str,
    gl_by_id: &HashMap<String, &hledger::Transaction>,
    _source_locator: &str,
    global_model: Option<&MnbModel>,
    account_model: Option<&MnbModel>,
    account_sample_count: usize,
    transfer_candidates: &[TransferCandidate],
    transfer_policy: &crate::automation::TransferPolicy,
    transfer_settings: &TransferSettings,
    rules: &[crate::automation::Resolution],
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
    let (transfer_match, near_miss_candidates, suggested) = if entry.posted.is_none() {
        let is_probable_transfer = transfer_detector::is_probable_transfer_with_extra(
            &entry.description,
            &transfer_settings.extra_patterns,
        ) || entry
            .tags
            .iter()
            .any(|(k, v)| k == "isTransfer" && v == "true");

        let (transfer_match, near_miss_candidates) = if is_probable_transfer {
            unique_or_candidates(find_transfer_matches(
                entry,
                login_name,
                label,
                transfer_candidates,
                transfer_policy,
                transfer_settings,
            ))
        } else {
            (None, Vec::new())
        };

        let suggested = if transfer_match.is_none() {
            suggest_category(entry, global_model, account_model, account_sample_count)
        } else {
            None
        };

        (transfer_match, near_miss_candidates, suggested)
    } else {
        (None, Vec::new(), None)
    };

    // Category rule match (independent of ML `suggested` and posted status).
    // Uses the raw description and the entry's signed posting amount.
    let rule_account = crate::automation::matching_rule_account(
        rules,
        &entry.description,
        entry_signed_amount(entry),
    );

    CategoryResult {
        suggested,
        amount_changed,
        status_changed,
        transfer_match,
        transfer_candidates: near_miss_candidates,
        rule_account,
    }
}

/// The entry's signed posting amount as f64 (first posting), for CategoryRule
/// amount-bound matching. `None` when unparseable or absent.
fn entry_signed_amount(entry: &account_journal::AccountEntry) -> Option<f64> {
    entry
        .postings
        .first()
        .and_then(|posting| posting.amount.as_ref())
        .and_then(|amount| amount.quantity.trim().parse::<f64>().ok())
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

    #[test]
    fn tokenize_description_normalizes_payee() {
        // Processor prefix + store number stripped by normalize_payee, so noisy
        // variants of one merchant collapse to the same tokens.
        assert_eq!(
            tokenize_description("SQ *BLUE BOTTLE #12"),
            vec!["BLUE".to_string(), "BOTTLE".to_string()]
        );
        assert_eq!(
            tokenize_description("SQ *BLUE BOTTLE #12"),
            tokenize_description("BLUE BOTTLE")
        );
        // The raw tokenizer, by contrast, keeps the "SQ" processor token.
        assert!(tokenize_text("SQ *BLUE BOTTLE #12").contains(&"SQ".to_string()));
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

    /// Old exactly-one semantics (find_transfer_matches + unique_or_candidates),
    /// kept as a helper so the pre-near-miss tests still assert the same behavior.
    fn find_transfer_match(
        entry: &AccountEntry,
        subject_login: &str,
        subject_label: &str,
        candidates: &[TransferCandidate],
        policy: &crate::automation::TransferPolicy,
    ) -> Option<TransferMatch> {
        unique_or_candidates(find_transfer_matches(
            entry,
            subject_login,
            subject_label,
            candidates,
            policy,
            &TransferSettings::default(),
        ))
        .0
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
        let result = find_transfer_match(
            &entry,
            "chase",
            "checking",
            &candidates,
            &crate::automation::TransferPolicy::default(),
        );
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.entry_id, "txn-b");
    }

    #[test]
    fn find_transfer_match_blocked_pair_returns_none() {
        // Negative transfer memory: a would-be-unique match is suppressed when the
        // pair is blocked. Mirrors dedup::policy_not_same_source_blocks_heuristic_match.
        let entry = make_entry("e1", "Transfer out", vec![]);
        let candidates = vec![make_candidate(
            "logins/boa/accounts/savings",
            "txn-b",
            "2024-01-15",
            21.32,
            "USD",
        )];
        let mut policy = crate::automation::TransferPolicy::default();
        policy.block_for_test(
            (
                "chase".to_string(),
                "checking".to_string(),
                "e1".to_string(),
            ),
            (
                "boa".to_string(),
                "savings".to_string(),
                "txn-b".to_string(),
            ),
        );
        assert!(find_transfer_match(&entry, "chase", "checking", &candidates, &policy).is_none());
    }

    #[test]
    fn find_transfer_match_blocked_candidate_does_not_spoil_uniqueness() {
        // Two same-amount candidates → ambiguous → None. Blocking one leaves the
        // other uniquely matching (blocked candidates are filtered BEFORE the count).
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
        assert!(find_transfer_match(
            &entry,
            "chase",
            "checking",
            &candidates,
            &crate::automation::TransferPolicy::default(),
        )
        .is_none());
        let mut policy = crate::automation::TransferPolicy::default();
        policy.block_for_test(
            (
                "chase".to_string(),
                "checking".to_string(),
                "e1".to_string(),
            ),
            (
                "boa".to_string(),
                "savings".to_string(),
                "txn-b".to_string(),
            ),
        );
        let m = find_transfer_match(&entry, "chase", "checking", &candidates, &policy).unwrap();
        assert_eq!(m.entry_id, "txn-c");
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
        assert!(find_transfer_match(
            &entry,
            "chase",
            "checking",
            &candidates,
            &crate::automation::TransferPolicy::default(),
        )
        .is_none());
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
        assert!(find_transfer_match(
            &entry,
            "chase",
            "checking",
            &candidates,
            &crate::automation::TransferPolicy::default(),
        )
        .is_none());
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
        assert!(find_transfer_match(
            &entry,
            "chase",
            "checking",
            &candidates,
            &crate::automation::TransferPolicy::default(),
        )
        .is_none());
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

    fn categorize_temp_dir(prefix: &str) -> std::path::PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-cat-{prefix}-{}-{now}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_gl_posting(account: &str) -> hledger::Posting {
        hledger::Posting {
            pdate: None,
            pdate2: None,
            pstatus: hledger::Status::Cleared,
            paccount: account.to_string(),
            pamount: vec![],
            pcomment: String::new(),
            ptype: hledger::PostingType::RegularPosting,
            ptags: vec![],
            pbalanceassertion: None,
            ptransaction_index: None,
            poriginal: None,
        }
    }

    fn make_generated_gl_txn(source: &str, counterpart: &str, desc: &str) -> hledger::Transaction {
        let pos = hledger::SourcePos {
            source_name: String::new(),
            source_line: 1,
            source_column: 1,
        };
        hledger::Transaction {
            tindex: 1,
            tprecedingcomment: String::new(),
            tsourcepos: hledger::SourceSpan(pos.clone(), pos),
            tdate: "2024-01-15".to_string(),
            tdate2: None,
            tstatus: hledger::Status::Cleared,
            tcode: String::new(),
            tdescription: desc.to_string(),
            tcomment: String::new(),
            ttags: vec![
                ("generated-by".to_string(), "refreshmint-post".to_string()),
                ("source".to_string(), source.to_string()),
            ],
            tpostings: vec![
                make_gl_posting("Assets:Checking"),
                make_gl_posting(counterpart),
            ],
        }
    }

    #[test]
    fn build_training_examples_skips_expenses_unknown_counterpart() {
        let dir = categorize_temp_dir("training-skip-unknown");

        // A login with one account and one posted entry.
        let mut cfg = login_config::LoginConfig::default();
        cfg.accounts.insert(
            "checking".to_string(),
            login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        login_config::write_login_config(&dir, "chase", &cfg).unwrap();

        let jpath = account_journal::login_account_journal_path(&dir, "chase", "checking");
        let unknown_entry = make_entry("e-unknown", "MYSTERY MERCHANT", vec![]);
        let known_entry = make_entry("e-known", "SHELL OIL", vec![]);
        account_journal::write_journal_at_path(&jpath, &[unknown_entry, known_entry]).unwrap();

        let locator = "logins/chase/accounts/checking";
        let gl_txns = vec![
            make_generated_gl_txn(
                &format!("{locator}:e-unknown"),
                "Expenses:Unknown",
                "MYSTERY MERCHANT",
            ),
            make_generated_gl_txn(&format!("{locator}:e-known"), "Expenses:Gas", "SHELL OIL"),
        ];

        let (global, account_specific) = build_training_examples(&dir, &gl_txns, locator).unwrap();

        // The classifier must never learn Expenses:Unknown as a class.
        assert!(
            !global.iter().any(|(_, class)| class == "Expenses:Unknown"),
            "Expenses:Unknown must not become a training example"
        );
        assert!(!account_specific
            .iter()
            .any(|(_, class)| class == "Expenses:Unknown"),);
        // But a genuinely categorized entry still contributes.
        assert!(
            account_specific
                .iter()
                .any(|(_, class)| class == "Expenses:Gas"),
            "a real counterpart should still be trained on"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn payee_rule_input(
        normalized_payee: &str,
        account: &str,
    ) -> crate::automation::NewResolutionInput {
        crate::automation::NewResolutionInput {
            kind: crate::automation::ResolutionKind::CategoryRule,
            subject_refs: vec![],
            parts: vec![crate::automation::ResolutionPart {
                amount: None,
                account: Some(account.to_string()),
                ref_: None,
                notes: None,
            }],
            notes: None,
            predicate: Some(crate::automation::CategoryRulePredicate {
                description_regex: None,
                normalized_payee: Some(normalized_payee.to_string()),
                amount_min: None,
                amount_max: None,
            }),
        }
    }

    #[test]
    fn suggest_categories_reports_rule_account() {
        let dir = categorize_temp_dir("rule-account-entry");
        let mut cfg = login_config::LoginConfig::default();
        cfg.accounts.insert(
            "checking".to_string(),
            login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        login_config::write_login_config(&dir, "chase", &cfg).unwrap();
        let jpath = account_journal::login_account_journal_path(&dir, "chase", "checking");
        account_journal::write_journal_at_path(
            &jpath,
            &[
                make_entry("e-match", "SAFEWAY #123", vec![]),
                make_entry("e-other", "COSTCO WHSE", vec![]),
            ],
        )
        .unwrap();

        // Global rule: normalize_payee("SAFEWAY #123") == "SAFEWAY".
        crate::automation::create_resolution(
            &dir,
            payee_rule_input("SAFEWAY", "Expenses:Groceries"),
        )
        .unwrap();

        let results = suggest_categories(&dir, "chase", "checking").unwrap();
        assert_eq!(
            results
                .get("e-match")
                .and_then(|r| r.rule_account.as_deref()),
            Some("Expenses:Groceries")
        );
        assert!(results["e-other"].rule_account.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two logins whose single entries form an opposite-amount pair, `days`
    /// apart, with the given subject description (shared setup for the
    /// configurable window/pattern tests).
    fn write_transfer_pair_ledger(
        prefix: &str,
        subject_desc: &str,
        days: u32,
    ) -> std::path::PathBuf {
        let dir = categorize_temp_dir(prefix);
        for (login, label) in [("chase", "checking"), ("boa", "savings")] {
            let mut cfg = login_config::LoginConfig::default();
            cfg.accounts.insert(
                label.to_string(),
                login_config::LoginAccountConfig { gl_account: None },
            );
            login_config::write_login_config(&dir, login, &cfg).unwrap();
        }
        // Subject: -21.32 on 2024-01-15 (make_entry defaults).
        account_journal::write_journal_at_path(
            &account_journal::login_account_journal_path(&dir, "chase", "checking"),
            &[make_entry("e-out", subject_desc, vec![])],
        )
        .unwrap();
        let mut other = make_entry("e-in", "Transfer in", vec![]);
        other.date = format!("2024-01-{:02}", 15 + days);
        other.postings[0].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "21.32".to_string(),
        });
        account_journal::write_journal_at_path(
            &account_journal::login_account_journal_path(&dir, "boa", "savings"),
            &[other],
        )
        .unwrap();
        dir
    }

    #[test]
    fn configured_date_window_admits_wider_match() {
        // 5 days apart: outside the default ±3 window, inside a configured 7.
        let dir = write_transfer_pair_ledger("window-config", "Transfer to savings", 5);

        let default_results = suggest_categories(&dir, "chase", "checking").unwrap();
        assert!(
            default_results["e-out"].transfer_match.is_none(),
            "5-day match must be outside the default window"
        );

        std::fs::write(
            dir.join("refreshmint.json"),
            r#"{"version":"0.0.0-test","transferDateWindowDays":7}"#,
        )
        .unwrap();
        let widened = suggest_categories(&dir, "chase", "checking").unwrap();
        assert_eq!(
            widened["e-out"]
                .transfer_match
                .as_ref()
                .map(|m| m.entry_id.as_str()),
            Some("e-in"),
            "transferDateWindowDays: 7 should admit the 5-day match"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn configured_extra_pattern_gates_custom_description() {
        // "MOVE MONEY 123" matches no built-in pattern; an extraTransferPatterns
        // entry (matched case-insensitively) turns on the transfer gate.
        let dir = write_transfer_pair_ledger("pattern-config", "Move Money 123", 0);

        let default_results = suggest_categories(&dir, "chase", "checking").unwrap();
        assert!(
            default_results["e-out"].transfer_match.is_none(),
            "custom description must not match built-in patterns"
        );

        std::fs::write(
            dir.join("refreshmint.json"),
            r#"{"version":"0.0.0-test","extraTransferPatterns":["move money"]}"#,
        )
        .unwrap();
        let gated = suggest_categories(&dir, "chase", "checking").unwrap();
        assert_eq!(
            gated["e-out"]
                .transfer_match
                .as_ref()
                .map(|m| m.entry_id.as_str()),
            Some("e-in"),
            "extraTransferPatterns should gate the custom description"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_categories_two_candidates_populate_transfer_candidates() {
        // Near-miss semantics: with 2+ post-filter matches, transfer_match stays
        // None and transfer_candidates carries all of them in date-proximity order.
        let dir = categorize_temp_dir("near-miss-account");
        let mut chase_cfg = login_config::LoginConfig::default();
        chase_cfg.accounts.insert(
            "checking".to_string(),
            login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        login_config::write_login_config(&dir, "chase", &chase_cfg).unwrap();
        let mut boa_cfg = login_config::LoginConfig::default();
        for label in ["savings", "brokerage"] {
            boa_cfg.accounts.insert(
                label.to_string(),
                login_config::LoginAccountConfig { gl_account: None },
            );
        }
        login_config::write_login_config(&dir, "boa", &boa_cfg).unwrap();

        // Subject: -21.32 on 2024-01-15 (make_entry defaults).
        let jpath = account_journal::login_account_journal_path(&dir, "chase", "checking");
        account_journal::write_journal_at_path(
            &jpath,
            &[make_entry("e-out", "Transfer to savings", vec![])],
        )
        .unwrap();
        // Two +21.32 candidates: savings 2 days away, brokerage same day.
        let mut far = make_entry("e-far", "Transfer in", vec![]);
        far.date = "2024-01-17".to_string();
        far.postings[0].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "21.32".to_string(),
        });
        let mut near = make_entry("e-near", "Transfer in", vec![]);
        near.postings[0].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "21.32".to_string(),
        });
        account_journal::write_journal_at_path(
            &account_journal::login_account_journal_path(&dir, "boa", "savings"),
            &[far],
        )
        .unwrap();
        account_journal::write_journal_at_path(
            &account_journal::login_account_journal_path(&dir, "boa", "brokerage"),
            &[near],
        )
        .unwrap();

        let results = suggest_categories(&dir, "chase", "checking").unwrap();
        let result = &results["e-out"];
        assert!(
            result.transfer_match.is_none(),
            "ambiguous match must not set transfer_match"
        );
        let ids: Vec<&str> = result
            .transfer_candidates
            .iter()
            .map(|c| c.entry_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["e-near", "e-far"],
            "candidates must be in date-proximity order"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_gl_categories_two_candidates_populate_transfer_candidates() {
        let dir = categorize_temp_dir("near-miss-gl");
        std::fs::write(
            dir.join("general.journal"),
            "2026-01-03 Transfer out  ; id: txn-out\n    \
             ; source: logins/chase/accounts/checking:e1\n    \
             Assets:Chase  -50.00 USD\n    Expenses:Unknown\n\n\
             2026-01-01 Transfer in far  ; id: txn-far\n    \
             ; source: logins/boa/accounts/savings:e2\n    \
             Assets:Boa  50.00 USD\n    Expenses:Unknown\n\n\
             2026-01-03 Transfer in near  ; id: txn-near\n    \
             ; source: logins/boa/accounts/brokerage:e3\n    \
             Assets:BoaBrokerage  50.00 USD\n    Expenses:Unknown\n",
        )
        .unwrap();

        let results = suggest_gl_categories(&dir).unwrap();
        let result = &results["txn-out"];
        assert!(
            result.transfer_match.is_none(),
            "ambiguous match must not set transfer_match"
        );
        let ids: Vec<&str> = result
            .transfer_candidates
            .iter()
            .map(|c| c.txn_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["txn-near", "txn-far"],
            "candidates must be in date-proximity order"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_gl_categories_excludes_same_source_account_candidates() {
        // Audit gap: a refund/charge pair from the SAME login account must not
        // collapse into a self-transfer. A candidate without a source tag is not
        // excluded (and here becomes the unique match).
        let dir = categorize_temp_dir("gl-same-account");
        std::fs::write(
            dir.join("general.journal"),
            "2026-01-03 Charge  ; id: txn-charge\n    \
             ; source: logins/chase/accounts/checking:e1\n    \
             Assets:Chase  -30.00 USD\n    Expenses:Unknown\n\n\
             2026-01-03 Refund  ; id: txn-refund\n    \
             ; source: logins/chase/accounts/checking:e2\n    \
             Assets:Chase  30.00 USD\n    Expenses:Unknown\n\n\
             2026-01-03 Manual counterpart  ; id: txn-manual\n    \
             Assets:Manual  30.00 USD\n    Expenses:Unknown\n",
        )
        .unwrap();

        let results = suggest_gl_categories(&dir).unwrap();
        let result = &results["txn-charge"];
        // txn-refund (same source account) is excluded; txn-manual (no source
        // tag) remains and matches uniquely.
        assert_eq!(
            result.transfer_match.as_ref().map(|m| m.txn_id.as_str()),
            Some("txn-manual"),
            "same-source-account candidate must be excluded; got {result:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_gl_categories_reports_rule_account() {
        let dir = categorize_temp_dir("rule-account-gl");
        std::fs::write(
            dir.join("general.journal"),
            "2026-01-01 SAFEWAY #123  ; id: txn-1\n    \
             ; source: logins/chase/accounts/checking:entry-1\n    \
             Assets:Chase  -21.32 USD\n    Expenses:Unknown\n",
        )
        .unwrap();

        crate::automation::create_resolution(
            &dir,
            payee_rule_input("SAFEWAY", "Expenses:Groceries"),
        )
        .unwrap();

        let results = suggest_gl_categories(&dir).unwrap();
        assert_eq!(
            results.get("txn-1").and_then(|r| r.rule_account.as_deref()),
            Some("Expenses:Groceries")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
