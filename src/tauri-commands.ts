import { invoke } from '@tauri-apps/api/core';
import type { ScrapeLogEntry } from './scrapeLog.ts';

export interface LedgerView {
    path: string;
    accounts: AccountRow[];
    transactions: TransactionRow[];
    glAccountConflicts: GlAccountConflict[];
}

export interface GlAccountConflict {
    glAccount: string;
    entries: GlAccountConflictEntry[];
}

export interface GlAccountConflictEntry {
    loginName: string;
    label: string;
}

export interface AccountRow {
    name: string;
    totals: AmountTotal[] | null;
    unpostedCount: number;
}

export interface ReconciliationSession {
    id: string;
    glAccount: string;
    statementStartDate: string | null;
    statementEndDate: string;
    statementStartingBalance: string | null;
    statementEndingBalance: string;
    currency: string | null;
    status: 'draft' | 'finalized' | 'reopened';
    reconciledTxnIds: string[];
    notes: string | null;
    createdAt: string;
    updatedAt: string;
}

export interface NewReconciliationSessionInput {
    glAccount: string;
    statementStartDate: string | null;
    statementEndDate: string;
    statementStartingBalance: string | null;
    statementEndingBalance: string;
    currency: string | null;
    reconciledTxnIds: string[];
    notes: string | null;
}

export interface UpdateReconciliationSessionInput extends NewReconciliationSessionInput {
    id: string;
}

export type TypedRefKind =
    | 'gl-txn'
    | 'login-entry'
    | 'document'
    | 'evidence-row';

export interface TypedRef {
    kind: TypedRefKind;
    id?: string | null;
    locator?: string | null;
    entryId?: string | null;
    loginName?: string | null;
    label?: string | null;
    filename?: string | null;
}

export type LinkKind = 'evidence-link' | 'settlement-link' | 'source-link';

export interface LinkRecord {
    id: string;
    kind: LinkKind;
    leftRef: TypedRef;
    rightRef: TypedRef;
    amount: string | null;
    notes: string | null;
    createdAt: string;
    updatedAt: string;
}

export interface NewLinkRecordInput {
    kind: LinkKind;
    leftRef: TypedRef;
    rightRef: TypedRef;
    amount: string | null;
    notes: string | null;
}

export type PeriodCloseStatus = 'draft' | 'soft-closed' | 'reopened';

export interface PeriodClose {
    periodId: string;
    status: PeriodCloseStatus;
    closedAt: string | null;
    closedBy: string | null;
    notes: string | null;
    reconciliationSessionIds: string[];
    adjustmentTxnIds: string[];
}

export interface UpsertPeriodCloseInput {
    periodId: string;
    status: PeriodCloseStatus;
    closedBy: string | null;
    notes: string | null;
    reconciliationSessionIds: string[];
    adjustmentTxnIds: string[];
}

export type ResolutionKind =
    | 'same-source'
    | 'not-same-source'
    | 'category'
    | 'category-rule'
    | 'posting-split'
    | 'transfer-link'
    // Negative transfer memory: the two login-entry subjects are NOT a transfer of
    // each other. Mirrors Rust automation::ResolutionKind::NotTransferLink.
    | 'not-transfer-link'
    | 'transfer-split'
    | 'ignore-source'
    | 'pending-retired'
    | 'reversal-link';

export type ResolutionStatus = 'active' | 'disabled';

export interface ResolutionPart {
    amount?: string | null;
    account?: string | null;
    ref?: TypedRef | null;
    notes?: string | null;
}

// Predicate for a 'category-rule' Resolution. Mirrors the Rust
// automation::CategoryRulePredicate (src-tauri/src/automation.rs). A rule matches
// when every set field matches; at least one of descriptionRegex/normalizedPayee
// is required.
export interface CategoryRulePredicate {
    descriptionRegex?: string | null;
    normalizedPayee?: string | null;
    amountMin?: string | null;
    amountMax?: string | null;
}

export interface Resolution {
    id: string;
    kind: ResolutionKind;
    status: ResolutionStatus;
    subjectRefs: TypedRef[];
    parts: ResolutionPart[];
    notes?: string | null;
    predicate?: CategoryRulePredicate | null;
    createdAt: string;
    updatedAt: string;
}

export interface NewResolutionInput {
    kind: ResolutionKind;
    subjectRefs: TypedRef[];
    parts: ResolutionPart[];
    notes?: string | null;
    predicate?: CategoryRulePredicate | null;
}

export interface AutomationScope {
    loginName?: string | null;
    label?: string | null;
    includeGl?: boolean | null;
}

export type AutomationProposalKind =
    | 'merge-source'
    | 'prevent-merge'
    | 'retire-pending'
    | 'post-category'
    | 'post-split'
    | 'link-transfer'
    | 'merge-gl-transfer'
    | 'recategorize-gl'
    | 'sync-posted'
    | 'review-anomaly';

export type ProposalPolicyDecision = 'auto' | 'review' | 'blocked' | 'skip';

export type ProposalReversibility = 'yes' | 'conditional' | 'no';

export type ProposalReasonResult =
    | 'matched'
    | 'similar'
    | 'derived-from-resolution'
    | 'model-suggested'
    | 'covered-by-export'
    | 'blocked';

export type ProposalReasonWeight = 'exact' | 'strong' | 'weak';

export interface ProposalReason {
    field: string;
    result: ProposalReasonResult;
    detail: string;
    weight?: ProposalReasonWeight | null;
}

export interface ProposalBlocker {
    code: string;
    detail: string;
}

export interface ProposalTransferMatch {
    loginName: string;
    label: string;
    entryId: string;
    matchedAmount: string;
}

export interface ProposalResult {
    suggestedAccount?: string | null;
    transferMatch?: ProposalTransferMatch | null;
    parts: ResolutionPart[];
    importAnomalyId?: string | null;
    resolutionId?: string | null;
    notes?: string | null;
}

export interface AutomationProposal {
    id: string;
    kind: AutomationProposalKind;
    subjectRefs: TypedRef[];
    proposedResult: ProposalResult;
    reasons: ProposalReason[];
    blockers: ProposalBlocker[];
    canApply: boolean;
    policyDecision: ProposalPolicyDecision;
    reversible: ProposalReversibility;
}

export async function listResolutions(ledger: string): Promise<Resolution[]> {
    return invoke('list_resolutions', { ledger });
}

export async function createResolution(
    ledger: string,
    resolution: NewResolutionInput,
): Promise<Resolution> {
    return invoke('create_resolution', { ledger, resolution });
}

export async function disableResolution(
    ledger: string,
    id: string,
): Promise<Resolution> {
    return invoke('disable_resolution', { ledger, id });
}

export async function enableResolution(
    ledger: string,
    id: string,
): Promise<Resolution> {
    return invoke('enable_resolution', { ledger, id });
}

export async function listAutomationProposals(
    ledger: string,
    scope: AutomationScope,
): Promise<AutomationProposal[]> {
    return invoke('list_automation_proposals', { ledger, scope });
}

export async function applyAutomationProposal(
    ledger: string,
    proposalId: string,
): Promise<string> {
    return invoke('apply_automation_proposal', { ledger, proposalId });
}

export async function applyAutomationPolicy(
    ledger: string,
    scope: AutomationScope,
): Promise<string[]> {
    return invoke('apply_automation_policy', { ledger, scope });
}

export interface TransactionRow {
    id: string;
    date: string;
    description: string;
    descriptionRaw: string;
    comment: string;
    evidence: string[];
    accounts: string;
    totals: AmountTotal[] | null;
    postings: PostingRow[];
    bookkeeping: TransactionBookkeeping;
}

export interface TransactionBookkeeping {
    generated: boolean;
    reconciledSessionIds: string[];
    linkedRecordIds: string[];
    settlementLinkIds: string[];
    softClosedPeriodId: string | null;
}

export interface AmountTotal {
    commodity: string;
    mantissa: string;
    scale: number;
    style: AmountStyleHint | null;
}

export interface AmountStyleHint {
    side: 'L' | 'R';
    spaced: boolean;
}

export interface PostingRow {
    account: string;
    amount: string | null;
    comment: string;
    totals: AmountTotal[] | null;
}

/** The placeholder counterpart account used for uncategorized GL transactions. */
export const UNCATEGORIZED_GL_ACCOUNT = 'Expenses:Unknown';

/** Per-domain credential status returned by list/sync commands. */
export interface DomainSecretEntry {
    domain: string;
    hasUsername: boolean;
    hasPassword: boolean;
}

export interface SecretSyncResult {
    /** All domains declared in the extension manifest. */
    required: DomainSecretEntry[];
    /** Required domains missing a username. */
    missingUsername: string[];
    /** Required domains missing a password. */
    missingPassword: string[];
    /** Domains in the store that are not declared by the manifest. */
    extras: string[];
}

export interface MigratedAccount {
    accountName: string;
    loginName: string;
    label: string;
}

export interface MigrationOutcome {
    dryRun: boolean;
    migrated: MigratedAccount[];
    skipped: string[];
    warnings: string[];
}

export interface DocumentInfo {
    mimeType: string;
    originalUrl?: string;
    scrapedAt: string;
    extensionName: string;
    accountName?: string;
    loginName?: string;
    label?: string;
    scrapeSessionId: string;
    coverageEndDate: string;
    dateRangeStart?: string;
    dateRangeEnd?: string;
}

export interface DocumentWithInfo {
    filename: string;
    info: DocumentInfo | null;
}

export interface AccountJournalEntry {
    id: string;
    date: string;
    bankStatus: 'pending' | 'posted' | 'unknown';
    statusMarker: '' | '!' | '*';
    description: string;
    comment: string;
    evidence: string[];
    posted: string | null;
    isTransfer: boolean;
    /** Quantity of the first posting (no commodity symbol), e.g. "-21.32". */
    amount: string | null;
    /** All tags on the entry as [key, value] pairs. */
    tags: [string, string][];
}

export type LockMetadataResource =
    | { kind: 'login'; loginName: string }
    | { kind: 'gl' };

export interface LockMetadata {
    version: number;
    owner: string;
    purpose: string;
    startedAt: string;
    pid?: number | null;
    resource: LockMetadataResource;
}

export interface LockStatus {
    locked: boolean;
    metadata: LockMetadata | null;
}

export interface LockStatusSnapshot {
    gl: LockStatus;
    logins: Record<string, LockStatus>;
}

export interface LoginExtractionSupport {
    supported: boolean;
    reason:
        | 'missing-extension'
        | 'missing-extractor'
        | 'broken-extractor'
        | null;
}

export interface NewTransactionInput {
    date: string;
    description: string;
    comment: string | null;
    postings: NewPostingInput[];
}

export interface NewPostingInput {
    account: string;
    amount: string | null;
    comment: string | null;
}

export async function openLedger(ledger: string): Promise<LedgerView> {
    return invoke('open_ledger', { ledger });
}

export async function addTransaction(
    ledger: string,
    transaction: NewTransactionInput,
): Promise<LedgerView> {
    return invoke('add_transaction', { ledger, transaction });
}

export async function validateTransaction(
    ledger: string,
    transaction: NewTransactionInput,
): Promise<void> {
    await invoke('validate_transaction', { ledger, transaction });
}

export async function addTransactionText(
    ledger: string,
    transaction: string,
): Promise<LedgerView> {
    return invoke('add_transaction_text', { ledger, transaction });
}

export async function validateTransactionText(
    ledger: string,
    transaction: string,
): Promise<void> {
    await invoke('validate_transaction_text', { ledger, transaction });
}

export async function listReconciliationSessions(
    ledger: string,
): Promise<ReconciliationSession[]> {
    return invoke('list_reconciliation_sessions', { ledger });
}

export async function createReconciliationSession(
    ledger: string,
    session: NewReconciliationSessionInput,
): Promise<ReconciliationSession> {
    return invoke('create_reconciliation_session', { ledger, session });
}

export async function updateReconciliationSession(
    ledger: string,
    session: UpdateReconciliationSessionInput,
): Promise<ReconciliationSession> {
    return invoke('update_reconciliation_session', { ledger, session });
}

export async function finalizeReconciliationSession(
    ledger: string,
    id: string,
): Promise<ReconciliationSession> {
    return invoke('finalize_reconciliation_session', { ledger, id });
}

export async function reopenReconciliationSession(
    ledger: string,
    id: string,
): Promise<ReconciliationSession> {
    return invoke('reopen_reconciliation_session', { ledger, id });
}

export async function listBookkeepingLinks(
    ledger: string,
): Promise<LinkRecord[]> {
    return invoke('list_bookkeeping_links', { ledger });
}

export async function createBookkeepingLink(
    ledger: string,
    link: NewLinkRecordInput,
): Promise<LinkRecord> {
    return invoke('create_bookkeeping_link', { ledger, link });
}

export async function deleteBookkeepingLink(
    ledger: string,
    id: string,
): Promise<void> {
    await invoke('delete_bookkeeping_link', { ledger, id });
}

export async function listPeriodCloses(ledger: string): Promise<PeriodClose[]> {
    return invoke('list_period_closes', { ledger });
}

export async function upsertPeriodClose(
    ledger: string,
    periodClose: UpsertPeriodCloseInput,
): Promise<PeriodClose> {
    return invoke('upsert_period_close', { ledger, periodClose });
}

export async function reopenPeriodClose(
    ledger: string,
    periodId: string,
): Promise<PeriodClose> {
    return invoke('reopen_period_close', { ledger, periodId });
}

export async function listScrapeExtensions(ledger: string): Promise<string[]> {
    return invoke('list_scrape_extensions', { ledger });
}

export async function loadScrapeExtension(
    ledger: string,
    source: string,
    replace: boolean,
): Promise<string> {
    return invoke('load_scrape_extension', { ledger, source, replace });
}

export async function startScrapeDebugSession(
    ledger: string,
    account: string,
): Promise<string> {
    return startScrapeDebugSessionForLogin(ledger, account);
}

export async function startScrapeDebugSessionForLogin(
    ledger: string,
    loginName: string,
    headless = false,
): Promise<string> {
    return invoke('start_scrape_debug_session_for_login', {
        ledger,
        loginName,
        headless,
    });
}

export async function stopScrapeDebugSession(): Promise<void> {
    await invoke('stop_scrape_debug_session');
}

export async function getScrapeDebugSessionSocket(): Promise<string | null> {
    return invoke('get_scrape_debug_session_socket');
}

export async function startLockMetadataWatch(ledger: string): Promise<void> {
    await invoke('start_lock_metadata_watch', { ledger });
}

export async function stopLockMetadataWatch(): Promise<void> {
    await invoke('stop_lock_metadata_watch');
}

export async function getLockStatusSnapshot(
    ledger: string,
    loginNames: string[],
): Promise<LockStatusSnapshot> {
    return invoke('get_lock_status_snapshot', { ledger, loginNames });
}

export async function getLoginExtractionSupport(
    ledger: string,
    loginName: string,
): Promise<LoginExtractionSupport> {
    return invoke('get_login_extraction_support', { ledger, loginName });
}

export async function runScrape(
    ledger: string,
    account: string,
): Promise<void> {
    await runScrapeForLogin(ledger, account);
}

export async function listDocuments(
    ledger: string,
    accountName: string,
): Promise<DocumentWithInfo[]> {
    return invoke('list_documents', { ledger, accountName });
}

export async function listLoginAccountDocuments(
    ledger: string,
    loginName: string,
    label: string,
): Promise<DocumentWithInfo[]> {
    return invoke('list_login_account_documents', { ledger, loginName, label });
}

export async function readAttachmentDataUrl(
    ledger: string,
    filename: string,
): Promise<string> {
    return invoke('read_attachment_data_url', { ledger, filename });
}

export async function readLoginAccountDocumentRows(
    ledger: string,
    loginName: string,
    label: string,
    documentName: string,
): Promise<string[][]> {
    return invoke('read_login_account_document_rows', {
        ledger,
        loginName,
        label,
        documentName,
    });
}

export async function readLoginAccountDocumentText(
    ledger: string,
    loginName: string,
    label: string,
    documentName: string,
): Promise<string> {
    return invoke('read_login_account_document_text', {
        ledger,
        loginName,
        label,
        documentName,
    });
}

export async function runExtraction(
    ledger: string,
    accountName: string,
    documentNames: string[],
): Promise<number> {
    return invoke('run_extraction', {
        ledger,
        accountName,
        documentNames,
    });
}

export interface DocumentError {
    documentName: string;
    error: string;
    extractionAttempts: number;
}

export interface ExtractionCommandResult {
    newEntryCount: number;
    failedDocuments: DocumentError[];
}

export async function runLoginAccountExtraction(
    ledger: string,
    loginName: string,
    label: string,
    documentNames: string[],
): Promise<ExtractionCommandResult> {
    return invoke('run_login_account_extraction', {
        ledger,
        loginName,
        label,
        documentNames,
    });
}

export async function resetDocumentExtractionFailure(
    ledger: string,
    loginName: string,
    label: string,
    filename: string,
): Promise<void> {
    return invoke('reset_document_extraction_failure', {
        ledger,
        loginName,
        label,
        filename,
    });
}

export async function getAccountJournal(
    ledger: string,
    accountName: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_account_journal', { ledger, accountName });
}

export async function getLoginAccountJournal(
    ledger: string,
    loginName: string,
    label: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_login_account_journal', { ledger, loginName, label });
}

export async function getUnposted(
    ledger: string,
    accountName: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_unposted', { ledger, accountName });
}

export async function getLoginAccountUnposted(
    ledger: string,
    loginName: string,
    label: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_login_account_unposted', {
        ledger,
        loginName,
        label,
    });
}

export async function postLoginAccountEntry(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    counterpartAccount: string,
    postingIndex: number | null,
): Promise<string> {
    return invoke('post_login_account_entry', {
        ledger,
        loginName,
        label,
        entryId,
        counterpartAccount,
        postingIndex,
    });
}

export interface SplitCounterpart {
    account: string;
    amount: string | null;
}

export async function postLoginAccountEntrySplit(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    counterparts: SplitCounterpart[],
): Promise<string> {
    return invoke('post_login_account_entry_split', {
        ledger,
        loginName,
        label,
        entryId,
        counterparts,
    });
}

export async function unpostLoginAccountEntry(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    postingIndex: number | null,
): Promise<void> {
    await invoke('unpost_login_account_entry', {
        ledger,
        loginName,
        label,
        entryId,
        postingIndex,
    });
}

export interface UnpostedTransferResult {
    loginName: string;
    label: string;
    entry: AccountJournalEntry;
}

export async function getUnpostedEntriesForTransfer(
    ledger: string,
    excludeLogin: string,
    excludeLabel: string,
    sourceEntryId: string,
): Promise<UnpostedTransferResult[]> {
    return invoke('get_unposted_entries_for_transfer', {
        ledger,
        excludeLogin,
        excludeLabel,
        sourceEntryId,
    });
}

export async function postLoginAccountTransfer(
    ledger: string,
    loginName1: string,
    label1: string,
    entryId1: string,
    loginName2: string,
    label2: string,
    entryId2: string,
): Promise<string> {
    return invoke('post_login_account_transfer', {
        ledger,
        loginName1,
        label1,
        entryId1,
        loginName2,
        label2,
        entryId2,
    });
}

export async function syncGlTransaction(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
): Promise<string> {
    return invoke('sync_gl_transaction', { ledger, loginName, label, entryId });
}

export type ImportAnomalyKind =
    | 'finalized-missing-from-covered-export'
    | 'unsafe-pending-retirement'
    | 'duplicate-import-repair-skipped'
    | 'posted-leg-amount-drift'
    | 'coverage-info-missing';

export type ImportAnomalyStatus = 'open' | 'reviewed';

export interface ImportAnomaly {
    id: string;
    kind: ImportAnomalyKind;
    status: ImportAnomalyStatus;
    loginName: string;
    label: string;
    sourceEntryId: string;
    glTxnId?: string | null;
    date: string;
    amount?: string | null;
    description: string;
    evidence: string[];
    coverageDocument: string;
    safeToRetire: boolean;
    safetyReasons: string[];
    linkedReversalRef?: TypedRef | null;
    notes?: string | null;
    createdAt: string;
    updatedAt: string;
    reviewedAt?: string | null;
}

export async function listImportAnomalies(
    ledger: string,
): Promise<ImportAnomaly[]> {
    return invoke('list_import_anomalies', { ledger });
}

export async function reviewImportAnomaly(
    ledger: string,
    anomaly: { id: string; notes?: string | null },
): Promise<ImportAnomaly> {
    return invoke('review_import_anomaly', { ledger, anomaly });
}

export async function linkImportAnomalyReversal(
    ledger: string,
    anomaly: {
        id: string;
        reversalRef: TypedRef;
        notes?: string | null;
    },
): Promise<ImportAnomaly> {
    return invoke('link_import_anomaly_reversal', { ledger, anomaly });
}

export async function retireLoginAccountEntry(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    reason: string,
): Promise<void> {
    await invoke('retire_login_account_entry', {
        ledger,
        loginName,
        label,
        entryId,
        reason,
    });
}

export interface TransferMatch {
    accountLocator: string;
    entryId: string;
    matchedAmount: string;
}

export interface CategoryResult {
    /** Suggested counterpart account, or null if confidence < 0.5. */
    suggested: string | null;
    /** True if the entry's posting amount differs from the GL transaction. */
    amountChanged: boolean;
    /** True if the entry's status differs from the GL transaction. */
    statusChanged: boolean;
    /** Auto-detected transfer match, or null if none / ambiguous. */
    transferMatch: TransferMatch | null;
    /**
     * Near-miss transfer candidates: when 2+ candidates match, `transferMatch`
     * stays null and this carries all of them in date-proximity order; empty
     * when 0 or exactly 1 match. Mirrors Rust
     * categorize::CategoryResult::transfer_candidates.
     */
    transferCandidates: TransferMatch[];
    /**
     * Counterpart account of the first matching active CategoryRule, or null.
     * Independent of `suggested`; post paths prefer it over 'Expenses:Unknown'.
     * Mirrors Rust categorize::CategoryResult::rule_account.
     */
    ruleAccount: string | null;
}

export async function suggestCategories(
    ledger: string,
    loginName: string,
    label: string,
): Promise<Record<string, CategoryResult>> {
    return invoke('suggest_categories', { ledger, loginName, label });
}

export interface GlTransferMatch {
    /** GL transaction ID of the matched counterpart. */
    txnId: string;
    description: string;
    date: string;
    matchedAmount: string;
}

export interface GlCategoryResult {
    /** ML-suggested replacement for `Expenses:Unknown`, or null. */
    suggested: string | null;
    /** Auto-detected transfer pair among other Expenses:Unknown GL txns. */
    transferMatch: GlTransferMatch | null;
    /**
     * Near-miss transfer candidates: when 2+ candidates match, `transferMatch`
     * stays null and this carries all of them in date-proximity order; empty
     * when 0 or exactly 1 match. Mirrors Rust
     * categorize::GlCategoryResult::transfer_candidates.
     */
    transferCandidates: GlTransferMatch[];
    /**
     * Counterpart account of the first matching active CategoryRule, or null.
     * Mirrors Rust categorize::GlCategoryResult::rule_account.
     */
    ruleAccount: string | null;
}

export async function suggestGlCategories(
    ledger: string,
): Promise<Record<string, GlCategoryResult>> {
    return invoke('suggest_gl_categories', { ledger });
}

/**
 * Normalize a raw transaction description to a stable merchant key (uppercased,
 * boilerplate/processor prefixes and trailing store/phone/location noise
 * stripped). Used to preview + store a CategoryRule's normalizedPayee predicate.
 * Mirrors Rust payee_normalize::normalize_payee.
 */
export async function normalizePayee(description: string): Promise<string> {
    return invoke('normalize_payee', { description });
}

export async function recategorizeGlTransaction(
    ledger: string,
    txnId: string,
    postingIndex: number,
    newAccount: string,
): Promise<void> {
    await invoke('recategorize_gl_transaction', {
        ledger,
        txnId,
        postingIndex,
        newAccount,
    });
}

export interface RecategorizeEdit {
    txnId: string;
    postingIndex: number;
    newAccount: string;
}

/**
 * Recategorize multiple GL postings in a single backend read/write/commit.
 * Preferred over looping {@link recategorizeGlTransaction}, which rewrites
 * `general.journal` and commits once per row.
 */
export async function recategorizeGlTransactions(
    ledger: string,
    edits: RecategorizeEdit[],
): Promise<void> {
    await invoke('recategorize_gl_transactions', { ledger, edits });
}

export async function mergeGlTransfer(
    ledger: string,
    txnId1: string,
    txnId2: string,
): Promise<string> {
    return invoke('merge_gl_transfer', { ledger, txnId1, txnId2 });
}

/**
 * An account entry that claims to be posted to a GL transaction that no longer
 * exists. Mirrors `consistency::DanglingRef` in the backend.
 */
export interface DanglingRef {
    loginName: string;
    label: string;
    entryId: string;
    postingIndex: number | null;
    glTxnId: string;
}

/**
 * A refreshmint-generated GL transaction whose source entry does not reference
 * it back. Mirrors `consistency::OrphanedGlTxn` in the backend.
 */
export interface OrphanedGlTxn {
    glTxnId: string;
    sourceLocator: string;
    sourceEntryId: string;
    /** Posting index for a per-leg source; null for a whole-entry source. */
    postingIndex: number | null;
}

/** Mirrors `consistency::ConsistencyReport`. */
export interface ConsistencyReport {
    danglingRefs: DanglingRef[];
    orphanedGlTxns: OrphanedGlTxn[];
}

export function isConsistencyReportClean(report: ConsistencyReport): boolean {
    return (
        report.danglingRefs.length === 0 && report.orphanedGlTxns.length === 0
    );
}

/**
 * Scan the ledger for referential inconsistencies (dangling posted-refs and
 * orphaned GL transactions) left by an interrupted operation. Read-only.
 */
export async function checkLedgerConsistency(
    ledger: string,
): Promise<ConsistencyReport> {
    return invoke('check_ledger_consistency', { ledger });
}

/**
 * Auto-complete recoverable inconsistencies (re-link orphaned GL txns from their
 * source backref — the redo-log replay that makes GL-first posting atomic) and
 * return the residual report. Run on ledger open.
 */
export async function recoverLedgerConsistency(
    ledger: string,
): Promise<ConsistencyReport> {
    return invoke('recover_ledger_consistency', { ledger });
}

/** Clear a dangling posted-ref so the entry can be re-posted. */
export async function repairDanglingRef(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    postingIndex: number | null,
): Promise<void> {
    await invoke('repair_dangling_ref', {
        ledger,
        loginName,
        label,
        entryId,
        postingIndex,
    });
}

/** Remove an orphaned GL transaction so its source entry can be re-posted. */
export async function repairOrphanedGlTxn(
    ledger: string,
    glTxnId: string,
): Promise<void> {
    await invoke('repair_orphaned_gl_txn', { ledger, glTxnId });
}

export interface AccountConfig {
    extension?: string;
}

export interface LoginAccountConfig {
    glAccount?: string | null;
}

export interface LoginConfig {
    extension?: string;
    accounts: Record<string, LoginAccountConfig>;
}

export async function getAccountConfig(
    ledger: string,
    accountName: string,
): Promise<AccountConfig> {
    return invoke('get_account_config', { ledger, accountName });
}

export async function setAccountExtension(
    ledger: string,
    accountName: string,
    extension: string,
): Promise<void> {
    await invoke('set_account_extension', {
        ledger,
        accountName,
        extension,
    });
}

export async function listLogins(ledger: string): Promise<string[]> {
    return invoke('list_logins', { ledger });
}

export async function getLoginConfig(
    ledger: string,
    loginName: string,
): Promise<LoginConfig> {
    return invoke('get_login_config', { ledger, loginName });
}

export async function createLogin(
    ledger: string,
    loginName: string,
    extension: string,
): Promise<void> {
    await invoke('create_login', { ledger, loginName, extension });
}

export async function setLoginExtension(
    ledger: string,
    loginName: string,
    extension: string,
): Promise<void> {
    await invoke('set_login_extension', { ledger, loginName, extension });
}

export async function deleteLogin(
    ledger: string,
    loginName: string,
): Promise<void> {
    await invoke('delete_login', { ledger, loginName });
}

export async function setLoginAccount(
    ledger: string,
    loginName: string,
    label: string,
    glAccount: string | null,
): Promise<void> {
    await invoke('set_login_account', { ledger, loginName, label, glAccount });
}

export async function removeLoginAccount(
    ledger: string,
    loginName: string,
    label: string,
): Promise<void> {
    await deleteLoginAccount(ledger, loginName, label);
}

export async function deleteLoginAccount(
    ledger: string,
    loginName: string,
    label: string,
): Promise<void> {
    await invoke('delete_login_account', { ledger, loginName, label });
}

export async function listLoginSecrets(
    loginName: string,
): Promise<DomainSecretEntry[]> {
    return invoke('list_login_secrets', { loginName });
}

export async function syncLoginSecretsForExtension(
    ledger: string,
    loginName: string,
    extension: string,
): Promise<SecretSyncResult> {
    return invoke('sync_login_secrets_for_extension', {
        ledger,
        loginName,
        extension,
    });
}

/** Store username + password together (one biometric prompt on macOS). */
export async function setLoginCredentials(
    loginName: string,
    domain: string,
    username: string,
    password: string,
): Promise<void> {
    await invoke('set_login_credentials', {
        loginName,
        domain,
        username,
        password,
    });
}

/** Store only the username (no biometric prompt on macOS). */
export async function setLoginUsername(
    loginName: string,
    domain: string,
    username: string,
): Promise<void> {
    await invoke('set_login_username', { loginName, domain, username });
}

/** Store only the password (biometric prompt on macOS). */
export async function setLoginPassword(
    loginName: string,
    domain: string,
    password: string,
): Promise<void> {
    await invoke('set_login_password', { loginName, domain, password });
}

/** Delete all credentials for a domain. */
export async function removeLoginDomain(
    loginName: string,
    domain: string,
): Promise<void> {
    await invoke('remove_login_domain', { loginName, domain });
}

/** Read the username for a domain — no biometric prompt. */
export async function getLoginUsername(
    loginName: string,
    domain: string,
): Promise<string> {
    return invoke('get_login_username', { loginName, domain });
}

/** Migrate legacy keychain entries to the new per-domain scheme. */
export async function migrateLoginSecrets(
    loginName: string,
): Promise<string[]> {
    return invoke('migrate_login_secrets', { loginName });
}

export async function clearLoginProfile(
    ledger: string,
    loginName: string,
): Promise<void> {
    await invoke('clear_login_profile', { ledger, loginName });
}

export async function runScrapeForLogin(
    ledger: string,
    loginName: string,
    source: 'manual' | 'auto' = 'manual',
    headless = false,
): Promise<void> {
    await invoke('run_scrape_for_login', {
        ledger,
        loginName,
        source,
        headless,
    });
}

export async function getScrapeLog(
    ledger: string,
    loginName: string,
): Promise<ScrapeLogEntry[]> {
    return invoke('get_scrape_log', { ledger, loginName });
}

export async function migrateLedger(
    ledger: string,
    dryRun: boolean,
): Promise<MigrationOutcome> {
    return invoke('migrate_ledger', { ledger, dryRun });
}

export async function repairLoginAccountLabels(
    ledger: string,
    loginName: string,
): Promise<MigrationOutcome> {
    return invoke('repair_login_account_labels', { ledger, loginName });
}

export async function queryTransactions(
    ledger: string,
    query: string,
): Promise<TransactionRow[]> {
    return invoke<TransactionRow[]>('query_transactions', { ledger, query });
}

export async function queryReconciliationCandidates(
    ledger: string,
    glAccount: string,
    statementStartDate: string | null,
    statementEndDate: string,
): Promise<TransactionRow[]> {
    return invoke<TransactionRow[]>('query_reconciliation_candidates', {
        ledger,
        glAccount,
        statementStartDate,
        statementEndDate,
    });
}

export interface HledgerReportResult {
    rows: string[][]; // all rows including header; empty for text commands
    text: string | null; // plain text output for stats/activity
}

export async function runHledgerReport(
    ledger: string,
    command: string,
    args: string[],
): Promise<HledgerReportResult> {
    return invoke('run_hledger_report', { ledger, command, args });
}
