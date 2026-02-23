import { invoke } from '@tauri-apps/api/core';

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
    unreconciledCount: number;
}

export interface TransactionRow {
    id: string;
    date: string;
    description: string;
    descriptionRaw: string;
    comment: string;
    accounts: string;
    totals: AmountTotal[] | null;
    postings: PostingRow[];
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

export interface SecretEntry {
    domain: string;
    name: string;
}

export interface AccountSecretEntry extends SecretEntry {
    hasValue: boolean;
}

export interface SecretSyncResult {
    required: SecretEntry[];
    added: SecretEntry[];
    existingRequired: SecretEntry[];
    extras: SecretEntry[];
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
    status: 'cleared' | 'pending' | 'unmarked';
    description: string;
    comment: string;
    evidence: string[];
    reconciled: string | null;
    isTransfer: boolean;
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

export async function listAccountSecrets(
    account: string,
): Promise<AccountSecretEntry[]> {
    return invoke('list_account_secrets', { account });
}

export async function syncAccountSecretsForExtension(
    ledger: string,
    account: string,
    extension: string,
): Promise<SecretSyncResult> {
    return invoke('sync_account_secrets_for_extension', {
        ledger,
        account,
        extension,
    });
}

export async function addAccountSecret(
    account: string,
    domain: string,
    name: string,
    value: string,
): Promise<void> {
    await invoke('add_account_secret', { account, domain, name, value });
}

export async function reenterAccountSecret(
    account: string,
    domain: string,
    name: string,
    value: string,
): Promise<void> {
    await invoke('reenter_account_secret', { account, domain, name, value });
}

export async function removeAccountSecret(
    account: string,
    domain: string,
    name: string,
): Promise<void> {
    await invoke('remove_account_secret', { account, domain, name });
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
): Promise<string> {
    return invoke('start_scrape_debug_session_for_login', {
        ledger,
        loginName,
    });
}

export async function stopScrapeDebugSession(): Promise<void> {
    await invoke('stop_scrape_debug_session');
}

export async function getScrapeDebugSessionSocket(): Promise<string | null> {
    return invoke('get_scrape_debug_session_socket');
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

export async function runLoginAccountExtraction(
    ledger: string,
    loginName: string,
    label: string,
    documentNames: string[],
): Promise<number> {
    return invoke('run_login_account_extraction', {
        ledger,
        loginName,
        label,
        documentNames,
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

export async function getUnreconciled(
    ledger: string,
    accountName: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_unreconciled', { ledger, accountName });
}

export async function getLoginAccountUnreconciled(
    ledger: string,
    loginName: string,
    label: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_login_account_unreconciled', {
        ledger,
        loginName,
        label,
    });
}

export async function reconcileEntry(
    ledger: string,
    accountName: string,
    entryId: string,
    counterpartAccount: string,
    postingIndex: number | null,
): Promise<string> {
    return invoke('reconcile_entry', {
        ledger,
        accountName,
        entryId,
        counterpartAccount,
        postingIndex,
    });
}

export async function reconcileLoginAccountEntry(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    counterpartAccount: string,
    postingIndex: number | null,
): Promise<string> {
    return invoke('reconcile_login_account_entry', {
        ledger,
        loginName,
        label,
        entryId,
        counterpartAccount,
        postingIndex,
    });
}

export async function unreconcileEntry(
    ledger: string,
    accountName: string,
    entryId: string,
    postingIndex: number | null,
): Promise<void> {
    await invoke('unreconcile_entry', {
        ledger,
        accountName,
        entryId,
        postingIndex,
    });
}

export async function unreconcileLoginAccountEntry(
    ledger: string,
    loginName: string,
    label: string,
    entryId: string,
    postingIndex: number | null,
): Promise<void> {
    await invoke('unreconcile_login_account_entry', {
        ledger,
        loginName,
        label,
        entryId,
        postingIndex,
    });
}

export async function reconcileTransfer(
    ledger: string,
    account1: string,
    entryId1: string,
    account2: string,
    entryId2: string,
): Promise<string> {
    return invoke('reconcile_transfer', {
        ledger,
        account1,
        entryId1,
        account2,
        entryId2,
    });
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
): Promise<AccountSecretEntry[]> {
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

export async function addLoginSecret(
    loginName: string,
    domain: string,
    name: string,
    value: string,
): Promise<void> {
    await invoke('add_login_secret', { loginName, domain, name, value });
}

export async function reenterLoginSecret(
    loginName: string,
    domain: string,
    name: string,
    value: string,
): Promise<void> {
    await invoke('reenter_login_secret', { loginName, domain, name, value });
}

export async function removeLoginSecret(
    loginName: string,
    domain: string,
    name: string,
): Promise<void> {
    await invoke('remove_login_secret', { loginName, domain, name });
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
): Promise<void> {
    await invoke('run_scrape_for_login', { ledger, loginName });
}

export async function migrateLedger(
    ledger: string,
    dryRun: boolean,
): Promise<MigrationOutcome> {
    return invoke('migrate_ledger', { ledger, dryRun });
}
