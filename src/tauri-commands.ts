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
