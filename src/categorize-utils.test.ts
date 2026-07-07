import { describe, it, expect } from 'vitest';
import {
    buildAcceptAllEdits,
    buildUndoPlan,
    categorizeChipLabel,
    mergeTransferChipLabel,
    postButtonLabel,
    summarizeBulkRecategorize,
} from './categorize-utils.ts';
import { UNCATEGORIZED_GL_ACCOUNT } from './tauri-commands.ts';
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
    return {
        suggested,
        transferMatch: null,
        transferCandidates: [],
        ruleAccount: null,
    };
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

describe('summarizeBulkRecategorize', () => {
    it('groups by oldAccount with counts', () => {
        expect(
            summarizeBulkRecategorize([
                { oldAccount: 'Expenses:A' },
                { oldAccount: 'Expenses:B' },
                { oldAccount: 'Expenses:A' },
            ]),
        ).toEqual([
            { oldAccount: 'Expenses:A', count: 2 },
            { oldAccount: 'Expenses:B', count: 1 },
        ]);
    });

    it('preserves first-seen order of accounts', () => {
        expect(
            summarizeBulkRecategorize([
                { oldAccount: 'Expenses:Z' },
                { oldAccount: 'Expenses:A' },
                { oldAccount: 'Expenses:Z' },
            ]),
        ).toEqual([
            { oldAccount: 'Expenses:Z', count: 2 },
            { oldAccount: 'Expenses:A', count: 1 },
        ]);
    });

    it('returns an empty array for empty input', () => {
        expect(summarizeBulkRecategorize([])).toEqual([]);
    });
});

describe('categorizeChipLabel', () => {
    it('names the destination account', () => {
        expect(categorizeChipLabel('Expenses:Groceries')).toBe(
            'Categorize as Expenses:Groceries',
        );
    });
});

describe('mergeTransferChipLabel', () => {
    it('prefixes the date and description with an action verb', () => {
        expect(
            mergeTransferChipLabel({
                date: '2026-07-01',
                description: 'ACH PAYMENT',
            }),
        ).toBe('Merge as transfer: 2026-07-01 ACH PAYMENT');
    });

    it('truncates a long description to 40 chars with an ellipsis', () => {
        const long = 'A'.repeat(50);
        expect(
            mergeTransferChipLabel({ date: '2026-07-01', description: long }),
        ).toBe(`Merge as transfer: 2026-07-01 ${'A'.repeat(40)}…`);
    });

    it('leaves a 40-char description unchanged', () => {
        const exact = 'B'.repeat(40);
        expect(
            mergeTransferChipLabel({ date: '2026-07-01', description: exact }),
        ).toBe(`Merge as transfer: 2026-07-01 ${exact}`);
    });
});

describe('buildUndoPlan', () => {
    it('inverts a categorize by recategorizing back to the old account', () => {
        expect(
            buildUndoPlan({
                kind: 'categorize',
                txnId: 't1',
                postingIndex: 1,
                oldAccount: 'Expenses:Unknown',
                newAccount: 'Expenses:Groceries',
            }),
        ).toEqual({
            kind: 'recategorize-back',
            txnId: 't1',
            postingIndex: 1,
            account: 'Expenses:Unknown',
        });
    });

    it('inverts a merge by unposting without recording negative memory', () => {
        expect(buildUndoPlan({ kind: 'merge', glTxnId: 'gl-9' })).toEqual({
            kind: 'unpost-no-memory',
            glTxnId: 'gl-9',
        });
    });
});

describe('postButtonLabel', () => {
    const base = {
        suggested: null,
        amountChanged: false,
        statusChanged: false,
        transferCandidates: [],
    };

    it('names a transfer post when a transfer match exists', () => {
        expect(
            postButtonLabel({
                ...base,
                transferMatch: {
                    accountLocator: 'logins/boa/accounts/savings',
                    entryId: 'e2',
                    matchedAmount: '100.00',
                },
                ruleAccount: null,
            }),
        ).toBe('Post as transfer');
    });

    it('names the rule account when one matches', () => {
        expect(
            postButtonLabel({
                ...base,
                transferMatch: null,
                ruleAccount: 'Expenses:Groceries',
            }),
        ).toBe('Post to Expenses:Groceries');
    });

    it('falls back to the uncategorized account', () => {
        expect(
            postButtonLabel({
                ...base,
                transferMatch: null,
                ruleAccount: null,
            }),
        ).toBe(`Post to ${UNCATEGORIZED_GL_ACCOUNT}`);
        expect(postButtonLabel(undefined)).toBe(
            `Post to ${UNCATEGORIZED_GL_ACCOUNT}`,
        );
    });
});
