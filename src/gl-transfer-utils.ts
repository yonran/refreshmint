import type { TransactionRow } from './tauri-commands.ts';
import { UNCATEGORIZED_GL_ACCOUNT } from './tauri-commands.ts';

export type BookkeepingFilter =
    | 'all'
    | 'reconciled'
    | 'linked'
    | 'settled'
    | 'softClosed'
    | 'generated';

export function hasStagingPosting(txn: TransactionRow): boolean {
    return txn.postings.some(
        (posting) =>
            posting.account.startsWith('Equity:Staging') ||
            posting.account.startsWith('Equity:Unreconciled'),
    );
}

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

/**
 * Two amounts cancel below this epsilon (cents tolerance). Mirrors the Rust
 * matchers' TRANSFER_CANCEL_EPSILON (src-tauri/src/post.rs).
 */
const TRANSFER_CANCEL_EPSILON = 0.005;

/** Pre-filter window for the no-search transfer candidate ranking, in days. */
const RANK_DATE_WINDOW_DAYS = 14;

/** Parse a "<quantity> <commodity>" posting amount string. */
function parsePostingAmount(
    amount: string | null,
): { value: number; commodity: string } | null {
    if (amount === null) return null;
    const [quantity, commodity = ''] = amount.trim().split(/\s+/);
    const value = Number(quantity);
    return Number.isFinite(value) ? { value, commodity } : null;
}

/**
 * The signed amount of a GL row's first explicit non-`Expenses:Unknown`
 * posting. Mirrors the Rust GL transfer-candidate shape
 * (categorize::build_gl_transfer_candidates).
 */
function explicitPostingAmount(
    txn: TransactionRow,
): { value: number; commodity: string } | null {
    for (const posting of txn.postings) {
        if (posting.account === UNCATEGORIZED_GL_ACCOUNT) continue;
        const parsed = parsePostingAmount(posting.amount);
        if (parsed !== null) return parsed;
    }
    return null;
}

/** Absolute day distance between two YYYY-MM-DD dates; null if unparseable. */
function dayDistance(a: string, b: string): number | null {
    const ta = Date.parse(a);
    const tb = Date.parse(b);
    if (Number.isNaN(ta) || Number.isNaN(tb)) return null;
    return Math.abs(ta - tb) / (24 * 60 * 60 * 1000);
}

/**
 * Rank the Link Transfer modal's candidate list for `subject`.
 *
 * With an empty search: pre-filter to opposite-sign cancelling amounts (same
 * commodity, |a+b| < 0.005) within ±14 days, sorted by date proximity — the
 * likely counterparts float to the top instead of listing all history.
 * With search text: keep `filterGlTransferCandidates`'s filtering (text /
 * date / `amt:`), but apply the same date-proximity sort.
 */
export function rankGlTransferCandidates(
    transactions: TransactionRow[],
    subject: TransactionRow,
    search: string,
): TransactionRow[] {
    const filtered = filterGlTransferCandidates(
        transactions,
        subject.id,
        search,
    );
    const subjectAmount = explicitPostingAmount(subject);
    const prefiltered =
        search.trim() === ''
            ? filtered.filter((t) => {
                  if (subjectAmount === null) return false;
                  const candidateAmount = explicitPostingAmount(t);
                  if (candidateAmount === null) return false;
                  const distance = dayDistance(subject.date, t.date);
                  return (
                      candidateAmount.commodity === subjectAmount.commodity &&
                      Math.abs(candidateAmount.value + subjectAmount.value) <
                          TRANSFER_CANCEL_EPSILON &&
                      distance !== null &&
                      distance <= RANK_DATE_WINDOW_DAYS
                  );
              })
            : filtered;
    return prefiltered
        .map((t, index) => ({
            t,
            index,
            distance:
                dayDistance(subject.date, t.date) ?? Number.POSITIVE_INFINITY,
        }))
        .sort((a, b) => a.distance - b.distance || a.index - b.index)
        .map(({ t }) => t);
}

/**
 * The fee residual left when linking `candidate` as `subject`'s transfer
 * counterpart: `-(a1 + a2)` of their explicit posting amounts. `null` when the
 * legs cancel (no fee prompt needed) or when either amount is missing or the
 * commodities differ (the backend merge guard reports those). Drives the fee
 * prompt in the Link Transfer modals; mirrors Rust
 * post::transfer_fee_residual.
 */
export function glTransferResidual(
    subject: TransactionRow,
    candidate: TransactionRow,
): { residual: number; commodity: string } | null {
    const a = explicitPostingAmount(subject);
    const b = explicitPostingAmount(candidate);
    if (a === null || b === null || a.commodity !== b.commodity) return null;
    const residual = -(a.value + b.value);
    if (Math.abs(residual) < TRANSFER_CANCEL_EPSILON) return null;
    return { residual, commodity: a.commodity };
}

export function filterTransactionsByBookkeepingState(
    transactions: TransactionRow[],
    filter: BookkeepingFilter,
): TransactionRow[] {
    if (filter === 'all') {
        return transactions;
    }
    return transactions.filter((txn) => {
        if (filter === 'reconciled') {
            return txn.bookkeeping.reconciledSessionIds.length > 0;
        }
        if (filter === 'linked') {
            return txn.bookkeeping.linkedRecordIds.length > 0;
        }
        if (filter === 'settled') {
            return txn.bookkeeping.settlementLinkIds.length > 0;
        }
        if (filter === 'softClosed') {
            return txn.bookkeeping.softClosedPeriodId !== null;
        }
        return txn.bookkeeping.generated;
    });
}
