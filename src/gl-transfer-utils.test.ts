import { describe, it, expect } from 'vitest';
import {
    filterGlTransferCandidates,
    filterReconciliationCandidates,
    filterTransactionsByBookkeepingState,
} from './gl-transfer-utils.ts';
import type { TransactionRow } from './tauri-commands.ts';

function makeTxn(
    id: string,
    overrides: Partial<TransactionRow> = {},
): TransactionRow {
    return {
        id,
        date: '2026-01-01',
        description: 'Test',
        descriptionRaw: 'Test',
        comment: '',
        evidence: [],
        accounts: '',
        totals: null,
        postings: [],
        bookkeeping: {
            generated: false,
            reconciledSessionIds: [],
            linkedRecordIds: [],
            settlementLinkIds: [],
            softClosedPeriodId: null,
        },
        ...overrides,
    };
}

const REFRESHMINT_COMMENT =
    '    ; generated-by: refreshmint-post\n    ; source: logins/bank/accounts/checking:abc123';

describe('filterGlTransferCandidates', () => {
    it('excludes the current transaction', () => {
        const txn = makeTxn('current', { comment: REFRESHMINT_COMMENT });
        expect(filterGlTransferCandidates([txn], 'current', '')).toEqual([]);
    });

    it('excludes transactions not posted by refreshmint', () => {
        const txn = makeTxn('other', { comment: '' });
        expect(filterGlTransferCandidates([txn], 'current', '')).toEqual([]);
    });

    it('includes refreshmint-posted transaction with Expenses:Unknown counterpart', () => {
        const txn = makeTxn('other', {
            comment: REFRESHMINT_COMMENT,
            postings: [
                {
                    account: 'Liabilities:CreditCard',
                    amount: '77.31 USD',
                    comment: '',
                    totals: null,
                },
                {
                    account: 'Expenses:Unknown',
                    amount: null,
                    comment: '',
                    totals: null,
                },
            ],
        });
        expect(filterGlTransferCandidates([txn], 'current', '')).toHaveLength(
            1,
        );
    });

    it('includes refreshmint-posted transaction with already-categorized counterpart', () => {
        // This is the 77.31 case: PAYMENT CITI AUTOPAY has Expenses:CreditCards, not Unknown
        const txn = makeTxn('checking-payment', {
            comment: REFRESHMINT_COMMENT,
            postings: [
                {
                    account: 'Assets:Provident:Checking',
                    amount: '-77.31 USD',
                    comment: '',
                    totals: null,
                },
                {
                    account: 'Expenses:CreditCards',
                    amount: null,
                    comment: '',
                    totals: null,
                },
            ],
        });
        expect(filterGlTransferCandidates([txn], 'current', '')).toHaveLength(
            1,
        );
    });

    it('amt: search matches a posting amount', () => {
        const txn = makeTxn('other', {
            comment: REFRESHMINT_COMMENT,
            postings: [
                {
                    account: 'Assets:Checking',
                    amount: '-77.31 USD',
                    comment: '',
                    totals: null,
                },
                {
                    account: 'Expenses:CreditCards',
                    amount: null,
                    comment: '',
                    totals: null,
                },
            ],
        });
        expect(
            filterGlTransferCandidates([txn], 'current', 'amt:77.31'),
        ).toHaveLength(1);
    });

    it('amt: search does not match a different amount', () => {
        const txn = makeTxn('other', {
            comment: REFRESHMINT_COMMENT,
            postings: [
                {
                    account: 'Assets:Checking',
                    amount: '-100.00 USD',
                    comment: '',
                    totals: null,
                },
            ],
        });
        expect(
            filterGlTransferCandidates([txn], 'current', 'amt:77.31'),
        ).toHaveLength(0);
    });

    it('plain text search filters by description', () => {
        const match = makeTxn('a', {
            comment: REFRESHMINT_COMMENT,
            description: 'AUTOPAY PAYMENT',
        });
        const noMatch = makeTxn('b', {
            comment: REFRESHMINT_COMMENT,
            description: 'AMAZON PURCHASE',
        });
        const result = filterGlTransferCandidates(
            [match, noMatch],
            'current',
            'autopay',
        );
        expect(result).toHaveLength(1);
        expect(result[0]?.id).toBe('a');
    });

    it('plain text search filters by date', () => {
        const match = makeTxn('a', {
            comment: REFRESHMINT_COMMENT,
            date: '2026-02-10',
        });
        const noMatch = makeTxn('b', {
            comment: REFRESHMINT_COMMENT,
            date: '2026-03-15',
        });
        const result = filterGlTransferCandidates(
            [match, noMatch],
            'current',
            '2026-02-10',
        );
        expect(result).toHaveLength(1);
        expect(result[0]?.id).toBe('a');
    });
});

describe('filterTransactionsByBookkeepingState', () => {
    it('filters reconciled transactions', () => {
        const rows = [
            makeTxn('a', {
                bookkeeping: {
                    generated: false,
                    reconciledSessionIds: ['rec-1'],
                    linkedRecordIds: [],
                    settlementLinkIds: [],
                    softClosedPeriodId: null,
                },
            }),
            makeTxn('b'),
        ];
        expect(
            filterTransactionsByBookkeepingState(rows, 'reconciled'),
        ).toEqual([rows[0]]);
    });

    it('filters settled transactions separately from generic links', () => {
        const linked = makeTxn('linked', {
            bookkeeping: {
                generated: false,
                reconciledSessionIds: [],
                linkedRecordIds: ['link-1'],
                settlementLinkIds: [],
                softClosedPeriodId: null,
            },
        });
        const settled = makeTxn('settled', {
            bookkeeping: {
                generated: false,
                reconciledSessionIds: [],
                linkedRecordIds: ['link-2'],
                settlementLinkIds: ['link-2'],
                softClosedPeriodId: null,
            },
        });
        expect(
            filterTransactionsByBookkeepingState([linked, settled], 'linked'),
        ).toEqual([linked, settled]);
        expect(
            filterTransactionsByBookkeepingState([linked, settled], 'settled'),
        ).toEqual([settled]);
    });
});

describe('filterReconciliationCandidates', () => {
    it('filters to transactions touching the chosen account', () => {
        const rows = [
            makeTxn('a', {
                postings: [
                    {
                        account: 'Assets:Checking',
                        amount: '-10.00 USD',
                        comment: '',
                        totals: null,
                    },
                ],
            }),
            makeTxn('b', {
                postings: [
                    {
                        account: 'Expenses:Food',
                        amount: '10.00 USD',
                        comment: '',
                        totals: null,
                    },
                ],
            }),
        ];
        expect(
            filterReconciliationCandidates(rows, 'Assets:Checking', '', '').map(
                (txn) => txn.id,
            ),
        ).toEqual(['a']);
    });

    it('filters by inclusive statement date bounds', () => {
        const rows = [
            makeTxn('a', {
                date: '2026-03-01',
                postings: [
                    {
                        account: 'Assets:Checking',
                        amount: '-10.00 USD',
                        comment: '',
                        totals: null,
                    },
                ],
            }),
            makeTxn('b', {
                date: '2026-03-31',
                postings: [
                    {
                        account: 'Assets:Checking',
                        amount: '-20.00 USD',
                        comment: '',
                        totals: null,
                    },
                ],
            }),
            makeTxn('c', {
                date: '2026-04-01',
                postings: [
                    {
                        account: 'Assets:Checking',
                        amount: '-30.00 USD',
                        comment: '',
                        totals: null,
                    },
                ],
            }),
        ];
        expect(
            filterReconciliationCandidates(
                rows,
                'Assets:Checking',
                '2026-03-01',
                '2026-03-31',
            ).map((txn) => txn.id),
        ).toEqual(['a', 'b']);
    });
});
