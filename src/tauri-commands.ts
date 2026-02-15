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
    totals: AmountTotal[] | null;
}

export async function openLedger(ledger: string): Promise<LedgerView> {
    return invoke('open_ledger', { ledger });
}
