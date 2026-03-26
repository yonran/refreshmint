import type { TransactionRow } from './tauri-commands.ts';

/**
 * Returns the subset of `transactions` that are valid candidates for manually
 * linking as the transfer counterpart of the transaction identified by
 * `currentTxnId`.
 *
 * A candidate must:
 * - Not be the current transaction itself.
 * - Have been posted by refreshmint (i.e. have `generated-by: refreshmint-post`
 *   in its transaction comment), so that `merge_gl_transfer` can find its
 *   source account-journal entry.
 *
 * The optional `search` string narrows the results:
 * - `amt:<value>` matches transactions that have a posting whose amount
 *   contains `<value>` (e.g. `amt:77.31`).
 * - Anything else is a case-insensitive substring match against description
 *   or date.
 */
export function filterGlTransferCandidates(
    transactions: TransactionRow[],
    currentTxnId: string,
    search: string,
): TransactionRow[] {
    const q = search.trim().toLowerCase();

    const isRefreshmintPosted = (t: TransactionRow) =>
        t.comment.includes('generated-by: refreshmint-post');

    const amtPrefix = 'amt:';
    const amtSearch = q.startsWith(amtPrefix)
        ? q.slice(amtPrefix.length)
        : null;

    return transactions
        .filter((t) => t.id !== currentTxnId && isRefreshmintPosted(t))
        .filter((t) => {
            if (!q) return true;
            if (amtSearch !== null) {
                return t.postings.some((p) =>
                    (p.amount ?? '').includes(amtSearch),
                );
            }
            return (
                t.description.toLowerCase().includes(q) || t.date.includes(q)
            );
        });
}
