use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::account_journal::{self, AccountEntry};
use crate::bookkeeping::{self, TypedRef, TypedRefKind};

const RESOLUTIONS_DIR: &str = "resolutions";

/// Cent-rounding tolerance for CategoryRule amount-bound comparisons.
const AMOUNT_EPSILON: f64 = 0.005;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub id: String,
    pub kind: ResolutionKind,
    pub status: ResolutionStatus,
    pub subject_refs: Vec<TypedRef>,
    pub parts: Vec<ResolutionPart>,
    pub notes: Option<String>,
    /// Predicate for `CategoryRule` resolutions. Additive/optional: pre-existing
    /// resolution JSON files (which never had this field) still parse, and the
    /// fingerprint only serializes it when `Some` so their ids stay byte-stable
    /// (see `resolution_fingerprint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<CategoryRulePredicate>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionKind {
    SameSource,
    NotSameSource,
    Category,
    /// Standing predicate rule: when a scraped entry (or Unknown GL txn) matches
    /// the `Resolution.predicate`, post/recategorize directly to the rule's
    /// account. Unlike `Category`, it is not bound to a specific entry id and can
    /// fire repeatedly. See `matching_rule_account` / `active_category_rules`.
    CategoryRule,
    PostingSplit,
    TransferLink,
    TransferSplit,
    IgnoreSource,
    PendingRetired,
    ReversalLink,
}

/// Predicate for a [`ResolutionKind::CategoryRule`]. A rule matches an entry when
/// every set field matches (logical AND). Validation requires at least one of
/// `description_regex` / `normalized_payee` (see `validate_resolution_input`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRulePredicate {
    /// Case-insensitive regex tested against the RAW (un-normalized) description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_regex: Option<String>,
    /// Exact match against `payee_normalize::normalize_payee(description)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_payee: Option<String>,
    /// Inclusive lower bound on the signed posting amount (parsed f64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_min: Option<String>,
    /// Inclusive upper bound on the signed posting amount (parsed f64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_max: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionPart {
    pub amount: Option<String>,
    pub account: Option<String>,
    #[serde(rename = "ref")]
    pub ref_: Option<TypedRef>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewResolutionInput {
    pub kind: ResolutionKind,
    pub subject_refs: Vec<TypedRef>,
    pub parts: Vec<ResolutionPart>,
    pub notes: Option<String>,
    #[serde(default)]
    pub predicate: Option<CategoryRulePredicate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationScope {
    pub login_name: Option<String>,
    pub label: Option<String>,
    pub include_gl: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationProposal {
    pub id: String,
    pub kind: AutomationProposalKind,
    pub subject_refs: Vec<TypedRef>,
    pub proposed_result: ProposalResult,
    pub reasons: Vec<ProposalReason>,
    pub blockers: Vec<ProposalBlocker>,
    pub can_apply: bool,
    pub policy_decision: ProposalPolicyDecision,
    pub reversible: ProposalReversibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutomationProposalKind {
    MergeSource,
    PreventMerge,
    RetirePending,
    PostCategory,
    PostSplit,
    LinkTransfer,
    MergeGlTransfer,
    /// Replace an existing `Expenses:Unknown` GL posting with a real account.
    /// Emitted by `gl_proposals`: rule match → Auto, ML suggestion → Review.
    RecategorizeGl,
    SyncPosted,
    ReviewAnomaly,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalResult {
    pub suggested_account: Option<String>,
    pub transfer_match: Option<ProposalTransferMatch>,
    pub parts: Vec<ResolutionPart>,
    pub import_anomaly_id: Option<String>,
    pub resolution_id: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalTransferMatch {
    pub login_name: String,
    pub label: String,
    pub entry_id: String,
    pub matched_amount: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalReason {
    pub field: String,
    pub result: ProposalReasonResult,
    pub detail: String,
    pub weight: Option<ProposalReasonWeight>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalReasonResult {
    Matched,
    Similar,
    DerivedFromResolution,
    ModelSuggested,
    CoveredByExport,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalReasonWeight {
    Exact,
    Strong,
    Weak,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalBlocker {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalPolicyDecision {
    Auto,
    Review,
    Blocked,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalReversibility {
    Yes,
    Conditional,
    No,
}

pub fn ensure_automation_layout(ledger_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(resolutions_dir(ledger_dir))
}

pub fn list_resolutions(ledger_dir: &Path) -> io::Result<Vec<Resolution>> {
    let dir = resolutions_dir(ledger_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut resolutions = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(path)?;
        resolutions.push(serde_json::from_str::<Resolution>(&content).map_err(json_error)?);
    }
    resolutions.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(resolutions)
}

pub fn create_resolution(ledger_dir: &Path, input: NewResolutionInput) -> io::Result<Resolution> {
    ensure_automation_layout(ledger_dir)?;
    validate_resolution_input(&input)?;
    let fingerprint = resolution_fingerprint(
        &input.kind,
        &input.subject_refs,
        &input.parts,
        &input.predicate,
    )?;
    for existing in list_resolutions(ledger_dir)? {
        if resolution_matches_fingerprint(&existing, &fingerprint)? {
            if existing.status == ResolutionStatus::Active {
                return Ok(existing);
            }
            return reactivate_resolution(ledger_dir, existing, input);
        }
    }
    let id = stable_id(&fingerprint);
    let now = crate::operations::now_timestamp();
    let resolution = Resolution {
        id,
        kind: input.kind,
        status: ResolutionStatus::Active,
        subject_refs: input.subject_refs,
        parts: input.parts,
        notes: normalize_optional(input.notes),
        predicate: input.predicate,
        created_at: now.clone(),
        updated_at: now,
    };
    fs::write(
        resolution_path(ledger_dir, &resolution.id),
        serde_json::to_string_pretty(&resolution).map_err(json_error)?,
    )?;
    Ok(resolution)
}

fn reactivate_resolution(
    ledger_dir: &Path,
    existing: Resolution,
    input: NewResolutionInput,
) -> io::Result<Resolution> {
    let resolution = Resolution {
        id: existing.id,
        kind: input.kind,
        status: ResolutionStatus::Active,
        subject_refs: input.subject_refs,
        parts: input.parts,
        notes: normalize_optional(input.notes),
        predicate: input.predicate,
        created_at: existing.created_at,
        updated_at: crate::operations::now_timestamp(),
    };
    fs::write(
        resolution_path(ledger_dir, &resolution.id),
        serde_json::to_string_pretty(&resolution).map_err(json_error)?,
    )?;
    Ok(resolution)
}

fn resolution_matches_fingerprint(resolution: &Resolution, fingerprint: &str) -> io::Result<bool> {
    Ok(resolution_fingerprint(
        &resolution.kind,
        &resolution.subject_refs,
        &resolution.parts,
        &resolution.predicate,
    )? == fingerprint)
}

fn resolution_fingerprint(
    kind: &ResolutionKind,
    subject_refs: &[TypedRef],
    parts: &[ResolutionPart],
    predicate: &Option<CategoryRulePredicate>,
) -> io::Result<String> {
    let canonical = canonical_subject_refs(kind, subject_refs);
    // Serialize the predicate only when present so that pre-CategoryRule kinds
    // (predicate == None) produce a byte-identical fingerprint to the historical
    // 3-tuple, keeping their stable ids unchanged.
    match predicate {
        None => serde_json::to_string(&(kind, canonical, parts)),
        Some(predicate) => serde_json::to_string(&(kind, canonical, parts, predicate)),
    }
    .map_err(json_error)
}

fn canonical_subject_refs(kind: &ResolutionKind, refs: &[TypedRef]) -> Vec<TypedRef> {
    let mut refs = refs.to_vec();
    if resolution_subject_order_is_commutative(kind) {
        refs.sort_by_key(typed_ref_sort_key);
    }
    refs
}

fn resolution_subject_order_is_commutative(kind: &ResolutionKind) -> bool {
    matches!(
        kind,
        ResolutionKind::SameSource
            | ResolutionKind::NotSameSource
            | ResolutionKind::TransferLink
            | ResolutionKind::TransferSplit
            | ResolutionKind::ReversalLink
    )
}

fn typed_ref_sort_key(value: &TypedRef) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn disable_resolution(ledger_dir: &Path, id: &str) -> io::Result<Resolution> {
    update_resolution_status(ledger_dir, id, ResolutionStatus::Disabled)
}

pub fn enable_resolution(ledger_dir: &Path, id: &str) -> io::Result<Resolution> {
    update_resolution_status(ledger_dir, id, ResolutionStatus::Active)
}

fn update_resolution_status(
    ledger_dir: &Path,
    id: &str,
    status: ResolutionStatus,
) -> io::Result<Resolution> {
    let id = require_non_empty("id", id)?;
    let path = resolution_path(ledger_dir, id);
    let mut resolution: Resolution =
        serde_json::from_str(&fs::read_to_string(&path)?).map_err(json_error)?;
    resolution.status = status;
    resolution.updated_at = crate::operations::now_timestamp();
    fs::write(
        &path,
        serde_json::to_string_pretty(&resolution).map_err(json_error)?,
    )?;
    Ok(resolution)
}

/// Compile a CategoryRule `description_regex` (case-insensitive). Shared by
/// validation (`validate_category_rule_input`) and matching (`predicate_matches`)
/// so both agree on the regex flavor.
fn compile_rule_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
}

/// Load active [`ResolutionKind::CategoryRule`] resolutions that apply to the given
/// scope, in match order (updated_at desc, then id asc — the order
/// `list_resolutions` returns). A rule applies when it is global (empty
/// subject_refs) or its single scope ref names `login_name`/`label`. Pass
/// `None`/`None` to get only global rules (used for GL txns that lack a reliable
/// login/account context). Paired with [`matching_rule_account`].
pub fn active_category_rules(
    ledger_dir: &Path,
    login_name: Option<&str>,
    label: Option<&str>,
) -> io::Result<Vec<Resolution>> {
    Ok(list_resolutions(ledger_dir)?
        .into_iter()
        .filter(|rule| {
            rule.kind == ResolutionKind::CategoryRule && rule.status == ResolutionStatus::Active
        })
        .filter(|rule| category_rule_matches_scope(rule, login_name, label))
        .collect())
}

/// Whether a CategoryRule applies to the given login/label scope. Global rules
/// (no subject_refs) always apply; scoped rules apply only to their login/account.
fn category_rule_matches_scope(
    rule: &Resolution,
    login_name: Option<&str>,
    label: Option<&str>,
) -> bool {
    match rule.subject_refs.as_slice() {
        [] => true,
        [scope] => match (login_name, label) {
            (Some(login), Some(label)) => scope
                .locator
                .as_deref()
                .and_then(parse_login_account_locator)
                .is_some_and(|(scope_login, scope_label)| {
                    scope_login == login && scope_label == label
                }),
            _ => false,
        },
        _ => false,
    }
}

/// First active CategoryRule whose predicate matches, returning its account. Rules
/// must already be scope-filtered (see [`active_category_rules`]) and are tried in
/// order — first match wins. Consumed by categorize.rs `suggest_categories` /
/// `suggest_gl_categories` to fill `rule_account`.
pub fn matching_rule_account(
    rules: &[Resolution],
    description: &str,
    amount: Option<f64>,
) -> Option<String> {
    for rule in rules {
        let Some(predicate) = &rule.predicate else {
            continue;
        };
        if predicate_matches(predicate, description, amount) {
            if let Some(account) = rule.parts.iter().find_map(|part| part.account.clone()) {
                return Some(account);
            }
        }
    }
    None
}

/// Whether every set field of `predicate` matches the entry (logical AND). Amount
/// bounds compare the signed posting amount with a cent-rounding tolerance.
fn predicate_matches(
    predicate: &CategoryRulePredicate,
    description: &str,
    amount: Option<f64>,
) -> bool {
    if let Some(pattern) = &predicate.description_regex {
        match compile_rule_regex(pattern) {
            Ok(regex) if regex.is_match(description) => {}
            _ => return false,
        }
    }
    if let Some(expected) = &predicate.normalized_payee {
        if &crate::payee_normalize::normalize_payee(description) != expected {
            return false;
        }
    }
    if let Some(min) = &predicate.amount_min {
        match (amount, min.parse::<f64>()) {
            (Some(amount), Ok(min)) if amount >= min - AMOUNT_EPSILON => {}
            _ => return false,
        }
    }
    if let Some(max) = &predicate.amount_max {
        match (amount, max.parse::<f64>()) {
            (Some(amount), Ok(max)) if amount <= max + AMOUNT_EPSILON => {}
            _ => return false,
        }
    }
    true
}

pub fn list_automation_proposals(
    ledger_dir: &Path,
    scope: AutomationScope,
) -> Result<Vec<AutomationProposal>, Box<dyn std::error::Error + Send + Sync>> {
    let resolutions = list_resolutions(ledger_dir)?;
    let mut proposals = Vec::new();
    if let (Some(login_name), Some(label)) = (scope.login_name.as_deref(), scope.label.as_deref()) {
        proposals.extend(login_account_proposals(
            ledger_dir,
            login_name,
            label,
            &resolutions,
        )?);
    } else if scope.login_name.is_none() && scope.label.is_none() {
        for login_name in crate::login_config::list_logins(ledger_dir)? {
            let config = crate::login_config::read_login_config(ledger_dir, &login_name);
            for label in config.accounts.keys() {
                proposals.extend(login_account_proposals(
                    ledger_dir,
                    &login_name,
                    label,
                    &resolutions,
                )?);
            }
        }
    }
    if scope.include_gl.unwrap_or(false) {
        proposals.extend(gl_proposals(ledger_dir)?);
    }
    proposals.extend(import_anomaly_proposals(
        ledger_dir,
        scope.login_name,
        scope.label,
    )?);
    proposals.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(proposals)
}

pub fn apply_automation_proposal(
    ledger_dir: &Path,
    proposal_id: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let proposal_id = require_non_empty("proposal_id", proposal_id)?;
    let proposals = list_automation_proposals(
        ledger_dir,
        AutomationScope {
            login_name: None,
            label: None,
            include_gl: Some(true),
        },
    )?;
    let Some(proposal) = proposals.into_iter().find(|p| p.id == proposal_id) else {
        return Err(format!("automation proposal not found: {proposal_id}").into());
    };
    if !proposal.blockers.is_empty() || proposal.policy_decision == ProposalPolicyDecision::Blocked
    {
        return Err(format!("automation proposal is blocked: {proposal_id}").into());
    }
    apply_proposal(ledger_dir, proposal)
}

pub fn apply_automation_policy(
    ledger_dir: &Path,
    scope: AutomationScope,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut applied = Vec::new();
    loop {
        let Some(proposal) = list_automation_proposals(ledger_dir, scope.clone())?
            .into_iter()
            .find(|proposal| {
                proposal.policy_decision == ProposalPolicyDecision::Auto
                    && proposal.blockers.is_empty()
                    && proposal_is_applyable(proposal)
            })
        else {
            break;
        };
        applied.push(apply_proposal(ledger_dir, proposal)?);
    }
    Ok(applied)
}

fn login_account_proposals(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    resolutions: &[Resolution],
) -> Result<Vec<AutomationProposal>, Box<dyn std::error::Error + Send + Sync>> {
    let entries = account_journal::read_journal_at_path(
        &account_journal::login_account_journal_path(ledger_dir, login_name, label),
    )?;
    let suggestions = crate::categorize::suggest_categories(ledger_dir, login_name, label)?;
    let active = ActiveResolutions::new(resolutions);
    let mut proposals = Vec::new();
    for entry in entries {
        let source_ref = login_entry_ref(login_name, label, &entry.id);
        if active.has_ignore_source(&source_ref) {
            continue;
        }
        if entry.posted.is_none() && entry.posted_postings.is_empty() {
            proposals.extend(resolution_backed_proposals(
                login_name, label, &entry, &active,
            ));
        }
        let Some(suggestion) = suggestions.get(&entry.id) else {
            continue;
        };
        if suggestion.amount_changed || suggestion.status_changed {
            proposals.push(sync_posted_proposal(login_name, label, &entry, suggestion));
        }
        if entry.posted.is_some() || !entry.posted_postings.is_empty() {
            continue;
        }
        if active.entry_is_resolved(&source_ref) {
            continue;
        }
        if let Some(transfer) = &suggestion.transfer_match {
            if let Some((other_login, other_label)) =
                parse_login_account_locator(&transfer.account_locator)
            {
                proposals.push(link_transfer_proposal(LinkTransferProposalInput {
                    login_name,
                    label,
                    entry: &entry,
                    other_login: &other_login,
                    other_label: &other_label,
                    other_entry_id: &transfer.entry_id,
                    matched_amount: &transfer.matched_amount,
                    resolution_id: None,
                    policy_decision: ProposalPolicyDecision::Review,
                }));
            }
        } else if let Some(account) = &suggestion.suggested {
            proposals.push(post_category_proposal(
                login_name,
                label,
                &entry,
                account,
                None,
                ProposalPolicyDecision::Review,
                ProposalReason {
                    field: "category".to_string(),
                    result: ProposalReasonResult::ModelSuggested,
                    detail: account.clone(),
                    weight: Some(ProposalReasonWeight::Weak),
                },
            ));
        }
    }
    Ok(proposals)
}

fn resolution_backed_proposals(
    login_name: &str,
    label: &str,
    entry: &AccountEntry,
    active: &ActiveResolutions<'_>,
) -> Vec<AutomationProposal> {
    let source_ref = login_entry_ref(login_name, label, &entry.id);
    let mut proposals = Vec::new();
    for resolution in active.for_subject(&source_ref) {
        match resolution.kind {
            ResolutionKind::Category => {
                if let Some(account) = resolution
                    .parts
                    .iter()
                    .find_map(|part| part.account.clone())
                {
                    proposals.push(post_category_proposal(
                        login_name,
                        label,
                        entry,
                        &account,
                        Some(resolution.id.clone()),
                        ProposalPolicyDecision::Auto,
                        ProposalReason {
                            field: "resolution".to_string(),
                            result: ProposalReasonResult::DerivedFromResolution,
                            detail: resolution.id.clone(),
                            weight: Some(ProposalReasonWeight::Exact),
                        },
                    ));
                }
            }
            ResolutionKind::TransferLink => {
                let other = resolution
                    .subject_refs
                    .iter()
                    .find(|candidate| *candidate != &source_ref)
                    .and_then(parse_login_entry_ref);
                if let Some((other_login, other_label, other_entry_id)) = other {
                    proposals.push(link_transfer_proposal(LinkTransferProposalInput {
                        login_name,
                        label,
                        entry,
                        other_login: &other_login,
                        other_label: &other_label,
                        other_entry_id: &other_entry_id,
                        matched_amount: "",
                        resolution_id: Some(resolution.id.clone()),
                        policy_decision: ProposalPolicyDecision::Auto,
                    }));
                }
            }
            ResolutionKind::PostingSplit => {
                if resolution.parts.len() >= 2 {
                    proposals.push(post_split_proposal(login_name, label, entry, resolution));
                }
            }
            ResolutionKind::SameSource | ResolutionKind::NotSameSource => {
                proposals.push(source_relationship_proposal(&source_ref, resolution));
            }
            ResolutionKind::PendingRetired => {
                proposals.push(pending_retired_proposal(
                    login_name,
                    label,
                    entry,
                    Some(resolution.id.clone()),
                    ProposalPolicyDecision::Auto,
                    ProposalReason {
                        field: "resolution".to_string(),
                        result: ProposalReasonResult::DerivedFromResolution,
                        detail: resolution.id.clone(),
                        weight: Some(ProposalReasonWeight::Exact),
                    },
                ));
            }
            _ => {}
        }
    }
    proposals
}

fn gl_proposals(
    ledger_dir: &Path,
) -> Result<Vec<AutomationProposal>, Box<dyn std::error::Error + Send + Sync>> {
    let suggestions = crate::categorize::suggest_gl_categories(ledger_dir)?;
    let mut proposals = Vec::new();
    for (txn_id, suggestion) in suggestions {
        // A transfer match means the Unknown txn is actually a transfer; it takes
        // priority over rule/ML recategorization (mirrors the post paths, which
        // check transferMatch before ruleAccount).
        if let Some(transfer) = suggestion.transfer_match {
            let refs = vec![gl_txn_ref(&txn_id), gl_txn_ref(&transfer.txn_id)];
            proposals.push(AutomationProposal {
                id: proposal_id("merge-gl-transfer", &refs, Some(&transfer.txn_id)),
                kind: AutomationProposalKind::MergeGlTransfer,
                subject_refs: refs,
                proposed_result: ProposalResult {
                    suggested_account: None,
                    transfer_match: None,
                    parts: Vec::new(),
                    import_anomaly_id: None,
                    resolution_id: None,
                    notes: Some(transfer.description),
                },
                reasons: vec![ProposalReason {
                    field: "amount/date".to_string(),
                    result: ProposalReasonResult::Matched,
                    detail: transfer.matched_amount,
                    weight: Some(ProposalReasonWeight::Strong),
                }],
                blockers: Vec::new(),
                can_apply: proposal_kind_is_applyable(&AutomationProposalKind::MergeGlTransfer),
                policy_decision: ProposalPolicyDecision::Review,
                reversible: ProposalReversibility::Conditional,
            });
            continue;
        }
        // A category rule gives a deterministic Auto recategorization; otherwise
        // the ML suggestion (previously dropped on the floor) becomes a Review
        // proposal, surfaced as a chip / bulk-accept in the Transactions tab.
        if let Some(account) = suggestion.rule_account {
            proposals.push(recategorize_gl_proposal(
                &txn_id,
                &account,
                ProposalReasonResult::DerivedFromResolution,
                ProposalReasonWeight::Exact,
                ProposalPolicyDecision::Auto,
            ));
        } else if let Some(account) = suggestion.suggested {
            proposals.push(recategorize_gl_proposal(
                &txn_id,
                &account,
                ProposalReasonResult::ModelSuggested,
                ProposalReasonWeight::Weak,
                ProposalPolicyDecision::Review,
            ));
        }
    }
    Ok(proposals)
}

/// Build a `RecategorizeGl` proposal replacing a txn's `Expenses:Unknown` posting
/// with `account`. The counterpart posting index is resolved at apply time (last
/// posting of the generated GL format — see `apply_proposal`).
fn recategorize_gl_proposal(
    txn_id: &str,
    account: &str,
    result: ProposalReasonResult,
    weight: ProposalReasonWeight,
    policy_decision: ProposalPolicyDecision,
) -> AutomationProposal {
    let refs = vec![gl_txn_ref(txn_id)];
    AutomationProposal {
        id: proposal_id("recategorize-gl", &refs, None),
        kind: AutomationProposalKind::RecategorizeGl,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: Some(account.to_string()),
            transfer_match: None,
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id: None,
            notes: None,
        },
        reasons: vec![ProposalReason {
            field: "category".to_string(),
            result,
            detail: account.to_string(),
            weight: Some(weight),
        }],
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::RecategorizeGl),
        policy_decision,
        reversible: ProposalReversibility::Conditional,
    }
}

fn import_anomaly_proposals(
    ledger_dir: &Path,
    login_name: Option<String>,
    label: Option<String>,
) -> io::Result<Vec<AutomationProposal>> {
    let anomalies = bookkeeping::list_import_anomalies(ledger_dir)?;
    let mut proposals = Vec::new();
    for anomaly in anomalies {
        if anomaly.status != bookkeeping::ImportAnomalyStatus::Open {
            continue;
        }
        if login_name
            .as_deref()
            .is_some_and(|value| value != anomaly.login_name)
        {
            continue;
        }
        if label.as_deref().is_some_and(|value| value != anomaly.label) {
            continue;
        }
        let source_ref = login_entry_ref(
            &anomaly.login_name,
            &anomaly.label,
            &anomaly.source_entry_id,
        );
        let mut refs = vec![source_ref];
        if let Some(gl_txn_id) = &anomaly.gl_txn_id {
            refs.push(gl_txn_ref(gl_txn_id));
        }
        let blockers = anomaly
            .safety_reasons
            .iter()
            .map(|detail| ProposalBlocker {
                code: "import-anomaly".to_string(),
                detail: detail.clone(),
            })
            .collect::<Vec<_>>();
        proposals.push(AutomationProposal {
            id: proposal_id("review-anomaly", &refs, Some(&anomaly.id)),
            kind: if anomaly.safe_to_retire {
                AutomationProposalKind::RetirePending
            } else {
                AutomationProposalKind::ReviewAnomaly
            },
            subject_refs: refs,
            proposed_result: ProposalResult {
                suggested_account: None,
                transfer_match: None,
                parts: Vec::new(),
                import_anomaly_id: Some(anomaly.id),
                resolution_id: None,
                notes: Some(anomaly.description),
            },
            reasons: vec![ProposalReason {
                field: "coverage".to_string(),
                result: ProposalReasonResult::CoveredByExport,
                detail: anomaly.coverage_document,
                weight: Some(ProposalReasonWeight::Strong),
            }],
            blockers: if anomaly.safe_to_retire {
                Vec::new()
            } else {
                blockers
            },
            can_apply: proposal_kind_is_applyable(if anomaly.safe_to_retire {
                &AutomationProposalKind::RetirePending
            } else {
                &AutomationProposalKind::ReviewAnomaly
            }),
            policy_decision: if anomaly.safe_to_retire {
                ProposalPolicyDecision::Auto
            } else {
                ProposalPolicyDecision::Review
            },
            reversible: ProposalReversibility::Conditional,
        });
    }
    Ok(proposals)
}

fn apply_proposal(
    ledger_dir: &Path,
    proposal: AutomationProposal,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match proposal.kind {
        AutomationProposalKind::PostCategory => {
            let Some((login_name, label, entry_id)) =
                proposal.subject_refs.iter().find_map(parse_login_entry_ref)
            else {
                return Err("post-category proposal missing source entry ref".into());
            };
            let Some(account) = proposal.proposed_result.suggested_account else {
                return Err("post-category proposal missing suggested account".into());
            };
            crate::post::post_login_account_entry(
                ledger_dir,
                &login_name,
                &label,
                &entry_id,
                &account,
                None,
                "automation",
            )
        }
        AutomationProposalKind::PostSplit => {
            let Some((login_name, label, entry_id)) =
                proposal.subject_refs.iter().find_map(parse_login_entry_ref)
            else {
                return Err("post-split proposal missing source entry ref".into());
            };
            let counterparts = proposal
                .proposed_result
                .parts
                .into_iter()
                .map(|part| {
                    let account = part
                        .account
                        .ok_or_else(|| "post-split part is missing account".to_string())?;
                    Ok(crate::post::SplitCounterpart {
                        account,
                        amount: part.amount,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            crate::post::post_login_account_entry_split(
                ledger_dir,
                &login_name,
                &label,
                &entry_id,
                counterparts,
                "automation",
            )
        }
        AutomationProposalKind::LinkTransfer => {
            let refs = proposal
                .subject_refs
                .iter()
                .filter_map(parse_login_entry_ref)
                .collect::<Vec<_>>();
            if refs.len() != 2 {
                return Err("link-transfer proposal must contain exactly two source refs".into());
            }
            crate::post::post_login_account_transfer(
                ledger_dir,
                &refs[0].0,
                &refs[0].1,
                &refs[0].2,
                &refs[1].0,
                &refs[1].1,
                &refs[1].2,
                "automation",
            )
        }
        AutomationProposalKind::RetirePending => {
            let Some((login_name, label, entry_id)) =
                proposal.subject_refs.iter().find_map(parse_login_entry_ref)
            else {
                return Err("retire-pending proposal missing source entry ref".into());
            };
            crate::post::retire_login_account_entry(
                ledger_dir,
                &login_name,
                &label,
                &entry_id,
                "automation proposal: retire pending",
                "automation",
            )?;
            if let Some(id) = proposal.proposed_result.import_anomaly_id {
                let _ = bookkeeping::review_import_anomaly(
                    ledger_dir,
                    bookkeeping::ReviewImportAnomalyInput {
                        id,
                        notes: Some("Applied retire-pending automation proposal".to_string()),
                    },
                );
            }
            Ok(entry_id)
        }
        AutomationProposalKind::SyncPosted => {
            let Some((login_name, label, entry_id)) =
                proposal.subject_refs.iter().find_map(parse_login_entry_ref)
            else {
                return Err("sync-posted proposal missing source entry ref".into());
            };
            crate::post::sync_gl_transaction(
                ledger_dir,
                &login_name,
                &label,
                &entry_id,
                "automation",
            )
        }
        AutomationProposalKind::MergeGlTransfer => {
            let refs = proposal
                .subject_refs
                .iter()
                .filter_map(parse_gl_txn_ref)
                .collect::<Vec<_>>();
            if refs.len() != 2 {
                return Err("merge-gl-transfer proposal must contain exactly two GL refs".into());
            }
            crate::post::merge_gl_transfer(ledger_dir, &refs[0], &refs[1], "automation")
        }
        AutomationProposalKind::RecategorizeGl => {
            let Some(txn_id) = proposal.subject_refs.iter().find_map(parse_gl_txn_ref) else {
                return Err("recategorize-gl proposal missing GL txn ref".into());
            };
            let Some(account) = proposal.proposed_result.suggested_account else {
                return Err("recategorize-gl proposal missing suggested account".into());
            };
            // The Expenses:Unknown counterpart is the last posting of the generated
            // GL format (mirrors categorize.rs "Counterpart is the last posting").
            let posting_index = crate::post::gl_txn_counterpart_posting_index(ledger_dir, &txn_id)?
                .ok_or_else(|| format!("recategorize-gl: transaction not found: {txn_id}"))?;
            crate::post::recategorize_gl_transaction(
                ledger_dir,
                &txn_id,
                posting_index,
                &account,
                "automation",
            )?;
            Ok(txn_id)
        }
        _ => Err(format!(
            "proposal kind is not directly applyable: {:?}",
            proposal.kind
        )
        .into()),
    }
}

fn proposal_is_applyable(proposal: &AutomationProposal) -> bool {
    proposal_kind_is_applyable(&proposal.kind)
}

fn proposal_kind_is_applyable(kind: &AutomationProposalKind) -> bool {
    matches!(
        kind,
        AutomationProposalKind::PostCategory
            | AutomationProposalKind::PostSplit
            | AutomationProposalKind::LinkTransfer
            | AutomationProposalKind::RetirePending
            | AutomationProposalKind::SyncPosted
            | AutomationProposalKind::MergeGlTransfer
            | AutomationProposalKind::RecategorizeGl
    )
}

fn post_split_proposal(
    login_name: &str,
    label: &str,
    entry: &AccountEntry,
    resolution: &Resolution,
) -> AutomationProposal {
    let refs = vec![login_entry_ref(login_name, label, &entry.id)];
    AutomationProposal {
        id: proposal_id("post-split", &refs, Some(&resolution.id)),
        kind: AutomationProposalKind::PostSplit,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: None,
            transfer_match: None,
            parts: resolution.parts.clone(),
            import_anomaly_id: None,
            resolution_id: Some(resolution.id.clone()),
            notes: Some(entry.description.clone()),
        },
        reasons: vec![ProposalReason {
            field: "resolution".to_string(),
            result: ProposalReasonResult::DerivedFromResolution,
            detail: resolution.id.clone(),
            weight: Some(ProposalReasonWeight::Exact),
        }],
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::PostSplit),
        policy_decision: ProposalPolicyDecision::Auto,
        reversible: ProposalReversibility::Conditional,
    }
}

fn pending_retired_proposal(
    login_name: &str,
    label: &str,
    entry: &AccountEntry,
    resolution_id: Option<String>,
    policy_decision: ProposalPolicyDecision,
    reason: ProposalReason,
) -> AutomationProposal {
    let refs = vec![login_entry_ref(login_name, label, &entry.id)];
    AutomationProposal {
        id: proposal_id("retire-pending", &refs, resolution_id.as_deref()),
        kind: AutomationProposalKind::RetirePending,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: None,
            transfer_match: None,
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id,
            notes: Some(entry.description.clone()),
        },
        reasons: vec![reason],
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::RetirePending),
        policy_decision,
        reversible: ProposalReversibility::Conditional,
    }
}

fn source_relationship_proposal(
    source_ref: &TypedRef,
    resolution: &Resolution,
) -> AutomationProposal {
    let kind = match resolution.kind {
        ResolutionKind::SameSource => AutomationProposalKind::MergeSource,
        ResolutionKind::NotSameSource => AutomationProposalKind::PreventMerge,
        _ => AutomationProposalKind::ReviewAnomaly,
    };
    let can_apply = proposal_kind_is_applyable(&kind);
    AutomationProposal {
        id: proposal_id(
            "source-relationship",
            &resolution.subject_refs,
            Some(&resolution.id),
        ),
        kind,
        subject_refs: resolution.subject_refs.clone(),
        proposed_result: ProposalResult {
            suggested_account: None,
            transfer_match: None,
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id: Some(resolution.id.clone()),
            notes: source_ref.entry_id.clone(),
        },
        reasons: vec![ProposalReason {
            field: "resolution".to_string(),
            result: ProposalReasonResult::DerivedFromResolution,
            detail: resolution.id.clone(),
            weight: Some(ProposalReasonWeight::Exact),
        }],
        blockers: Vec::new(),
        can_apply,
        policy_decision: ProposalPolicyDecision::Skip,
        reversible: ProposalReversibility::No,
    }
}

fn post_category_proposal(
    login_name: &str,
    label: &str,
    entry: &AccountEntry,
    account: &str,
    resolution_id: Option<String>,
    policy_decision: ProposalPolicyDecision,
    reason: ProposalReason,
) -> AutomationProposal {
    let refs = vec![login_entry_ref(login_name, label, &entry.id)];
    AutomationProposal {
        id: proposal_id("post-category", &refs, Some(account)),
        kind: AutomationProposalKind::PostCategory,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: Some(account.to_string()),
            transfer_match: None,
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id,
            notes: Some(entry.description.clone()),
        },
        reasons: vec![reason],
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::PostCategory),
        policy_decision,
        reversible: ProposalReversibility::Yes,
    }
}

struct LinkTransferProposalInput<'a> {
    login_name: &'a str,
    label: &'a str,
    entry: &'a AccountEntry,
    other_login: &'a str,
    other_label: &'a str,
    other_entry_id: &'a str,
    matched_amount: &'a str,
    resolution_id: Option<String>,
    policy_decision: ProposalPolicyDecision,
}

fn link_transfer_proposal(input: LinkTransferProposalInput<'_>) -> AutomationProposal {
    let refs = vec![
        login_entry_ref(input.login_name, input.label, &input.entry.id),
        login_entry_ref(input.other_login, input.other_label, input.other_entry_id),
    ];
    AutomationProposal {
        id: proposal_id("link-transfer", &refs, Some(input.other_entry_id)),
        kind: AutomationProposalKind::LinkTransfer,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: None,
            transfer_match: Some(ProposalTransferMatch {
                login_name: input.other_login.to_string(),
                label: input.other_label.to_string(),
                entry_id: input.other_entry_id.to_string(),
                matched_amount: input.matched_amount.to_string(),
            }),
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id: input.resolution_id,
            notes: Some(input.entry.description.clone()),
        },
        reasons: vec![ProposalReason {
            field: "transfer".to_string(),
            result: if input.policy_decision == ProposalPolicyDecision::Auto {
                ProposalReasonResult::DerivedFromResolution
            } else {
                ProposalReasonResult::Matched
            },
            detail: "opposite amount candidate".to_string(),
            weight: Some(ProposalReasonWeight::Strong),
        }],
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::LinkTransfer),
        policy_decision: input.policy_decision,
        reversible: ProposalReversibility::Conditional,
    }
}

fn sync_posted_proposal(
    login_name: &str,
    label: &str,
    entry: &AccountEntry,
    suggestion: &crate::categorize::CategoryResult,
) -> AutomationProposal {
    let refs = vec![login_entry_ref(login_name, label, &entry.id)];
    let mut reasons = Vec::new();
    if suggestion.amount_changed {
        reasons.push(ProposalReason {
            field: "amount".to_string(),
            result: ProposalReasonResult::Matched,
            detail: "source amount differs from generated GL".to_string(),
            weight: Some(ProposalReasonWeight::Strong),
        });
    }
    if suggestion.status_changed {
        reasons.push(ProposalReason {
            field: "status".to_string(),
            result: ProposalReasonResult::Matched,
            detail: "source status differs from generated GL".to_string(),
            weight: Some(ProposalReasonWeight::Strong),
        });
    }
    AutomationProposal {
        id: proposal_id("sync-posted", &refs, None),
        kind: AutomationProposalKind::SyncPosted,
        subject_refs: refs,
        proposed_result: ProposalResult {
            suggested_account: None,
            transfer_match: None,
            parts: Vec::new(),
            import_anomaly_id: None,
            resolution_id: None,
            notes: Some(entry.description.clone()),
        },
        reasons,
        blockers: Vec::new(),
        can_apply: proposal_kind_is_applyable(&AutomationProposalKind::SyncPosted),
        policy_decision: ProposalPolicyDecision::Review,
        reversible: ProposalReversibility::Conditional,
    }
}

struct ActiveResolutions<'a> {
    by_subject: BTreeMap<String, Vec<&'a Resolution>>,
}

impl<'a> ActiveResolutions<'a> {
    fn new(resolutions: &'a [Resolution]) -> Self {
        let mut by_subject: BTreeMap<String, Vec<&Resolution>> = BTreeMap::new();
        for resolution in resolutions
            .iter()
            .filter(|resolution| resolution.status == ResolutionStatus::Active)
        {
            for subject in &resolution.subject_refs {
                by_subject
                    .entry(ref_key(subject))
                    .or_default()
                    .push(resolution);
            }
        }
        Self { by_subject }
    }

    fn for_subject(&self, subject: &TypedRef) -> &[&'a Resolution] {
        self.by_subject
            .get(&ref_key(subject))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn has_ignore_source(&self, subject: &TypedRef) -> bool {
        self.for_subject(subject)
            .iter()
            .any(|resolution| resolution.kind == ResolutionKind::IgnoreSource)
    }

    fn entry_is_resolved(&self, subject: &TypedRef) -> bool {
        self.for_subject(subject).iter().any(|resolution| {
            matches!(
                resolution.kind,
                ResolutionKind::Category
                    | ResolutionKind::PostingSplit
                    | ResolutionKind::TransferLink
                    | ResolutionKind::TransferSplit
                    | ResolutionKind::IgnoreSource
                    | ResolutionKind::PendingRetired
            )
        })
    }
}

fn validate_resolution_input(input: &NewResolutionInput) -> io::Result<()> {
    // CategoryRule is the only kind allowed to be global (empty subject_refs); all
    // others act on specific entries and require a subject.
    if input.subject_refs.is_empty() && input.kind != ResolutionKind::CategoryRule {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "subjectRefs is required",
        ));
    }
    match input.kind {
        ResolutionKind::Category
        | ResolutionKind::PostingSplit
        | ResolutionKind::IgnoreSource
        | ResolutionKind::PendingRetired => {
            if input.subject_refs.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "this resolution kind requires exactly one subject ref",
                ));
            }
        }
        _ => {}
    }
    match input.kind {
        ResolutionKind::Category => {
            if input
                .parts
                .iter()
                .all(|part| part.account.as_deref().unwrap_or("").is_empty())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "category resolutions require at least one account part",
                ));
            }
        }
        ResolutionKind::PostingSplit => {
            let account_part_count = input
                .parts
                .iter()
                .filter(|part| !part.account.as_deref().unwrap_or("").is_empty())
                .count();
            if account_part_count < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "posting split resolutions require at least two account parts",
                ));
            }
        }
        ResolutionKind::IgnoreSource | ResolutionKind::PendingRetired => {
            if !input.parts.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "this resolution kind must not include parts",
                ));
            }
        }
        ResolutionKind::TransferLink
        | ResolutionKind::SameSource
        | ResolutionKind::NotSameSource
        | ResolutionKind::ReversalLink => {
            if input.subject_refs.len() != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "this resolution kind requires exactly two subject refs",
                ));
            }
            if !input.parts.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "this resolution kind must not include parts",
                ));
            }
        }
        ResolutionKind::TransferSplit => {
            if input.subject_refs.len() < 2 || input.parts.len() < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "transfer split resolutions require at least two subject refs and two parts",
                ));
            }
        }
        ResolutionKind::CategoryRule => validate_category_rule_input(input)?,
    }
    Ok(())
}

/// Validate the CategoryRule-specific shape (see the batch plan): a predicate with
/// at least one of description_regex/normalized_payee, a compilable case-insensitive
/// regex if present, exactly one non-empty account part, and subject_refs that are
/// EITHER empty (global) or a single login-scope ref (LoginEntry with a
/// `logins/<login>/accounts/<label>` locator and no entry_id).
fn validate_category_rule_input(input: &NewResolutionInput) -> io::Result<()> {
    let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg.to_string());

    let Some(predicate) = &input.predicate else {
        return Err(invalid("category rule resolutions require a predicate"));
    };
    if predicate.description_regex.is_none() && predicate.normalized_payee.is_none() {
        return Err(invalid(
            "category rule predicate requires descriptionRegex or normalizedPayee",
        ));
    }
    if let Some(pattern) = &predicate.description_regex {
        compile_rule_regex(pattern)
            .map_err(|err| invalid(&format!("category rule descriptionRegex is invalid: {err}")))?;
    }
    let account_part_count = input
        .parts
        .iter()
        .filter(|part| !part.account.as_deref().unwrap_or("").is_empty())
        .count();
    if account_part_count != 1 {
        return Err(invalid(
            "category rule resolutions require exactly one account part",
        ));
    }
    match input.subject_refs.as_slice() {
        [] => {}
        [scope] if is_login_scope_ref(scope) => {}
        _ => {
            return Err(invalid(
                "category rule subjectRefs must be empty (global) or a single login-scope ref",
            ));
        }
    }
    Ok(())
}

/// A login-account scope ref: LoginEntry with a `logins/<login>/accounts/<label>`
/// locator and NO entry_id (distinguishing it from an entry-bound ref).
fn is_login_scope_ref(value: &TypedRef) -> bool {
    value.kind == TypedRefKind::LoginEntry
        && value.entry_id.is_none()
        && value
            .locator
            .as_deref()
            .and_then(parse_login_account_locator)
            .is_some()
}

fn resolutions_dir(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join("bookkeeping").join(RESOLUTIONS_DIR)
}

fn resolution_path(ledger_dir: &Path, id: &str) -> PathBuf {
    resolutions_dir(ledger_dir).join(format!("{id}.json"))
}

fn login_entry_ref(login_name: &str, label: &str, entry_id: &str) -> TypedRef {
    TypedRef {
        kind: TypedRefKind::LoginEntry,
        id: None,
        locator: Some(format!("logins/{login_name}/accounts/{label}")),
        entry_id: Some(entry_id.to_string()),
        login_name: Some(login_name.to_string()),
        label: Some(label.to_string()),
        filename: None,
    }
}

fn gl_txn_ref(txn_id: &str) -> TypedRef {
    TypedRef {
        kind: TypedRefKind::GlTxn,
        id: Some(txn_id.to_string()),
        locator: None,
        entry_id: None,
        login_name: None,
        label: None,
        filename: None,
    }
}

fn parse_login_entry_ref(value: &TypedRef) -> Option<(String, String, String)> {
    if value.kind != TypedRefKind::LoginEntry {
        return None;
    }
    let entry_id = value.entry_id.clone()?;
    if let (Some(login_name), Some(label)) = (value.login_name.clone(), value.label.clone()) {
        return Some((login_name, label, entry_id));
    }
    let locator = value.locator.as_deref()?;
    let (login_name, label) = parse_login_account_locator(locator)?;
    Some((login_name, label, entry_id))
}

fn parse_login_account_locator(locator: &str) -> Option<(String, String)> {
    let rest = locator.strip_prefix("logins/")?;
    let (login_name, rest) = rest.split_once("/accounts/")?;
    Some((login_name.to_string(), rest.to_string()))
}

fn parse_gl_txn_ref(value: &TypedRef) -> Option<String> {
    if value.kind != TypedRefKind::GlTxn {
        return None;
    }
    value.id.clone()
}

fn ref_key(value: &TypedRef) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn proposal_id(kind: &str, refs: &[TypedRef], extra: Option<&str>) -> String {
    stable_id(&serde_json::to_string(&(kind, refs, extra.unwrap_or(""))).unwrap_or_default())
}

fn stable_id(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn require_non_empty<'a>(field_name: &str, value: &'a str) -> io::Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{field_name} is required"),
        ))
    } else {
        Ok(trimmed)
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn json_error(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(name: &str) -> io::Result<PathBuf> {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "refreshmint-automation-{}-{}",
            name,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn write_test_login_entry(
        root: &Path,
        login_name: &str,
        label: &str,
        entry_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut config = crate::login_config::LoginConfig {
            extension: Some("test".to_string()),
            accounts: BTreeMap::new(),
        };
        config.accounts.insert(
            label.to_string(),
            crate::login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        crate::login_config::write_login_config(root, login_name, &config)?;
        let journal_path = account_journal::login_account_journal_path(root, login_name, label);
        let journal_parent = journal_path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "journal path has no parent")
        })?;
        fs::create_dir_all(journal_parent)?;
        account_journal::write_journal_at_path(
            &journal_path,
            &[AccountEntry {
                id: entry_id.to_string(),
                date: "2026-04-01".to_string(),
                status: account_journal::EntryStatus::Cleared,
                description: "Cafe".to_string(),
                comment: String::new(),
                evidence: vec!["doc.csv:2:1".to_string()],
                postings: vec![account_journal::EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(account_journal::SimpleAmount {
                        quantity: "-12.50".to_string(),
                        commodity: "USD".to_string(),
                    }),
                }],
                tags: Vec::new(),
                extracted_by: None,
                posted: None,
                posted_postings: Vec::new(),
            }],
        )?;
        Ok(())
    }

    #[test]
    fn resolutions_round_trip_and_disable() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        let root = temp_dir("resolution")?;
        let created = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::Category,
                subject_refs: vec![login_entry_ref("bank", "checking", "entry-1")],
                parts: vec![ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Food".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: Some("merchant rule".to_string()),
                predicate: None,
            },
        )?;
        assert_eq!(list_resolutions(&root)?.len(), 1);
        let disabled = disable_resolution(&root, &created.id)?;
        assert_eq!(disabled.status, ResolutionStatus::Disabled);
        let enabled = enable_resolution(&root, &created.id)?;
        assert_eq!(enabled.status, ResolutionStatus::Active);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn commutative_resolutions_are_idempotent(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("resolution-idempotent")?;
        let left = login_entry_ref("bank", "checking", "entry-1");
        let right = login_entry_ref("card", "primary", "entry-2");
        let first = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::TransferLink,
                subject_refs: vec![left.clone(), right.clone()],
                parts: Vec::new(),
                notes: Some("first".to_string()),
                predicate: None,
            },
        )?;
        let second = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::TransferLink,
                subject_refs: vec![right, left],
                parts: Vec::new(),
                notes: Some("second".to_string()),
                predicate: None,
            },
        )?;
        let resolutions = list_resolutions(&root)?;

        assert_eq!(first.id, second.id);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].notes.as_deref(), Some("first"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn category_resolution_generates_auto_post_proposal(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("category-proposal")?;
        let login_name = "bank";
        let label = "checking";
        write_test_login_entry(&root, login_name, label, "entry-1")?;
        create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::Category,
                subject_refs: vec![login_entry_ref(login_name, label, "entry-1")],
                parts: vec![ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Dining".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: Some(login_name.to_string()),
                label: Some(label.to_string()),
                include_gl: Some(false),
            },
        )?;

        assert!(proposals.iter().any(|proposal| {
            proposal.kind == AutomationProposalKind::PostCategory
                && proposal.policy_decision == ProposalPolicyDecision::Auto
                && proposal.proposed_result.suggested_account.as_deref() == Some("Expenses:Dining")
        }));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn posting_split_resolution_generates_auto_post_split_proposal(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("split-proposal")?;
        let login_name = "bank";
        let label = "checking";
        write_test_login_entry(&root, login_name, label, "entry-1")?;
        create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::PostingSplit,
                subject_refs: vec![login_entry_ref(login_name, label, "entry-1")],
                parts: vec![
                    ResolutionPart {
                        amount: Some("5.00 USD".to_string()),
                        account: Some("Expenses:Coffee".to_string()),
                        ref_: None,
                        notes: None,
                    },
                    ResolutionPart {
                        amount: None,
                        account: Some("Expenses:Dining".to_string()),
                        ref_: None,
                        notes: None,
                    },
                ],
                notes: None,
                predicate: None,
            },
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: Some(login_name.to_string()),
                label: Some(label.to_string()),
                include_gl: Some(false),
            },
        )?;

        assert!(proposals.iter().any(|proposal| {
            proposal.kind == AutomationProposalKind::PostSplit
                && proposal.policy_decision == ProposalPolicyDecision::Auto
                && proposal.proposed_result.parts.len() == 2
        }));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn posting_split_resolution_requires_two_account_parts(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("split-validation")?;
        let result = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::PostingSplit,
                subject_refs: vec![login_entry_ref("bank", "checking", "entry-1")],
                parts: vec![ResolutionPart {
                    amount: Some("5.00 USD".to_string()),
                    account: Some("Expenses:Coffee".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => return Err("posting-split with one account part should fail".into()),
        };
        assert!(err.to_string().contains("at least two account parts"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn pending_retired_resolution_generates_auto_retire_proposal(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("pending-retired-proposal")?;
        let login_name = "bank";
        let label = "checking";
        write_test_login_entry(&root, login_name, label, "entry-1")?;
        let resolution = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::PendingRetired,
                subject_refs: vec![login_entry_ref(login_name, label, "entry-1")],
                parts: Vec::new(),
                notes: None,
                predicate: None,
            },
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: Some(login_name.to_string()),
                label: Some(label.to_string()),
                include_gl: Some(false),
            },
        )?;

        assert!(proposals.iter().any(|proposal| {
            proposal.kind == AutomationProposalKind::RetirePending
                && proposal.policy_decision == ProposalPolicyDecision::Auto
                && proposal.proposed_result.resolution_id.as_deref() == Some(&resolution.id)
        }));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn ignore_source_resolution_suppresses_entry_proposals(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("ignore-source-proposal")?;
        let login_name = "bank";
        let label = "checking";
        write_test_login_entry(&root, login_name, label, "entry-1")?;
        create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::Category,
                subject_refs: vec![login_entry_ref(login_name, label, "entry-1")],
                parts: vec![ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Dining".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        )?;
        create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::IgnoreSource,
                subject_refs: vec![login_entry_ref(login_name, label, "entry-1")],
                parts: Vec::new(),
                notes: None,
                predicate: None,
            },
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: Some(login_name.to_string()),
                label: Some(label.to_string()),
                include_gl: Some(false),
            },
        )?;

        assert!(proposals.is_empty());

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn generated_update_proposals_are_applyable() {
        let sync_refs = vec![login_entry_ref("bank", "checking", "entry-1")];
        let merge_refs = vec![gl_txn_ref("txn-1"), gl_txn_ref("txn-2")];
        let sync = AutomationProposal {
            id: proposal_id("sync-posted", &sync_refs, None),
            kind: AutomationProposalKind::SyncPosted,
            subject_refs: sync_refs,
            proposed_result: ProposalResult {
                suggested_account: None,
                transfer_match: None,
                parts: Vec::new(),
                import_anomaly_id: None,
                resolution_id: None,
                notes: None,
            },
            reasons: Vec::new(),
            blockers: Vec::new(),
            can_apply: proposal_kind_is_applyable(&AutomationProposalKind::SyncPosted),
            policy_decision: ProposalPolicyDecision::Review,
            reversible: ProposalReversibility::Conditional,
        };
        let merge = AutomationProposal {
            id: proposal_id("merge-gl-transfer", &merge_refs, None),
            kind: AutomationProposalKind::MergeGlTransfer,
            subject_refs: merge_refs,
            proposed_result: ProposalResult {
                suggested_account: None,
                transfer_match: None,
                parts: Vec::new(),
                import_anomaly_id: None,
                resolution_id: None,
                notes: None,
            },
            reasons: Vec::new(),
            blockers: Vec::new(),
            can_apply: proposal_kind_is_applyable(&AutomationProposalKind::MergeGlTransfer),
            policy_decision: ProposalPolicyDecision::Review,
            reversible: ProposalReversibility::Conditional,
        };

        assert!(proposal_is_applyable(&sync));
        assert!(proposal_is_applyable(&merge));
    }

    #[test]
    fn single_subject_resolutions_reject_extra_subjects_and_parts(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("single-subject-validation")?;
        let result = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::IgnoreSource,
                subject_refs: vec![
                    login_entry_ref("bank", "checking", "entry-1"),
                    login_entry_ref("bank", "checking", "entry-2"),
                ],
                parts: Vec::new(),
                notes: None,
                predicate: None,
            },
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => return Err("ignore-source with multiple subjects should fail".into()),
        };
        assert!(err.to_string().contains("exactly one subject ref"));

        let result = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::PendingRetired,
                subject_refs: vec![login_entry_ref("bank", "checking", "entry-1")],
                parts: vec![ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Dining".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => return Err("pending-retired with parts should fail".into()),
        };
        assert!(err.to_string().contains("must not include parts"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn relationship_resolutions_reject_parts(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("relationship-validation")?;
        let result = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::ReversalLink,
                subject_refs: vec![
                    login_entry_ref("bank", "checking", "entry-1"),
                    gl_txn_ref("txn-1"),
                ],
                parts: vec![ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Dining".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => return Err("reversal-link with parts should fail".into()),
        };
        assert!(err.to_string().contains("must not include parts"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn pairwise_relationship_resolutions_require_two_subjects(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("pairwise-relationship-validation")?;
        let result = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::TransferLink,
                subject_refs: vec![
                    login_entry_ref("bank", "checking", "entry-1"),
                    login_entry_ref("bank", "checking", "entry-2"),
                    login_entry_ref("bank", "checking", "entry-3"),
                ],
                parts: Vec::new(),
                notes: None,
                predicate: None,
            },
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => return Err("transfer-link with three subjects should fail".into()),
        };
        assert!(err.to_string().contains("exactly two subject refs"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn same_and_not_same_source_resolutions_are_visible_as_proposals(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("source-relationship");
        let root = root?;
        let login_name = "bank";
        let label = "checking";
        write_test_login_entry(&root, login_name, label, "entry-1")?;
        let entry_1 = login_entry_ref(login_name, label, "entry-1");
        let entry_2 = login_entry_ref(login_name, label, "entry-2");
        create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::NotSameSource,
                subject_refs: vec![entry_1, entry_2],
                parts: Vec::new(),
                notes: None,
                predicate: None,
            },
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: Some(login_name.to_string()),
                label: Some(label.to_string()),
                include_gl: Some(false),
            },
        )?;

        assert!(proposals
            .iter()
            .any(|proposal| proposal.kind == AutomationProposalKind::PreventMerge));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    fn account_part(account: &str) -> ResolutionPart {
        ResolutionPart {
            amount: None,
            account: Some(account.to_string()),
            ref_: None,
            notes: None,
        }
    }

    fn payee_predicate(normalized_payee: &str) -> CategoryRulePredicate {
        CategoryRulePredicate {
            description_regex: None,
            normalized_payee: Some(normalized_payee.to_string()),
            amount_min: None,
            amount_max: None,
        }
    }

    fn login_scope_ref(login_name: &str, label: &str) -> TypedRef {
        TypedRef {
            kind: TypedRefKind::LoginEntry,
            id: None,
            locator: Some(format!("logins/{login_name}/accounts/{label}")),
            entry_id: None,
            login_name: Some(login_name.to_string()),
            label: Some(label.to_string()),
            filename: None,
        }
    }

    fn category_rule(
        subject_refs: Vec<TypedRef>,
        predicate: CategoryRulePredicate,
        account: &str,
    ) -> NewResolutionInput {
        NewResolutionInput {
            kind: ResolutionKind::CategoryRule,
            subject_refs,
            parts: vec![account_part(account)],
            notes: None,
            predicate: Some(predicate),
        }
    }

    #[test]
    fn category_rule_validation() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("rule-validation")?;

        // Happy path: global rule with a payee predicate + one account.
        create_resolution(
            &root,
            category_rule(vec![], payee_predicate("STARBUCKS"), "Expenses:Coffee"),
        )?;

        // Missing predicate.
        let mut input = category_rule(vec![], payee_predicate("X"), "Expenses:X");
        input.predicate = None;
        assert!(create_resolution(&root, input).is_err());

        // Predicate with no descriptionRegex/normalizedPayee.
        assert!(create_resolution(
            &root,
            category_rule(
                vec![],
                CategoryRulePredicate {
                    description_regex: None,
                    normalized_payee: None,
                    amount_min: Some("1".to_string()),
                    amount_max: None,
                },
                "Expenses:X",
            ),
        )
        .is_err());

        // Uncompilable regex.
        assert!(create_resolution(
            &root,
            category_rule(
                vec![],
                CategoryRulePredicate {
                    description_regex: Some("(".to_string()),
                    normalized_payee: None,
                    amount_min: None,
                    amount_max: None,
                },
                "Expenses:X",
            ),
        )
        .is_err());

        // No account part.
        let mut no_account = category_rule(vec![], payee_predicate("X"), "Expenses:X");
        no_account.parts = Vec::new();
        assert!(create_resolution(&root, no_account).is_err());

        // Two account parts.
        let mut two_accounts = category_rule(vec![], payee_predicate("X"), "Expenses:X");
        two_accounts.parts.push(account_part("Expenses:Y"));
        assert!(create_resolution(&root, two_accounts).is_err());

        // Two subject refs (must be empty or exactly one scope ref).
        assert!(create_resolution(
            &root,
            category_rule(
                vec![
                    login_scope_ref("bank", "checking"),
                    login_scope_ref("bank", "savings")
                ],
                payee_predicate("X"),
                "Expenses:X",
            ),
        )
        .is_err());

        // Entry-bound ref (has entry_id) is not a valid scope ref.
        assert!(create_resolution(
            &root,
            category_rule(
                vec![login_entry_ref("bank", "checking", "entry-1")],
                payee_predicate("X"),
                "Expenses:X",
            ),
        )
        .is_err());

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn category_rule_fingerprint_includes_predicate(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("rule-fingerprint")?;
        // Same kind + parts + (global) subject, differing only in predicate.
        let first = create_resolution(
            &root,
            category_rule(vec![], payee_predicate("STARBUCKS"), "Expenses:Coffee"),
        )?;
        let second = create_resolution(
            &root,
            category_rule(vec![], payee_predicate("SAFEWAY"), "Expenses:Coffee"),
        )?;
        assert_ne!(first.id, second.id);
        assert_eq!(list_resolutions(&root)?.len(), 2);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn old_category_resolution_id_is_stable() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        let root = temp_dir("id-stability")?;
        let refs = vec![login_entry_ref("bank", "checking", "entry-1")];
        let parts = vec![account_part("Expenses:Food")];
        let created = create_resolution(
            &root,
            NewResolutionInput {
                kind: ResolutionKind::Category,
                subject_refs: refs.clone(),
                parts: parts.clone(),
                notes: None,
                predicate: None,
            },
        )?;
        // A predicate-less resolution must hash exactly the historical 3-tuple, so
        // adding the CategoryRule predicate field left existing ids untouched.
        let expected_fp = serde_json::to_string(&(
            &ResolutionKind::Category,
            canonical_subject_refs(&ResolutionKind::Category, &refs),
            &parts,
        ))?;
        assert_eq!(created.id, stable_id(&expected_fp));
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn matching_rule_account_first_match_wins() {
        let make = |id: &str, account: &str| Resolution {
            id: id.to_string(),
            kind: ResolutionKind::CategoryRule,
            status: ResolutionStatus::Active,
            subject_refs: Vec::new(),
            parts: vec![account_part(account)],
            notes: None,
            predicate: Some(payee_predicate("STARBUCKS")),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let rules = vec![make("a", "Expenses:First"), make("b", "Expenses:Second")];
        assert_eq!(
            matching_rule_account(&rules, "STARBUCKS", None).as_deref(),
            Some("Expenses:First")
        );
        assert_eq!(matching_rule_account(&rules, "PEETS", None), None);
    }

    #[test]
    fn active_category_rules_sorted_newest_first(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("rule-ordering")?;
        ensure_automation_layout(&root)?;
        // Write two matching global rules directly with controlled updated_at.
        for (id, ts, account) in [
            ("id-older", "2026-01-01T00:00:00Z", "Expenses:Old"),
            ("id-newer", "2026-06-01T00:00:00Z", "Expenses:New"),
        ] {
            let rule = Resolution {
                id: id.to_string(),
                kind: ResolutionKind::CategoryRule,
                status: ResolutionStatus::Active,
                subject_refs: Vec::new(),
                parts: vec![account_part(account)],
                notes: None,
                predicate: Some(payee_predicate("STARBUCKS")),
                created_at: ts.to_string(),
                updated_at: ts.to_string(),
            };
            fs::write(
                resolution_path(&root, id),
                serde_json::to_string_pretty(&rule)?,
            )?;
        }
        let rules = active_category_rules(&root, None, None)?;
        assert_eq!(
            matching_rule_account(&rules, "STARBUCKS", None).as_deref(),
            Some("Expenses:New")
        );
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn category_rules_scoped_vs_global() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("rule-scope")?;
        create_resolution(
            &root,
            category_rule(vec![], payee_predicate("SAFEWAY"), "Expenses:Global"),
        )?;
        create_resolution(
            &root,
            category_rule(
                vec![login_scope_ref("bank", "checking")],
                payee_predicate("SAFEWAY"),
                "Expenses:Scoped",
            ),
        )?;

        // Matching scope sees both rules.
        let in_scope = active_category_rules(&root, Some("bank"), Some("checking"))?;
        assert_eq!(in_scope.len(), 2);

        // A different scope sees only the global rule.
        let out_of_scope = active_category_rules(&root, Some("other"), Some("acct"))?;
        assert_eq!(out_of_scope.len(), 1);
        assert_eq!(
            matching_rule_account(&out_of_scope, "SAFEWAY", None).as_deref(),
            Some("Expenses:Global")
        );
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn predicate_matches_regex_payee_and_amount() {
        let regex = CategoryRulePredicate {
            description_regex: Some("star.*bucks".to_string()),
            normalized_payee: None,
            amount_min: None,
            amount_max: None,
        };
        assert!(predicate_matches(&regex, "STARBUCKS #123", None));
        assert!(!predicate_matches(&regex, "PEETS COFFEE", None));

        let payee = payee_predicate("COSTCO WHSE");
        assert!(predicate_matches(&payee, "COSTCO WHSE #0123", None));
        assert!(!predicate_matches(&payee, "TARGET #55", None));

        let bounded = CategoryRulePredicate {
            description_regex: None,
            normalized_payee: Some("GYM".to_string()),
            amount_min: Some("10".to_string()),
            amount_max: Some("20".to_string()),
        };
        assert!(predicate_matches(&bounded, "GYM", Some(15.0)));
        assert!(!predicate_matches(&bounded, "GYM", Some(25.0)));
        // Bound set but no amount available -> no match.
        assert!(!predicate_matches(&bounded, "GYM", None));
    }

    /// One generated-GL transaction block (Expenses:Unknown as the last posting).
    fn gl_block(id: &str, desc: &str, source_entry: &str, counterpart: &str) -> String {
        format!(
            "2026-01-01 {desc}  ; id: {id}\n    ; generated-by: refreshmint-post\n    \
             ; source: logins/chase/accounts/checking:{source_entry}\n    \
             Assets:Chase  -10.00 USD\n    {counterpart}\n\n"
        )
    }

    #[test]
    fn gl_rule_match_generates_auto_recategorize_proposal(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("gl-rule-auto")?;
        fs::write(
            root.join("general.journal"),
            gl_block("txn-1", "SAFEWAY #123", "e1", "Expenses:Unknown"),
        )?;
        create_resolution(
            &root,
            category_rule(vec![], payee_predicate("SAFEWAY"), "Expenses:Groceries"),
        )?;

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: None,
                label: None,
                include_gl: Some(true),
            },
        )?;
        assert!(proposals.iter().any(|proposal| {
            proposal.kind == AutomationProposalKind::RecategorizeGl
                && proposal.policy_decision == ProposalPolicyDecision::Auto
                && proposal.proposed_result.suggested_account.as_deref()
                    == Some("Expenses:Groceries")
        }));
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn gl_ml_suggestion_generates_review_recategorize_proposal(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("gl-ml-review")?;
        let mut journal = String::new();
        // Dense training so the classifier is confident (prob >= 0.5).
        for i in 0..20 {
            journal.push_str(&gl_block(
                &format!("real-{i}"),
                "SHELL OIL",
                &format!("r{i}"),
                "Expenses:Gas",
            ));
        }
        journal.push_str(&gl_block(
            "txn-unknown",
            "SHELL OIL",
            "u1",
            "Expenses:Unknown",
        ));
        fs::write(root.join("general.journal"), journal)?;
        // No rule -> the ML suggestion (previously dropped) becomes a Review proposal.

        let proposals = list_automation_proposals(
            &root,
            AutomationScope {
                login_name: None,
                label: None,
                include_gl: Some(true),
            },
        )?;
        assert!(proposals.iter().any(|proposal| {
            proposal.kind == AutomationProposalKind::RecategorizeGl
                && proposal.policy_decision == ProposalPolicyDecision::Review
                && proposal.proposed_result.suggested_account.is_some()
        }));
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn apply_automation_policy_drains_auto_recategorize(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = temp_dir("gl-policy")?;
        let mut journal = String::new();
        journal.push_str(&gl_block("txn-1", "SAFEWAY #1", "e1", "Expenses:Unknown"));
        journal.push_str(&gl_block("txn-2", "SAFEWAY #2", "e2", "Expenses:Unknown"));
        journal.push_str(&gl_block(
            "txn-3",
            "ZZUNKNOWNMERCHANT",
            "e3",
            "Expenses:Unknown",
        ));
        fs::write(root.join("general.journal"), journal)?;
        create_resolution(
            &root,
            category_rule(vec![], payee_predicate("SAFEWAY"), "Expenses:Groceries"),
        )?;

        let applied = apply_automation_policy(
            &root,
            AutomationScope {
                login_name: None,
                label: None,
                include_gl: Some(true),
            },
        )?;
        // Both SAFEWAY rows drain; the mystery row has no Auto proposal.
        assert_eq!(applied.len(), 2);

        let gl = fs::read_to_string(root.join("general.journal"))?;
        assert_eq!(gl.matches("Expenses:Groceries").count(), 2);
        // Policy only applies Auto proposals, so the unmatched merchant stays Unknown.
        assert!(gl.contains("Expenses:Unknown"));
        let _ = fs::remove_dir_all(root);
        Ok(())
    }
}
