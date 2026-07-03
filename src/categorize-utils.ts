import type { GlCategoryResult, TransactionRow } from './tauri-commands';

/** One accepted ML suggestion, ready for recategorizeGlTransactions. */
export interface AcceptAllEdit {
    txnId: string;
    postingIndex: number;
    oldAccount: string;
    newAccount: string;
}

/**
 * Build recategorize edits for every visible transaction that both (a) is
 * eligible — has exactly one recategorizable counterpart posting, per
 * `eligibility` — and (b) has a non-empty ML `suggested` account. Non-eligible
 * rows and rows without a suggestion are skipped.
 *
 * Pure so it can be vitest-tested; the "Accept N suggestions" button feeds the
 * result to recategorizeGlTransactions in one batch.
 */
export function buildAcceptAllEdits(
    transactions: TransactionRow[],
    suggestions: Record<string, GlCategoryResult>,
    eligibility: (
        txn: TransactionRow,
    ) => { posting: { account: string }; postingIndex: number } | null,
): AcceptAllEdit[] {
    const edits: AcceptAllEdit[] = [];
    for (const txn of transactions) {
        const suggested = suggestions[txn.id]?.suggested;
        if (suggested == null || suggested === '') continue;
        const match = eligibility(txn);
        if (match === null) continue;
        edits.push({
            txnId: txn.id,
            postingIndex: match.postingIndex,
            oldAccount: match.posting.account,
            newAccount: suggested,
        });
    }
    return edits;
}
