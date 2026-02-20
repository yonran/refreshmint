import { invoke } from '@tauri-apps/api/core';

export interface LedgerView {
    path: string;
    accounts: AccountRow[];
    transactions: TransactionRow[];
}

export interface AccountRow {
    name: string;
    totals: AmountTotal[] | null;
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

export interface DocumentInfo {
    mimeType: string;
    originalUrl?: string;
    scrapedAt: string;
    extensionName: string;
    accountName: string;
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
): Promise<SecretEntry[]> {
    return invoke('list_account_secrets', { account });
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
    extension: string,
): Promise<string> {
    return invoke('start_scrape_debug_session', { ledger, account, extension });
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
    extension: string,
): Promise<void> {
    await invoke('run_scrape', { ledger, account, extension });
}

export async function listDocuments(
    ledger: string,
    accountName: string,
): Promise<DocumentWithInfo[]> {
    return invoke('list_documents', { ledger, accountName });
}

export async function runExtraction(
    ledger: string,
    accountName: string,
    extensionName: string,
    documentNames: string[],
): Promise<number> {
    return invoke('run_extraction', {
        ledger,
        accountName,
        extensionName,
        documentNames,
    });
}

export async function getAccountJournal(
    ledger: string,
    accountName: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_account_journal', { ledger, accountName });
}

export async function getUnreconciled(
    ledger: string,
    accountName: string,
): Promise<AccountJournalEntry[]> {
    return invoke('get_unreconciled', { ledger, accountName });
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
