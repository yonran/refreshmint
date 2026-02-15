import { useState } from 'react';
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
    openLedger,
    type AccountRow,
    type LedgerView,
    type PostingRow,
    type TransactionRow,
} from './tauri-commands.ts';

function App() {
    const [createStatus, setCreateStatus] = useState<string | null>(null);
    const [isCreating, setIsCreating] = useState(false);
    const [openStatus, setOpenStatus] = useState<string | null>(null);
    const [isOpening, setIsOpening] = useState(false);
    const [ledger, setLedger] = useState<LedgerView | null>(null);
    const [activeTab, setActiveTab] = useState<'accounts' | 'transactions'>(
        'accounts',
    );

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

                    <div className="table-wrap">
                        {activeTab === 'accounts' ? (
                            <AccountsTable accounts={ledger.accounts} />
                        ) : (
                            <TransactionsTable
                                transactions={ledger.transactions}
                            />
                        )}
                    </div>
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
