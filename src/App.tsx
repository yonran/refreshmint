import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { documentDir, join } from '@tauri-apps/api/path';
import {
    open as openDialog,
    save as saveDialog,
} from '@tauri-apps/plugin-dialog';
import './App.css';
import {
    type AmountStyleHint,
    type AmountTotal,
    addTransaction,
    addTransactionText,
    openLedger,
    type AccountRow,
    type LedgerView,
    type NewTransactionInput,
    type PostingRow,
    type TransactionRow,
    validateTransaction,
    validateTransactionText,
} from './tauri-commands.ts';

type TransactionDraft = {
    date: string;
    description: string;
    comment: string;
    postings: DraftPosting[];
};

type DraftPosting = {
    account: string;
    amount: string;
    comment: string;
};

type TransactionEntryMode = 'form' | 'raw';

function localIsoDate(): string {
    const now = new Date();
    const offset = now.getTimezoneOffset() * 60_000;
    return new Date(now.getTime() - offset).toISOString().slice(0, 10);
}

function createTransactionDraft(): TransactionDraft {
    return {
        date: localIsoDate(),
        description: '',
        comment: '',
        postings: [
            { account: '', amount: '', comment: '' },
            { account: '', amount: '', comment: '' },
        ],
    };
}

function App() {
    const [createStatus, setCreateStatus] = useState<string | null>(null);
    const [isCreating, setIsCreating] = useState(false);
    const [openStatus, setOpenStatus] = useState<string | null>(null);
    const [isOpening, setIsOpening] = useState(false);
    const [ledger, setLedger] = useState<LedgerView | null>(null);
    const [activeTab, setActiveTab] = useState<'accounts' | 'transactions'>(
        'accounts',
    );
    const [transactionDraft, setTransactionDraft] = useState<TransactionDraft>(
        createTransactionDraft,
    );
    const [rawDraft, setRawDraft] = useState('');
    const [entryMode, setEntryMode] = useState<TransactionEntryMode>('form');
    const [addStatus, setAddStatus] = useState<string | null>(null);
    const [isAdding, setIsAdding] = useState(false);
    const [draftStatus, setDraftStatus] = useState<string | null>(null);
    const [isValidatingDraft, setIsValidatingDraft] = useState(false);

    useEffect(() => {
        if (ledger) {
            setTransactionDraft(createTransactionDraft());
            setRawDraft('');
            setAddStatus(null);
            setDraftStatus(null);
        }
    }, [ledger]);

    const buildTransactionInput = (
        draft: TransactionDraft,
    ): { transaction: NewTransactionInput | null; error: string | null } => {
        const date = draft.date.trim();
        if (!date) {
            return { transaction: null, error: 'Date is required.' };
        }

        const trimmedPostings = draft.postings.map((posting) => ({
            account: posting.account.trim(),
            amount: posting.amount.trim(),
            comment: posting.comment.trim(),
        }));
        const nonEmptyPostings = trimmedPostings.filter(
            (posting) =>
                posting.account.length > 0 ||
                posting.amount.length > 0 ||
                posting.comment.length > 0,
        );

        if (nonEmptyPostings.some((posting) => posting.account.length === 0)) {
            return {
                transaction: null,
                error: 'Every amount or note needs an account.',
            };
        }

        if (nonEmptyPostings.length < 2) {
            return { transaction: null, error: 'Add at least two postings.' };
        }

        const missingAmounts = nonEmptyPostings.filter(
            (posting) => posting.amount.length === 0,
        ).length;
        if (missingAmounts > 1) {
            return {
                transaction: null,
                error: 'Only one posting may omit an amount.',
            };
        }

        return {
            transaction: {
                date,
                description: draft.description.trim(),
                comment: draft.comment.trim() || null,
                postings: nonEmptyPostings.map((posting) => ({
                    account: posting.account,
                    amount: posting.amount.length === 0 ? null : posting.amount,
                    comment:
                        posting.comment.length === 0 ? null : posting.comment,
                })),
            },
            error: null,
        };
    };

    useEffect(() => {
        if (!ledger || entryMode !== 'form') {
            return;
        }
        const hasDraftContent =
            transactionDraft.description.trim().length > 0 ||
            transactionDraft.comment.trim().length > 0 ||
            transactionDraft.postings.some(
                (posting) =>
                    posting.account.trim().length > 0 ||
                    posting.amount.trim().length > 0 ||
                    posting.comment.trim().length > 0,
            );
        if (!hasDraftContent) {
            setDraftStatus(null);
            setIsValidatingDraft(false);
            return;
        }
        const { transaction, error } = buildTransactionInput(transactionDraft);
        if (transaction === null) {
            setDraftStatus(error ?? 'Draft is incomplete.');
            setIsValidatingDraft(false);
            return;
        }

        let cancelled = false;
        setIsValidatingDraft(true);
        const timer = window.setTimeout(() => {
            void validateTransaction(ledger.path, transaction)
                .then(() => {
                    if (!cancelled) {
                        setDraftStatus('Draft passes hledger check.');
                    }
                })
                .catch((err: unknown) => {
                    if (!cancelled) {
                        setDraftStatus(`Draft check failed: ${String(err)}`);
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsValidatingDraft(false);
                    }
                });
        }, 350);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledger, transactionDraft, entryMode]);

    useEffect(() => {
        if (!ledger || entryMode !== 'raw') {
            return;
        }
        if (rawDraft.trim().length === 0) {
            setDraftStatus(null);
            setIsValidatingDraft(false);
            return;
        }

        let cancelled = false;
        setIsValidatingDraft(true);
        const timer = window.setTimeout(() => {
            void validateTransactionText(ledger.path, rawDraft)
                .then(() => {
                    if (!cancelled) {
                        setDraftStatus('Draft passes hledger check.');
                    }
                })
                .catch((err: unknown) => {
                    if (!cancelled) {
                        setDraftStatus(`Draft check failed: ${String(err)}`);
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsValidatingDraft(false);
                    }
                });
        }, 350);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledger, rawDraft, entryMode]);

    async function handleNewLedger() {
        setIsCreating(true);
        try {
            const created = await promptNewLedgerLocation();
            if (created) {
                setCreateStatus('Ledger created.');
            }
        } finally {
            setIsCreating(false);
        }
    }

    async function handleOpenLedger() {
        setIsOpening(true);
        setOpenStatus('Choose a Refreshmint ledger...');
        try {
            // On macOS, .refreshmint is treated as a package, so directory picker grays it out.
            const chooseDirectory = !navigator.userAgent.includes('Mac');
            const selection = (await openDialog({
                directory: chooseDirectory,
                multiple: false,
                title: 'Open Refreshmint ledger',
                ...(chooseDirectory
                    ? {}
                    : {
                          filters: [
                              {
                                  name: 'Refreshmint',
                                  extensions: ['refreshmint'],
                              },
                          ],
                      }),
            })) as string | string[] | null;
            if (selection === null) {
                setOpenStatus(null);
                return;
            }
            const path = Array.isArray(selection) ? selection[0] : selection;
            if (typeof path !== 'string' || path.length === 0) {
                setOpenStatus('Open canceled.');
                return;
            }
            setOpenStatus('Opening ledger...');
            const opened = await openLedger(path);
            setLedger(opened);
            setActiveTab('accounts');
            setOpenStatus(`Opened ${opened.path}`);
        } catch (error) {
            setOpenStatus(`Failed to open ledger: ${String(error)}`);
        } finally {
            setIsOpening(false);
        }
    }

    async function handleAddTransaction() {
        if (!ledger) {
            return;
        }
        setAddStatus(null);

        if (entryMode === 'raw') {
            if (rawDraft.trim().length === 0) {
                setAddStatus('Raw transaction is required.');
                return;
            }
            setIsAdding(true);
            try {
                const updated = await addTransactionText(ledger.path, rawDraft);
                setLedger(updated);
                setAddStatus('Transaction added.');
                setRawDraft('');
            } catch (error) {
                setAddStatus(`Failed to add transaction: ${String(error)}`);
            } finally {
                setIsAdding(false);
            }
            return;
        }

        const { transaction, error } = buildTransactionInput(transactionDraft);
        if (transaction === null) {
            setAddStatus(error ?? 'Transaction is incomplete.');
            return;
        }

        setIsAdding(true);
        try {
            const updated = await addTransaction(ledger.path, transaction);
            setLedger(updated);
            setAddStatus('Transaction added.');
            setTransactionDraft(createTransactionDraft());
        } catch (error) {
            setAddStatus(`Failed to add transaction: ${String(error)}`);
        } finally {
            setIsAdding(false);
        }
    }

    function selectMostRecentTransaction(
        transactions: TransactionRow[],
    ): TransactionRow | null {
        const [first, ...rest] = transactions;
        if (!first) {
            return null;
        }
        let best = first;
        let bestIndex = Number.parseInt(best.id, 10);
        if (Number.isNaN(bestIndex)) {
            bestIndex = -1;
        }
        for (const txn of rest) {
            const candidateIndex = Number.parseInt(txn.id, 10);
            if (!Number.isNaN(candidateIndex) && candidateIndex >= bestIndex) {
                best = txn;
                bestIndex = candidateIndex;
            } else if (bestIndex < 0) {
                best = txn;
            }
        }
        return best;
    }

    async function promptNewLedgerLocation(): Promise<boolean> {
        setCreateStatus('Choose a location for the new ledger...');
        try {
            const defaultPath = await defaultLedgerPath();
            const selection = await saveDialog({
                title: 'Create Refreshmint ledger',
                ...(defaultPath !== undefined ? { defaultPath } : {}),
                filters: [{ name: 'Refreshmint', extensions: ['refreshmint'] }],
            });
            if (typeof selection !== 'string' || selection.length === 0) {
                setCreateStatus('Create canceled.');
                return false;
            }
            await invoke('new_ledger', { ledger: selection });
            return true;
        } catch (error) {
            setCreateStatus(`Failed to create ledger: ${String(error)}`);
            return false;
        }
    }

    async function defaultLedgerPath(): Promise<string | undefined> {
        try {
            const documents = await documentDir();
            return await join(documents, 'accounting.refreshmint');
        } catch {
            return undefined;
        }
    }

    return (
        <div className="app">
            <header className="app-header">
                <div>
                    <p className="app-eyebrow">Refreshmint</p>
                    <h1>Ledger workspace</h1>
                    <p className="app-subtitle">
                        Open a <span>.refreshmint</span> directory to review
                        accounts and transactions.
                    </p>
                </div>
                <div className="app-actions">
                    <button
                        onClick={() => {
                            void handleNewLedger();
                        }}
                        disabled={isCreating}
                        type="button"
                    >
                        New...
                    </button>
                    <button
                        onClick={() => {
                            void handleOpenLedger();
                        }}
                        disabled={isOpening}
                        type="button"
                    >
                        Open...
                    </button>
                </div>
                {createStatus === null ? null : (
                    <p className="status">{createStatus}</p>
                )}
                {openStatus === null ? null : (
                    <p className="status">{openStatus}</p>
                )}
            </header>

            {ledger === null ? (
                <div className="empty-state">
                    <p>No ledger loaded yet.</p>
                </div>
            ) : (
                <section className="ledger">
                    <div className="tabs">
                        <button
                            className={
                                activeTab === 'accounts' ? 'tab active' : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('accounts');
                            }}
                            type="button"
                        >
                            Accounts
                        </button>
                        <button
                            className={
                                activeTab === 'transactions'
                                    ? 'tab active'
                                    : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('transactions');
                            }}
                            type="button"
                        >
                            Transactions
                        </button>
                    </div>

                    {activeTab === 'accounts' ? (
                        <div className="table-wrap">
                            <AccountsTable accounts={ledger.accounts} />
                        </div>
                    ) : (
                        <div className="transactions-panel">
                            <section className="txn-form">
                                <div className="txn-form-header">
                                    <div>
                                        <h2>New transaction</h2>
                                        <p>
                                            Amounts accept hledger syntax (for
                                            costs or balance assertions);
                                            comments can hold tags.
                                        </p>
                                    </div>
                                    <div className="header-actions">
                                        <div className="mode-toggle">
                                            <button
                                                className={
                                                    entryMode === 'form'
                                                        ? 'mode-button active'
                                                        : 'mode-button'
                                                }
                                                type="button"
                                                onClick={() => {
                                                    setEntryMode('form');
                                                    setAddStatus(null);
                                                    setDraftStatus(null);
                                                    setIsValidatingDraft(false);
                                                }}
                                            >
                                                Form
                                            </button>
                                            <button
                                                className={
                                                    entryMode === 'raw'
                                                        ? 'mode-button active'
                                                        : 'mode-button'
                                                }
                                                type="button"
                                                onClick={() => {
                                                    setEntryMode('raw');
                                                    setAddStatus(null);
                                                    setDraftStatus(null);
                                                    setIsValidatingDraft(false);
                                                }}
                                            >
                                                Raw
                                            </button>
                                        </div>
                                        {entryMode === 'form' ? (
                                            <button
                                                className="ghost-button"
                                                type="button"
                                                onClick={() => {
                                                    const last =
                                                        selectMostRecentTransaction(
                                                            ledger.transactions,
                                                        );
                                                    if (!last) {
                                                        setAddStatus(
                                                            'No transactions to copy.',
                                                        );
                                                        return;
                                                    }
                                                    setTransactionDraft(
                                                        (current) => ({
                                                            date: current.date,
                                                            description:
                                                                last.descriptionRaw.trim()
                                                                    .length > 0
                                                                    ? last.descriptionRaw
                                                                    : '',
                                                            comment:
                                                                last.comment,
                                                            postings:
                                                                last.postings.map(
                                                                    (
                                                                        posting,
                                                                    ) => ({
                                                                        account:
                                                                            posting.account,
                                                                        amount:
                                                                            posting.amount ??
                                                                            '',
                                                                        comment:
                                                                            posting.comment,
                                                                    }),
                                                                ),
                                                        }),
                                                    );
                                                    setAddStatus(
                                                        'Copied last transaction.',
                                                    );
                                                    setDraftStatus(null);
                                                }}
                                            >
                                                Copy last
                                            </button>
                                        ) : null}
                                        <button
                                            className="ghost-button"
                                            type="button"
                                            onClick={() => {
                                                if (entryMode === 'raw') {
                                                    setRawDraft('');
                                                } else {
                                                    setTransactionDraft(
                                                        createTransactionDraft(),
                                                    );
                                                }
                                                setAddStatus(null);
                                                setDraftStatus(null);
                                            }}
                                        >
                                            Reset
                                        </button>
                                    </div>
                                </div>
                                {entryMode === 'form' ? (
                                    <>
                                        <div className="txn-grid">
                                            <label className="field">
                                                <span>Date</span>
                                                <input
                                                    type="date"
                                                    value={
                                                        transactionDraft.date
                                                    }
                                                    placeholder="YYYY-MM-DD"
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
                                                                ...current,
                                                                date: value,
                                                            }),
                                                        );
                                                        setAddStatus(null);
                                                    }}
                                                />
                                            </label>
                                            <label className="field">
                                                <span>Description</span>
                                                <input
                                                    type="text"
                                                    value={
                                                        transactionDraft.description
                                                    }
                                                    placeholder="Description"
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
                                                                ...current,
                                                                description:
                                                                    value,
                                                            }),
                                                        );
                                                        setAddStatus(null);
                                                        setDraftStatus(null);
                                                    }}
                                                />
                                            </label>
                                            <label className="field">
                                                <span>Notes / tags</span>
                                                <input
                                                    type="text"
                                                    value={
                                                        transactionDraft.comment
                                                    }
                                                    placeholder="tag:food, note:..."
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
                                                                ...current,
                                                                comment: value,
                                                            }),
                                                        );
                                                        setAddStatus(null);
                                                        setDraftStatus(null);
                                                    }}
                                                />
                                            </label>
                                        </div>
                                        <div className="txn-postings">
                                            <datalist id="account-options">
                                                {ledger.accounts
                                                    .map(
                                                        (account) =>
                                                            account.name,
                                                    )
                                                    .filter(
                                                        (name, index, names) =>
                                                            names.indexOf(
                                                                name,
                                                            ) === index,
                                                    )
                                                    .map((name) => (
                                                        <option
                                                            key={name}
                                                            value={name}
                                                        />
                                                    ))}
                                            </datalist>
                                            {transactionDraft.postings.map(
                                                (posting, index) => (
                                                    <div
                                                        key={`posting-${index}`}
                                                        className="txn-posting-row"
                                                    >
                                                        <input
                                                            type="text"
                                                            value={
                                                                posting.account
                                                            }
                                                            placeholder="Account"
                                                            list="account-options"
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                const value =
                                                                    event.target
                                                                        .value;
                                                                setTransactionDraft(
                                                                    (
                                                                        current,
                                                                    ) => ({
                                                                        ...current,
                                                                        postings:
                                                                            current.postings.map(
                                                                                (
                                                                                    entry,
                                                                                    postingIndex,
                                                                                ) =>
                                                                                    postingIndex ===
                                                                                    index
                                                                                        ? {
                                                                                              ...entry,
                                                                                              account:
                                                                                                  value,
                                                                                          }
                                                                                        : entry,
                                                                            ),
                                                                    }),
                                                                );
                                                                setAddStatus(
                                                                    null,
                                                                );
                                                                setDraftStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        />
                                                        <input
                                                            type="text"
                                                            value={
                                                                posting.amount
                                                            }
                                                            placeholder="Amount (optional, supports assertions)"
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                const value =
                                                                    event.target
                                                                        .value;
                                                                setTransactionDraft(
                                                                    (
                                                                        current,
                                                                    ) => ({
                                                                        ...current,
                                                                        postings:
                                                                            current.postings.map(
                                                                                (
                                                                                    entry,
                                                                                    postingIndex,
                                                                                ) =>
                                                                                    postingIndex ===
                                                                                    index
                                                                                        ? {
                                                                                              ...entry,
                                                                                              amount: value,
                                                                                          }
                                                                                        : entry,
                                                                            ),
                                                                    }),
                                                                );
                                                                setAddStatus(
                                                                    null,
                                                                );
                                                                setDraftStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        />
                                                        <input
                                                            type="text"
                                                            value={
                                                                posting.comment
                                                            }
                                                            placeholder="Notes / tags"
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                const value =
                                                                    event.target
                                                                        .value;
                                                                setTransactionDraft(
                                                                    (
                                                                        current,
                                                                    ) => ({
                                                                        ...current,
                                                                        postings:
                                                                            current.postings.map(
                                                                                (
                                                                                    entry,
                                                                                    postingIndex,
                                                                                ) =>
                                                                                    postingIndex ===
                                                                                    index
                                                                                        ? {
                                                                                              ...entry,
                                                                                              comment:
                                                                                                  value,
                                                                                          }
                                                                                        : entry,
                                                                            ),
                                                                    }),
                                                                );
                                                                setAddStatus(
                                                                    null,
                                                                );
                                                                setDraftStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        />
                                                        <button
                                                            className="icon-button"
                                                            type="button"
                                                            disabled={
                                                                transactionDraft
                                                                    .postings
                                                                    .length <= 2
                                                            }
                                                            onClick={() => {
                                                                if (
                                                                    transactionDraft
                                                                        .postings
                                                                        .length <=
                                                                    2
                                                                ) {
                                                                    return;
                                                                }
                                                                setTransactionDraft(
                                                                    (
                                                                        current,
                                                                    ) => ({
                                                                        ...current,
                                                                        postings:
                                                                            current.postings.filter(
                                                                                (
                                                                                    _,
                                                                                    postingIndex,
                                                                                ) =>
                                                                                    postingIndex !==
                                                                                    index,
                                                                            ),
                                                                    }),
                                                                );
                                                                setAddStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        >
                                                            Remove
                                                        </button>
                                                    </div>
                                                ),
                                            )}
                                            <button
                                                className="ghost-button"
                                                type="button"
                                                onClick={() => {
                                                    setTransactionDraft(
                                                        (current) => ({
                                                            ...current,
                                                            postings: [
                                                                ...current.postings,
                                                                {
                                                                    account: '',
                                                                    amount: '',
                                                                    comment: '',
                                                                },
                                                            ],
                                                        }),
                                                    );
                                                    setAddStatus(null);
                                                }}
                                            >
                                                Add posting
                                            </button>
                                        </div>
                                    </>
                                ) : (
                                    <div className="raw-entry">
                                        <label className="field">
                                            <span>Raw transaction</span>
                                            <textarea
                                                className="raw-textarea"
                                                value={rawDraft}
                                                placeholder="Paste full hledger transaction text here."
                                                onChange={(event) => {
                                                    const value =
                                                        event.target.value;
                                                    setRawDraft(value);
                                                    setAddStatus(null);
                                                    setDraftStatus(null);
                                                }}
                                            />
                                        </label>
                                        <p className="hint">
                                            Accepts full hledger syntax (status,
                                            code, tags, balance assertions,
                                            virtual postings).
                                        </p>
                                    </div>
                                )}
                                <div className="txn-actions">
                                    <button
                                        type="button"
                                        className="primary-button"
                                        onClick={() => {
                                            void handleAddTransaction();
                                        }}
                                        disabled={isAdding}
                                    >
                                        {isAdding
                                            ? 'Adding...'
                                            : 'Add transaction'}
                                    </button>
                                </div>
                                {isValidatingDraft ? (
                                    <p className="status">Checking draft...</p>
                                ) : null}
                                {draftStatus === null ? null : (
                                    <p className="status">{draftStatus}</p>
                                )}
                                {addStatus === null ? null : (
                                    <p className="status">{addStatus}</p>
                                )}
                            </section>
                            <div className="table-wrap">
                                <TransactionsTable
                                    transactions={ledger.transactions}
                                />
                            </div>
                        </div>
                    )}
                </section>
            )}
        </div>
    );
}

function AccountsTable({ accounts }: { accounts: AccountRow[] }) {
    return (
        <table className="ledger-table">
            <thead>
                <tr>
                    <th>Account</th>
                    <th>Balance</th>
                </tr>
            </thead>
            <tbody>
                {accounts.length === 0 ? (
                    <tr>
                        <td colSpan={2} className="table-empty">
                            No accounts found.
                        </td>
                    </tr>
                ) : (
                    accounts.map((account) => (
                        <tr key={account.name}>
                            <td>{account.name}</td>
                            <td className="amount">
                                {formatTotals(account.totals)}
                            </td>
                        </tr>
                    ))
                )}
            </tbody>
        </table>
    );
}

function TransactionsTable({
    transactions,
}: {
    transactions: TransactionRow[];
}) {
    return (
        <table className="ledger-table">
            <thead>
                <tr>
                    <th>Date</th>
                    <th>Description</th>
                    <th>Postings</th>
                    <th>Amount</th>
                </tr>
            </thead>
            <tbody>
                {transactions.length === 0 ? (
                    <tr>
                        <td colSpan={4} className="table-empty">
                            No transactions found.
                        </td>
                    </tr>
                ) : (
                    transactions.map((txn) => (
                        <tr key={txn.id}>
                            <td className="mono">{txn.date}</td>
                            <td>{txn.description}</td>
                            <td>
                                <PostingsList postings={txn.postings} />
                            </td>
                            <td className="amount">
                                {formatTotals(txn.totals)}
                            </td>
                        </tr>
                    ))
                )}
            </tbody>
        </table>
    );
}

function formatTotals(totals: AmountTotal[] | null): string {
    if (!totals || totals.length === 0) {
        return 'N/A';
    }
    return totals.map(formatTotal).join(', ');
}

function formatTotal(total: AmountTotal): string {
    const value = formatScaled(total.mantissa, total.scale);
    const { side, spaced } = normalizeStyle(total.style);
    const separator = spaced ? ' ' : '';
    return side === 'L'
        ? `${total.commodity}${separator}${value}`
        : `${value}${separator}${total.commodity}`;
}

function normalizeStyle(style: AmountStyleHint | null) {
    if (style === null) {
        return { side: 'R' as const, spaced: true };
    }
    return style;
}

function formatScaled(mantissa: string, scale: number): string {
    let negative = false;
    let digits = mantissa;
    if (digits.startsWith('-')) {
        negative = true;
        digits = digits.slice(1);
    }
    if (scale > 0) {
        const scaleInt = Math.max(scale, 0);
        if (digits.length <= scaleInt) {
            const needed = scaleInt + 1 - digits.length;
            digits = `${'0'.repeat(needed)}${digits}`;
        }
        const split = digits.length - scaleInt;
        const value = `${digits.slice(0, split)}.${digits.slice(split)}`;
        return negative ? `-${value}` : value;
    }
    return negative ? `-${digits}` : digits;
}

function PostingsList({ postings }: { postings: PostingRow[] }) {
    return (
        <div className="postings-list">
            {postings.map((posting) => (
                <div key={posting.account} className="postings-item">
                    <span>{posting.account}</span>
                    <span className="amount">
                        {formatTotals(posting.totals)}
                    </span>
                </div>
            ))}
        </div>
    );
}

export default App;
