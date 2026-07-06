import type { GlCategoryResult, TransactionRow } from './tauri-commands';

/** A per-old-account grouping for the bulk-recategorize confirm modal. */
export interface BulkRecategorizeGroup {
    oldAccount: string;
    count: number;
}

/**
 * Group bulk-recategorize entries by their current (old) account, keeping the
 * first-seen order of accounts and counting how many entries fall under each.
 * Pure so the shared confirm modal and its callers stay behavior-identical.
 */
export function summarizeBulkRecategorize(
    entries: { oldAccount: string }[],
): BulkRecategorizeGroup[] {
    const counts = new Map<string, number>();
    for (const entry of entries) {
        counts.set(entry.oldAccount, (counts.get(entry.oldAccount) ?? 0) + 1);
    }
    return [...counts.entries()].map(([oldAccount, count]) => ({
        oldAccount,
        count,
    }));
}

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
