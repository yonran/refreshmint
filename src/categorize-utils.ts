import type {
    CategoryResult,
    GlCategoryResult,
    TransactionRow,
} from './tauri-commands';
import { UNCATEGORIZED_GL_ACCOUNT } from './tauri-commands';

/** Max description length shown in a merge-transfer chip before truncation. */
const MERGE_CHIP_DESC_MAX = 40;

/**
 * Label for the one-click "merge as transfer" chip. Names the action and the
 * counterpart (date + description), truncating a long description so the chip
 * stays a reasonable width. Replaces the previous bare "↔ {date} {desc}" glyph.
 */
export function mergeTransferChipLabel({
    date,
    description,
}: {
    date: string;
    description: string;
}): string {
    const truncated =
        description.length > MERGE_CHIP_DESC_MAX
            ? `${description.slice(0, MERGE_CHIP_DESC_MAX)}…`
            : description;
    return `Merge as transfer: ${date} ${truncated}`;
}

/**
 * Label for the one-click categorize chip, naming the destination account.
 * Replaces the previous bare "{account}" chip text.
 */
export function categorizeChipLabel(account: string): string {
    return `Categorize as ${account}`;
}

/**
 * Label for a Pipeline per-entry Post button, naming where the entry will land:
 * a transfer, its matching CategoryRule account, or the uncategorized account.
 * Mirrors doPipelinePostForAccount's destination logic so the button and the
 * post agree. Replaces the bare "Post".
 */
export function postButtonLabel(
    suggestion:
        | Pick<CategoryResult, 'transferMatch' | 'ruleAccount'>
        | undefined,
): string {
    if (suggestion?.transferMatch) return 'Post as transfer';
    return `Post to ${suggestion?.ruleAccount ?? UNCATEGORIZED_GL_ACCOUNT}`;
}

/**
 * One pre-merge source account entry, enough to re-post it to Expenses:Unknown
 * when undoing a chip merge. Captured at action time from each source GL txn's
 * comment (parseGlSourceRefs + parseLoginAccountLocator, src/evidence-nav-utils.ts).
 */
export interface MergeSourceRef {
    loginName: string;
    label: string;
    entryId: string;
}

/**
 * A completed one-click chip action, enough to compute its exact inverse for an
 * Undo toast. Kept in this pure module so the inverse logic is unit-testable.
 */
export type ChipAction =
    | {
          kind: 'categorize';
          txnId: string;
          postingIndex: number;
          oldAccount: string;
          newAccount: string;
      }
    | { kind: 'merge'; glTxnId: string; sourceRefs: MergeSourceRef[] };

/** The reversing operation for a {@link ChipAction}. */
export type UndoPlan =
    | {
          kind: 'recategorize-back';
          txnId: string;
          postingIndex: number;
          account: string;
      }
    // Unpost the merged transfer WITHOUT recording not-a-transfer negative
    // memory (recordMemory: false), so an undone merge stays re-suggestable,
    // then re-post each source entry to Expenses:Unknown so the two pre-merge
    // rows reappear (with new txn ids).
    | {
          kind: 'unpost-no-memory';
          glTxnId: string;
          sourceRefs: MergeSourceRef[];
      };

/**
 * Build the exact inverse of a just-performed chip action:
 * - categorize → recategorize the same posting back to its old account;
 * - merge → unpost the new transfer without negative memory, then re-post the
 *   captured source entries.
 */
export function buildUndoPlan(action: ChipAction): UndoPlan {
    switch (action.kind) {
        case 'categorize':
            return {
                kind: 'recategorize-back',
                txnId: action.txnId,
                postingIndex: action.postingIndex,
                account: action.oldAccount,
            };
        case 'merge':
            return {
                kind: 'unpost-no-memory',
                glTxnId: action.glTxnId,
                sourceRefs: action.sourceRefs,
            };
    }
}

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
