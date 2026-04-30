use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::account_journal::{self, AccountEntry};
use crate::bookkeeping::{self, TypedRef, TypedRefKind};

const RESOLUTIONS_DIR: &str = "resolutions";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub id: String,
    pub kind: ResolutionKind,
    pub status: ResolutionStatus,
    pub subject_refs: Vec<TypedRef>,
    pub parts: Vec<ResolutionPart>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionKind {
    SameSource,
    NotSameSource,
    Category,
    PostingSplit,
    TransferLink,
    TransferSplit,
    IgnoreSource,
    PendingRetired,
    ReversalLink,
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
    SplitTransfer,
    MergeGlTransfer,
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
    let id = stable_id(
        &serde_json::to_string(&(&input.kind, &input.subject_refs, &input.parts))
            .map_err(json_error)?,
    );
    let now = crate::operations::now_timestamp();
    let path = resolution_path(ledger_dir, &id);
    let created_at = if path.exists() {
        let existing: Resolution =
            serde_json::from_str(&fs::read_to_string(&path)?).map_err(json_error)?;
        existing.created_at
    } else {
        now.clone()
    };
    let resolution = Resolution {
        id,
        kind: input.kind,
        status: ResolutionStatus::Active,
        subject_refs: input.subject_refs,
        parts: input.parts,
        notes: normalize_optional(input.notes),
        created_at,
        updated_at: now,
    };
    fs::write(
        resolution_path(ledger_dir, &resolution.id),
        serde_json::to_string_pretty(&resolution).map_err(json_error)?,
    )?;
    Ok(resolution)
}

pub fn disable_resolution(ledger_dir: &Path, id: &str) -> io::Result<Resolution> {
    let id = require_non_empty("id", id)?;
    let path = resolution_path(ledger_dir, id);
    let mut resolution: Resolution =
        serde_json::from_str(&fs::read_to_string(&path)?).map_err(json_error)?;
    resolution.status = ResolutionStatus::Disabled;
    resolution.updated_at = crate::operations::now_timestamp();
    fs::write(
        &path,
        serde_json::to_string_pretty(&resolution).map_err(json_error)?,
    )?;
    Ok(resolution)
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
                policy_decision: ProposalPolicyDecision::Review,
                reversible: ProposalReversibility::Conditional,
            });
        }
    }
    Ok(proposals)
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
        _ => Err(format!(
            "proposal kind is not directly applyable: {:?}",
            proposal.kind
        )
        .into()),
    }
}

fn proposal_is_applyable(proposal: &AutomationProposal) -> bool {
    matches!(
        proposal.kind,
        AutomationProposalKind::PostCategory
            | AutomationProposalKind::PostSplit
            | AutomationProposalKind::LinkTransfer
            | AutomationProposalKind::RetirePending
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
        policy_decision: ProposalPolicyDecision::Auto,
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
            )
        })
    }
}

fn validate_resolution_input(input: &NewResolutionInput) -> io::Result<()> {
    if input.subject_refs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "subjectRefs is required",
        ));
    }
    match input.kind {
        ResolutionKind::Category | ResolutionKind::PostingSplit => {
            if input
                .parts
                .iter()
                .all(|part| part.account.as_deref().unwrap_or("").is_empty())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "category/posting split resolutions require at least one account part",
                ));
            }
        }
        ResolutionKind::TransferLink
        | ResolutionKind::SameSource
        | ResolutionKind::NotSameSource => {
            if input.subject_refs.len() < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "this resolution kind requires at least two subject refs",
                ));
            }
        }
        _ => {}
    }
    Ok(())
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
            },
        )?;
        assert_eq!(list_resolutions(&root)?.len(), 1);
        let disabled = disable_resolution(&root, &created.id)?;
        assert_eq!(disabled.status, ResolutionStatus::Disabled);
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
}
