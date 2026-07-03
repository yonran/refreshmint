import { describe, it, expect } from 'vitest';
import { buildAcceptAllEdits } from './categorize-utils.ts';
import type {
    GlCategoryResult,
    PostingRow,
    TransactionRow,
} from './tauri-commands.ts';

function posting(account: string, amount: string): PostingRow {
    return { account, amount, comment: '', totals: null };
}

function txn(id: string, postings: PostingRow[]): TransactionRow {
    return {
        id,
        date: '2026-01-01',
        description: id,
        descriptionRaw: id,
        comment: '',
        evidence: [],
        accounts: '',
        totals: null,
        postings,
        bookkeeping: {
            generated: false,
            reconciledSessionIds: [],
            linkedRecordIds: [],
            settlementLinkIds: [],
            softClosedPeriodId: null,
        },
    };
}

function simpleTxn(id: string, counterpart: string): TransactionRow {
    return txn(id, [
        posting('Assets:Checking', '-10.00'),
        posting(counterpart, '10.00'),
    ]);
}

// Mirror TransactionsTable.singleNonBalancingPostingWithIndex: exactly one
// non-Assets/Liabilities posting is eligible.
function eligibility(t: TransactionRow) {
    const candidates = t.postings
        .map((posting, postingIndex) => ({ posting, postingIndex }))
        .filter(
            ({ posting }) =>
                !posting.account.startsWith('Assets:') &&
                !posting.account.startsWith('Liabilities:'),
        );
    return candidates.length === 1 ? (candidates[0] ?? null) : null;
}

function suggestion(suggested: string | null): GlCategoryResult {
    return { suggested, transferMatch: null, ruleAccount: null };
}

describe('buildAcceptAllEdits', () => {
    it('builds edits for rows with a suggestion, targeting the counterpart index', () => {
        const rows = [
            simpleTxn('a', 'Expenses:Unknown'),
            simpleTxn('b', 'Expenses:Unknown'),
        ];
        const edits = buildAcceptAllEdits(
            rows,
            {
                a: suggestion('Expenses:Groceries'),
                b: suggestion('Expenses:Dining'),
            },
            eligibility,
        );
        expect(edits).toEqual([
            {
                txnId: 'a',
                postingIndex: 1,
                oldAccount: 'Expenses:Unknown',
                newAccount: 'Expenses:Groceries',
            },
            {
                txnId: 'b',
                postingIndex: 1,
                oldAccount: 'Expenses:Unknown',
                newAccount: 'Expenses:Dining',
            },
        ]);
    });

    it('skips rows without a suggestion', () => {
        const rows = [
            simpleTxn('a', 'Expenses:Unknown'),
            simpleTxn('b', 'Expenses:Unknown'),
        ];
        const edits = buildAcceptAllEdits(
            rows,
            { a: suggestion('Expenses:Groceries'), b: suggestion(null) },
            eligibility,
        );
        expect(edits.map((e) => e.txnId)).toEqual(['a']);
    });

    it('skips non-eligible rows (more than one counterpart posting)', () => {
        const messy = txn('c', [
            posting('Expenses:Unknown', '5'),
            posting('Expenses:Fees', '5'),
            posting('Assets:Checking', '-10'),
        ]);
        const edits = buildAcceptAllEdits(
            [messy],
            { c: suggestion('Expenses:Groceries') },
            eligibility,
        );
        expect(edits).toHaveLength(0);
    });

    it('returns no edits when no suggestions exist', () => {
        const rows = [simpleTxn('a', 'Expenses:Unknown')];
        expect(buildAcceptAllEdits(rows, {}, eligibility)).toHaveLength(0);
    });
});
