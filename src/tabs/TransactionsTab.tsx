import { useEffect, useRef, useState } from 'react';
import {
    addTransaction,
    addTransactionText,
    type AmountStyleHint,
    type AmountTotal,
    type GlCategoryResult,
    type LedgerView,
    mergeGlTransfer,
    type NewTransactionInput,
    queryTransactions,
    recategorizeGlTransaction,
    suggestGlCategories,
    type TransactionRow,
    validateTransaction,
    validateTransactionText,
} from '../tauri-commands.ts';
import {
    getCurrentToken,
    getSearchSuggestions,
    quoteHledgerValue,
} from '../search-utils.ts';
import {
    type RecategorizeTab,
    type SimilarRecategorizeSeed,
    type TransactionDraft,
    type TransactionEntryMode,
    type TransactionsTabSession,
} from '../types.ts';
import { AccountInput, TransactionsTable } from './TransactionsTable.tsx';

function normalizeStyle(style: AmountStyleHint | null) {
    if (style === null) {
        return { side: 'R' as const, spaced: true };
    }
    return style;
}

function formatTotal(total: AmountTotal): string {
    let negative = false;
    let digits = total.mantissa;
    if (digits.startsWith('-')) {
        negative = true;
        digits = digits.slice(1);
    }
    const { side, spaced } = normalizeStyle(total.style);
    const separator = spaced ? ' ' : '';
    const scale = total.scale;
    let value: string;
    if (scale > 0) {
        const scaleInt = Math.max(scale, 0);
        if (digits.length <= scaleInt) {
            const needed = scaleInt + 1 - digits.length;
            digits = `${'0'.repeat(needed)}${digits}`;
        }
        const split = digits.length - scaleInt;
        value = `${digits.slice(0, split)}.${digits.slice(split)}`;
    } else {
        value = digits;
    }
    if (negative) {
        value = `-${value}`;
    }
    return side === 'L'
        ? `${total.commodity}${separator}${value}`
        : `${value}${separator}${total.commodity}`;
}

function formatTotals(totals: AmountTotal[] | null): string {
    if (!totals || totals.length === 0) {
        return 'N/A';
    }
    return totals.map(formatTotal).join(', ');
}

function joinQueryClauses(...clauses: string[]): string {
    return clauses
        .map((clause) => clause.trim())
        .filter((clause) => clause.length > 0)
        .join(' ');
}

function buildRecategorizeSeedQuery(
    description: string,
    balancingAccount: string,
): string {
    return joinQueryClauses(
        `desc:${quoteHledgerValue(description)}`,
        balancingAccount.length > 0
            ? `acct:${quoteHledgerValue(balancingAccount)}`
            : '',
    );
}

function buildCurrentFilterQuery(
    transactionsSearch: string,
    unpostedOnly: boolean,
): string {
    return joinQueryClauses(
        transactionsSearch,
        unpostedOnly ? 'acct:^Equity:Unreconciled' : '',
    );
}

function buildRecategorizeQuery(
    seedQuery: string,
    currentFilterQuery: string,
    includeAll: boolean,
): string {
    return joinQueryClauses(seedQuery, includeAll ? '' : currentFilterQuery);
}

interface RecategorizePostingOption {
    account: string;
    postingIndex: number;
    selectable: boolean;
}

type RecategorizeSelectionEntry = {
    txnId: string;
    postingIndex: number;
    oldAccount: string;
};

function getRecategorizePostingOptions(
    txn: TransactionRow,
): RecategorizePostingOption[] {
    return txn.postings.map((posting, postingIndex) => {
        const isBalanceSheet =
            posting.account.startsWith('Assets:') ||
            posting.account.startsWith('Liabilities:');
        return {
            account: posting.account,
            postingIndex,
            selectable: !isBalanceSheet,
        };
    });
}

function getSelectableRecategorizePostings(
    txn: TransactionRow,
): RecategorizePostingOption[] {
    return getRecategorizePostingOptions(txn).filter(
        (posting) => posting.selectable,
    );
}

interface TransactionsTabProps {
    ledger: LedgerView;
    isActive: boolean;
    hideObviousAmounts: boolean;
    onLedgerRefresh: () => void;
    // Recategorize tab integration (lifted to App.tsx for tab bar)
    onRecategorizeTabsChange: (
        updater: (current: RecategorizeTab[]) => RecategorizeTab[],
    ) => void;
    activeRecategorizeTab: RecategorizeTab | null;
    onOpenNewRecategorizeTab: (id: number) => void;
    onNavigateToTransactions: () => void;
    recategorizeTabIdRef: { current: number };
    // Cross-tab navigation: set a search term from outside (e.g. PipelineTab)
    pendingSearch: string | null;
    onPendingSearchConsumed: () => void;
    session: TransactionsTabSession;
    onSessionChange: (
        updater: (current: TransactionsTabSession) => TransactionsTabSession,
    ) => void;
}

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

function buildTransactionInput(draft: TransactionDraft): {
    transaction: NewTransactionInput | null;
    error: string | null;
} {
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
                comment: posting.comment.length === 0 ? null : posting.comment,
            })),
        },
        error: null,
    };
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

export function TransactionsTab({
    ledger,
    isActive,
    hideObviousAmounts,
    onLedgerRefresh,
    onRecategorizeTabsChange,
    activeRecategorizeTab,
    onOpenNewRecategorizeTab,
    onNavigateToTransactions,
    recategorizeTabIdRef,
    pendingSearch,
    onPendingSearchConsumed,
    session,
    onSessionChange,
}: TransactionsTabProps) {
    const [unpostedOnly, setUnpostedOnly] = useState(session.unpostedOnly);
    const [transactionDraft, setTransactionDraft] = useState<TransactionDraft>(
        session.transactionDraft ?? createTransactionDraft(),
    );
    const [rawDraft, setRawDraft] = useState(session.rawDraft);
    const [entryMode, setEntryMode] = useState<TransactionEntryMode>(
        session.entryMode,
    );
    const [addStatus, setAddStatus] = useState<string | null>(null);
    const [isAdding, setIsAdding] = useState(false);
    const [draftStatus, setDraftStatus] = useState<string | null>(null);
    const [isValidatingDraft, setIsValidatingDraft] = useState(false);
    const [transactionsSearch, setTransactionsSearch] = useState(
        session.transactionsSearch,
    );
    const [selectedTransactionIds, setSelectedTransactionIds] = useState(
        session.selectedTransactionIds,
    );
    const [queryResults, setQueryResults] = useState<TransactionRow[] | null>(
        null,
    );
    const [queryError, setQueryError] = useState<string | null>(null);
    const [isNewTxnExpandedOverride, setIsNewTxnExpandedOverride] = useState<
        boolean | null
    >(session.isNewTxnExpandedOverride);
    const [acSuggestions, setAcSuggestions] = useState<string[]>([]);
    const [acActiveIndex, setAcActiveIndex] = useState(-1);
    const [similarAcSuggestions, setSimilarAcSuggestions] = useState<string[]>(
        [],
    );
    const [similarAcActiveIndex, setSimilarAcActiveIndex] = useState(-1);
    const [glCategorySuggestions, setGlCategorySuggestions] = useState<
        Record<string, GlCategoryResult>
    >({});
    const [glTransferModalTxnId, setGlTransferModalTxnId] = useState<
        string | null
    >(session.glTransferModalTxnId);
    const [glTransferModalSearch, setGlTransferModalSearch] = useState(
        session.glTransferModalSearch,
    );
    const [recategorizeBulkConfirm, setRecategorizeBulkConfirm] = useState<{
        entries: RecategorizeSelectionEntry[];
        newAccount: string;
    } | null>(null);

    const searchInputRef = useRef<HTMLInputElement>(null);
    const similarSearchInputRef = useRef<HTMLInputElement>(null);
    const hasSeenLedgerRef = useRef(false);
    const sessionRef = useRef<TransactionsTabSession>(session);
    const transactionsTableScrollTopRef = useRef(
        session.transactionsTableScrollTop,
    );

    const ledgerPath = ledger.path;

    sessionRef.current = {
        unpostedOnly,
        transactionDraft,
        rawDraft,
        entryMode,
        transactionsSearch,
        selectedTransactionIds,
        transactionsTableScrollTop: transactionsTableScrollTopRef.current,
        isNewTxnExpandedOverride,
        glTransferModalTxnId,
        glTransferModalSearch,
    };

    useEffect(() => {
        setUnpostedOnly(session.unpostedOnly);
        setTransactionDraft(
            session.transactionDraft ?? createTransactionDraft(),
        );
        setRawDraft(session.rawDraft);
        setEntryMode(session.entryMode);
        setTransactionsSearch(session.transactionsSearch);
        setSelectedTransactionIds(session.selectedTransactionIds);
        transactionsTableScrollTopRef.current =
            session.transactionsTableScrollTop;
        setIsNewTxnExpandedOverride(session.isNewTxnExpandedOverride);
        setGlTransferModalTxnId(session.glTransferModalTxnId);
        setGlTransferModalSearch(session.glTransferModalSearch);
    }, [session]);

    // Persist the latest local tab state when the tab unmounts.
    useEffect(() => {
        return () => {
            onSessionChange(() => sessionRef.current);
        };
    }, [onSessionChange]);

    // Reset state when ledger changes (but not on first mount).
    useEffect(() => {
        if (!hasSeenLedgerRef.current) {
            hasSeenLedgerRef.current = true;
            return;
        }
        setTransactionDraft(createTransactionDraft());
        setRawDraft('');
        setAddStatus(null);
        setDraftStatus(null);
        setQueryResults(null);
        setQueryError(null);
        setIsNewTxnExpandedOverride(null);
        setTransactionsSearch('');
        setSelectedTransactionIds([]);
        transactionsTableScrollTopRef.current = 0;
        setAcSuggestions([]);
        setAcActiveIndex(-1);
        setSimilarAcSuggestions([]);
        setSimilarAcActiveIndex(-1);
        setUnpostedOnly(false);
    }, [ledgerPath]);

    // Apply cross-tab search navigation (e.g. from PipelineTab)
    useEffect(() => {
        if (pendingSearch !== null) {
            setTransactionsSearch(pendingSearch);
            onPendingSearchConsumed();
        }
    }, [pendingSearch, onPendingSearchConsumed]);

    useEffect(() => {
        setRecategorizeBulkConfirm(null);
    }, [activeRecategorizeTab?.id]);

    // Cmd+F focuses search when this tab is active
    useEffect(() => {
        if (!isActive) return;
        const handler = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
                e.preventDefault();
                searchInputRef.current?.focus();
                searchInputRef.current?.select();
            }
        };
        window.addEventListener('keydown', handler);
        return () => {
            window.removeEventListener('keydown', handler);
        };
    }, [isActive]);

    // Query transactions with debounce
    useEffect(() => {
        const q = transactionsSearch.trim();
        if (!q) {
            setQueryResults(null);
            setQueryError(null);
            return;
        }
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const rows = await queryTransactions(ledgerPath, q);
                    setQueryResults(rows);
                    setQueryError(null);
                } catch (err) {
                    setQueryError(String(err));
                    setQueryResults(null);
                }
            })();
        }, 300);
        return () => {
            clearTimeout(timer);
        };
    }, [transactionsSearch, ledger.transactions, ledgerPath]);

    const activeRecategorizeTabId = activeRecategorizeTab?.id ?? null;
    const activeRecategorizeSearchQuery =
        activeRecategorizeTab?.plan.searchQuery ?? '';

    // Query transactions for active recategorize tab
    useEffect(() => {
        if (activeRecategorizeTabId === null) {
            return;
        }
        const q = activeRecategorizeSearchQuery.trim();
        if (!q) {
            onRecategorizeTabsChange((current) =>
                current.map((tab) =>
                    tab.id === activeRecategorizeTabId
                        ? {
                              ...tab,
                              queryResults: ledger.transactions,
                              queryError: null,
                          }
                        : tab,
                ),
            );
            return;
        }
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const rows = await queryTransactions(ledgerPath, q);
                    onRecategorizeTabsChange((current) =>
                        current.map((tab) =>
                            tab.id === activeRecategorizeTabId
                                ? {
                                      ...tab,
                                      queryResults: rows,
                                      queryError: null,
                                  }
                                : tab,
                        ),
                    );
                } catch (err) {
                    onRecategorizeTabsChange((current) =>
                        current.map((tab) =>
                            tab.id === activeRecategorizeTabId
                                ? {
                                      ...tab,
                                      queryResults: null,
                                      queryError: String(err),
                                  }
                                : tab,
                        ),
                    );
                }
            })();
        }, 300);
        return () => {
            clearTimeout(timer);
        };
    }, [
        activeRecategorizeSearchQuery,
        activeRecategorizeTabId,
        ledger.transactions,
        ledgerPath,
        onRecategorizeTabsChange,
    ]);

    // Load GL category suggestions when this tab is active
    useEffect(() => {
        if (!isActive || activeRecategorizeTab !== null) {
            return;
        }
        let cancelled = false;
        suggestGlCategories(ledgerPath)
            .then((result) => {
                if (!cancelled) {
                    setGlCategorySuggestions(result);
                }
            })
            .catch((err: unknown) => {
                console.error('suggestGlCategories failed:', err);
            });
        return () => {
            cancelled = true;
        };
    }, [isActive, activeRecategorizeTab, ledgerPath]);

    // Validate form transaction draft
    useEffect(() => {
        if (entryMode !== 'form') {
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
            void validateTransaction(ledgerPath, transaction)
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
    }, [ledgerPath, transactionDraft, entryMode]);

    // Validate raw transaction draft
    useEffect(() => {
        if (entryMode !== 'raw') {
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
            void validateTransactionText(ledgerPath, rawDraft)
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
    }, [ledgerPath, rawDraft, entryMode]);

    const filteredTransactions = (() => {
        const base = queryResults ?? ledger.transactions;
        return base.filter((txn) => {
            if (unpostedOnly) {
                if (
                    !txn.postings.some((p) =>
                        p.account.startsWith('Equity:Unreconciled'),
                    )
                ) {
                    return false;
                }
            }
            return true;
        });
    })();

    const isNewTxnExpanded =
        isNewTxnExpandedOverride ?? ledger.transactions.length === 0;

    async function handleAddTransaction() {
        setAddStatus(null);

        if (entryMode === 'raw') {
            if (rawDraft.trim().length === 0) {
                setAddStatus('Raw transaction is required.');
                return;
            }
            setIsAdding(true);
            try {
                await addTransactionText(ledgerPath, rawDraft);
                onLedgerRefresh();
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
            await addTransaction(ledgerPath, transaction);
            onLedgerRefresh();
            setAddStatus('Transaction added.');
            setTransactionDraft(createTransactionDraft());
        } catch (error) {
            setAddStatus(`Failed to add transaction: ${String(error)}`);
        } finally {
            setIsAdding(false);
        }
    }

    async function handleRecategorizeGlTransaction(
        txnId: string,
        postingIndex: number,
        newAccount: string,
    ) {
        try {
            await recategorizeGlTransaction(
                ledgerPath,
                txnId,
                postingIndex,
                newAccount,
            );
            onLedgerRefresh();
            const suggestions = await suggestGlCategories(ledgerPath);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('recategorize failed:', error);
        }
    }

    async function handleMergeGlTransfer(txnId1: string, txnId2: string) {
        try {
            await mergeGlTransfer(ledgerPath, txnId1, txnId2);
            onLedgerRefresh();
            const suggestions = await suggestGlCategories(ledgerPath);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('merge transfer failed:', error);
        }
    }

    async function handleBulkRecategorize(
        entries: RecategorizeSelectionEntry[],
        newAccount: string,
    ) {
        try {
            for (const { txnId, postingIndex } of entries) {
                await recategorizeGlTransaction(
                    ledgerPath,
                    txnId,
                    postingIndex,
                    newAccount,
                );
            }
            onLedgerRefresh();
            const suggestions = await suggestGlCategories(ledgerPath);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('bulk recategorize failed:', error);
        }
    }

    function closeActiveRecategorizeTab() {
        if (activeRecategorizeTab === null) {
            return;
        }
        setRecategorizeBulkConfirm(null);
        onRecategorizeTabsChange((current) =>
            current.filter((tab) => tab.id !== activeRecategorizeTab.id),
        );
        onNavigateToTransactions();
    }

    function applyRecategorizeSelection(
        entries: RecategorizeSelectionEntry[],
        newAccount: string,
    ) {
        const accounts = new Set(entries.map((entry) => entry.oldAccount));
        if (accounts.size > 1) {
            setRecategorizeBulkConfirm({ entries, newAccount });
            return;
        }
        void handleBulkRecategorize(entries, newAccount);
        closeActiveRecategorizeTab();
    }

    function applySearchCompletion(suggestion: string) {
        const input = searchInputRef.current;
        if (!input) return;
        const cursorPos = input.selectionStart ?? transactionsSearch.length;
        const { token, start, end } = getCurrentToken(
            transactionsSearch,
            cursorPos,
        );
        const cursorOffsetInToken = cursorPos - start;
        const colonIdx = token.indexOf(':');
        const cursorBeforeColon =
            colonIdx !== -1 && cursorOffsetInToken <= colonIdx;

        let inserted: string;
        let replaceEnd: number;
        if (cursorBeforeColon && suggestion.endsWith(':')) {
            inserted = suggestion + token.substring(colonIdx + 1);
            replaceEnd = end;
        } else {
            inserted = suggestion;
            replaceEnd = end;
        }

        const needsTrailingSpace = replaceEnd >= transactionsSearch.length;
        const newValue =
            transactionsSearch.substring(0, start) +
            inserted +
            (needsTrailingSpace ? ' ' : '') +
            transactionsSearch.substring(replaceEnd);
        setTransactionsSearch(newValue);
        setAcSuggestions([]);
        setAcActiveIndex(-1);
        requestAnimationFrame(() => {
            const pos = start + inserted.length + (needsTrailingSpace ? 1 : 0);
            input.setSelectionRange(pos, pos);
        });
    }

    function applySimilarSearchCompletion(suggestion: string) {
        const input = similarSearchInputRef.current;
        const currentQuery = activeRecategorizeTab?.plan.searchQuery ?? '';
        if (!input || activeRecategorizeTab === null) return;
        const cursorPos = input.selectionStart ?? currentQuery.length;
        const { token, start, end } = getCurrentToken(currentQuery, cursorPos);
        const cursorOffsetInToken = cursorPos - start;
        const colonIdx = token.indexOf(':');
        const cursorBeforeColon =
            colonIdx !== -1 && cursorOffsetInToken <= colonIdx;

        let inserted: string;
        let replaceEnd: number;
        if (cursorBeforeColon && suggestion.endsWith(':')) {
            inserted = suggestion + token.substring(colonIdx + 1);
            replaceEnd = end;
        } else {
            inserted = suggestion;
            replaceEnd = end;
        }

        const needsTrailingSpace = replaceEnd >= currentQuery.length;
        const newValue =
            currentQuery.substring(0, start) +
            inserted +
            (needsTrailingSpace ? ' ' : '') +
            currentQuery.substring(replaceEnd);
        onRecategorizeTabsChange((current) =>
            current.map((tab) =>
                tab.id === activeRecategorizeTab.id
                    ? {
                          ...tab,
                          plan: {
                              ...tab.plan,
                              searchQuery: newValue,
                              queryCustomized: true,
                          },
                      }
                    : tab,
            ),
        );
        setSimilarAcSuggestions([]);
        setSimilarAcActiveIndex(-1);
        requestAnimationFrame(() => {
            const pos = start + inserted.length + (needsTrailingSpace ? 1 : 0);
            input.setSelectionRange(pos, pos);
        });
    }

    function appendSearchTerm(term: string) {
        const current = transactionsSearch.trim();
        setTransactionsSearch(current ? `${current} ${term}` : term);
    }

    function handleOpenSimilarRecategorize(seed: SimilarRecategorizeSeed) {
        const id = recategorizeTabIdRef.current++;
        const currentFilterQuery = buildCurrentFilterQuery(
            transactionsSearch,
            unpostedOnly,
        );
        const seedQuery = buildRecategorizeSeedQuery(
            seed.description,
            seed.balancingAccount,
        );
        onRecategorizeTabsChange((current) => [
            ...current,
            {
                id,
                plan: {
                    ...seed,
                    searchQuery: buildRecategorizeQuery(
                        seedQuery,
                        currentFilterQuery,
                        false,
                    ),
                    seedQuery,
                    currentFilterQuery,
                    includeAll: false,
                    queryCustomized: false,
                },
                queryResults: null,
                queryError: null,
                selectedPostingIndexByTxn: {},
            },
        ]);
        setSimilarAcSuggestions([]);
        setSimilarAcActiveIndex(-1);
        onOpenNewRecategorizeTab(id);
    }

    if (activeRecategorizeTab !== null) {
        const visibleRecategorizeTransactions =
            activeRecategorizeTab.queryResults ??
            (activeRecategorizeTab.plan.searchQuery.trim().length === 0
                ? ledger.transactions
                : []);
        const updateActiveRecategorizeTab = (
            updater: (tab: RecategorizeTab) => RecategorizeTab,
        ) => {
            onRecategorizeTabsChange((current) =>
                current.map((tab) =>
                    tab.id === activeRecategorizeTab.id ? updater(tab) : tab,
                ),
            );
        };
        const getSelectedRecategorizePostingIndex = (
            txn: TransactionRow,
        ): number | null => {
            if (
                Object.prototype.hasOwnProperty.call(
                    activeRecategorizeTab.selectedPostingIndexByTxn,
                    txn.id,
                )
            ) {
                return (
                    activeRecategorizeTab.selectedPostingIndexByTxn[txn.id] ??
                    null
                );
            }
            const candidatePostings = getSelectableRecategorizePostings(txn);
            return candidatePostings.length === 1
                ? (candidatePostings[0]?.postingIndex ?? null)
                : null;
        };
        const selectedRecategorizeEntries =
            visibleRecategorizeTransactions.flatMap((txn) => {
                const postingIndex = getSelectedRecategorizePostingIndex(txn);
                if (postingIndex === null) {
                    return [];
                }
                const posting = txn.postings[postingIndex];
                return posting === undefined
                    ? []
                    : [
                          {
                              txnId: txn.id,
                              postingIndex,
                              oldAccount: posting.account,
                          },
                      ];
            });
        const selectableCandidateAccounts =
            visibleRecategorizeTransactions.flatMap((txn) =>
                getSelectableRecategorizePostings(txn).map(
                    (posting) => posting.account,
                ),
            );
        const unambiguousRecategorizeTransactions =
            visibleRecategorizeTransactions.filter(
                (txn) => getSelectableRecategorizePostings(txn).length === 1,
            );
        const allUnambiguousSelected =
            unambiguousRecategorizeTransactions.length > 0 &&
            unambiguousRecategorizeTransactions.every(
                (txn) => getSelectedRecategorizePostingIndex(txn) !== null,
            );
        const someUnambiguousSelected =
            unambiguousRecategorizeTransactions.some(
                (txn) => getSelectedRecategorizePostingIndex(txn) !== null,
            );
        const canToggleIncludeAll =
            activeRecategorizeTab.plan.currentFilterQuery.trim().length > 0;
        return (
            <div className="transactions-panel">
                {(() => {
                    return (
                        <section className="txn-form">
                            <div className="txn-form-header">
                                <div>
                                    <h2>Recategorize</h2>
                                    <p>
                                        Refine query and destination, then apply
                                        to matching rows.
                                    </p>
                                </div>
                                <div className="header-actions">
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            onNavigateToTransactions();
                                        }}
                                    >
                                        Back to Transactions
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            closeActiveRecategorizeTab();
                                        }}
                                    >
                                        Close
                                    </button>
                                </div>
                            </div>
                            <div className="search-bar-wrapper">
                                <input
                                    ref={similarSearchInputRef}
                                    type="search"
                                    placeholder="Search… (hledger query: desc:amazon acct:^Expenses date:thismonth)"
                                    value={
                                        activeRecategorizeTab.plan.searchQuery
                                    }
                                    onChange={(e) => {
                                        const val = e.target.value;
                                        updateActiveRecategorizeTab((tab) => ({
                                            ...tab,
                                            plan: {
                                                ...tab.plan,
                                                searchQuery: val,
                                                queryCustomized: true,
                                            },
                                        }));
                                        const cursorPos =
                                            e.target.selectionStart ??
                                            val.length;
                                        const { token, start } =
                                            getCurrentToken(val, cursorPos);
                                        const cursorOffsetInToken =
                                            cursorPos - start;
                                        const sugs = getSearchSuggestions(
                                            token,
                                            cursorOffsetInToken,
                                            ledger.accounts,
                                        );
                                        setSimilarAcSuggestions(sugs);
                                        setSimilarAcActiveIndex(-1);
                                    }}
                                    onKeyDown={(e) => {
                                        if (similarAcSuggestions.length === 0)
                                            return;
                                        if (e.key === 'ArrowDown') {
                                            e.preventDefault();
                                            setSimilarAcActiveIndex((i) =>
                                                Math.min(
                                                    i + 1,
                                                    similarAcSuggestions.length -
                                                        1,
                                                ),
                                            );
                                        } else if (e.key === 'ArrowUp') {
                                            e.preventDefault();
                                            setSimilarAcActiveIndex((i) =>
                                                Math.max(i - 1, 0),
                                            );
                                        } else if (
                                            (e.key === 'Enter' ||
                                                e.key === 'Tab') &&
                                            similarAcActiveIndex >= 0
                                        ) {
                                            e.preventDefault();
                                            applySimilarSearchCompletion(
                                                similarAcSuggestions[
                                                    similarAcActiveIndex
                                                ] ?? '',
                                            );
                                        } else if (e.key === 'Escape') {
                                            setSimilarAcSuggestions([]);
                                            setSimilarAcActiveIndex(-1);
                                        }
                                    }}
                                    onBlur={() => {
                                        setTimeout(() => {
                                            setSimilarAcSuggestions([]);
                                            setSimilarAcActiveIndex(-1);
                                        }, 150);
                                    }}
                                />
                                {similarAcSuggestions.length > 0 && (
                                    <div
                                        className="search-autocomplete"
                                        role="listbox"
                                    >
                                        {similarAcSuggestions.map((sug, i) => (
                                            <div
                                                key={sug}
                                                className={`ac-item${i === similarAcActiveIndex ? ' active' : ''}`}
                                                role="option"
                                                aria-selected={
                                                    i === similarAcActiveIndex
                                                }
                                                onMouseDown={(e) => {
                                                    e.preventDefault();
                                                    applySimilarSearchCompletion(
                                                        sug,
                                                    );
                                                }}
                                            >
                                                {sug}
                                            </div>
                                        ))}
                                    </div>
                                )}
                            </div>
                            {activeRecategorizeTab.queryError !== null && (
                                <div className="query-error">
                                    {activeRecategorizeTab.queryError}
                                </div>
                            )}
                            <p>
                                Recategorize{' '}
                                <strong>
                                    {selectedRecategorizeEntries.length}
                                </strong>{' '}
                                selected postings across{' '}
                                <strong>
                                    {visibleRecategorizeTransactions.length}
                                </strong>{' '}
                                matching transactions to{' '}
                                <strong>
                                    {activeRecategorizeTab.plan.newAccount.trim() ||
                                        '(choose category)'}
                                </strong>
                                ?
                            </p>
                            <AccountInput
                                value={activeRecategorizeTab.plan.newAccount}
                                onChange={(v) => {
                                    updateActiveRecategorizeTab((tab) => ({
                                        ...tab,
                                        plan: {
                                            ...tab.plan,
                                            newAccount: v,
                                        },
                                    }));
                                }}
                                accounts={ledger.accounts.map((a) => a.name)}
                                oldAccount={
                                    selectedRecategorizeEntries.length > 0
                                        ? selectedRecategorizeEntries.map(
                                              (entry) => entry.oldAccount,
                                          )
                                        : selectableCandidateAccounts
                                }
                                placeholder="Destination category…"
                            />
                            {canToggleIncludeAll && (
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={
                                            activeRecategorizeTab.plan
                                                .includeAll
                                        }
                                        disabled={
                                            activeRecategorizeTab.plan
                                                .queryCustomized
                                        }
                                        onChange={(e) => {
                                            const checked = e.target.checked;
                                            updateActiveRecategorizeTab(
                                                (tab) => ({
                                                    ...tab,
                                                    plan: {
                                                        ...tab.plan,
                                                        includeAll: checked,
                                                        searchQuery:
                                                            buildRecategorizeQuery(
                                                                tab.plan
                                                                    .seedQuery,
                                                                tab.plan
                                                                    .currentFilterQuery,
                                                                checked,
                                                            ),
                                                    },
                                                }),
                                            );
                                        }}
                                    />
                                    <span>
                                        Include transactions not in current
                                        filter
                                    </span>
                                </label>
                            )}
                            {activeRecategorizeTab.plan.queryCustomized &&
                                canToggleIncludeAll && (
                                    <p className="status">
                                        Query customized. The include toggle no
                                        longer rewrites it.
                                    </p>
                                )}
                            <div className="table-wrap similar-confirm-table-wrap">
                                <table className="ledger-table">
                                    <thead>
                                        <tr>
                                            <th>Date</th>
                                            <th>Description</th>
                                            <th>
                                                {unambiguousRecategorizeTransactions.length >
                                                0 ? (
                                                    <input
                                                        type="checkbox"
                                                        checked={
                                                            allUnambiguousSelected
                                                        }
                                                        ref={(el) => {
                                                            if (el) {
                                                                el.indeterminate =
                                                                    someUnambiguousSelected &&
                                                                    !allUnambiguousSelected;
                                                            }
                                                        }}
                                                        onChange={() => {
                                                            const shouldSelect =
                                                                !allUnambiguousSelected;
                                                            updateActiveRecategorizeTab(
                                                                (tab) => {
                                                                    const nextSelections =
                                                                        {
                                                                            ...tab.selectedPostingIndexByTxn,
                                                                        };
                                                                    for (const txn of unambiguousRecategorizeTransactions) {
                                                                        const candidate =
                                                                            getSelectableRecategorizePostings(
                                                                                txn,
                                                                            )[0]
                                                                                ?.postingIndex ??
                                                                            null;
                                                                        nextSelections[
                                                                            txn.id
                                                                        ] =
                                                                            shouldSelect
                                                                                ? candidate
                                                                                : null;
                                                                    }
                                                                    return {
                                                                        ...tab,
                                                                        selectedPostingIndexByTxn:
                                                                            nextSelections,
                                                                    };
                                                                },
                                                            );
                                                        }}
                                                    />
                                                ) : null}{' '}
                                                Posting
                                            </th>
                                            <th>Amount</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {visibleRecategorizeTransactions.length ===
                                        0 ? (
                                            <tr>
                                                <td
                                                    colSpan={4}
                                                    className="table-empty"
                                                >
                                                    No transactions match the
                                                    current query.
                                                </td>
                                            </tr>
                                        ) : (
                                            visibleRecategorizeTransactions.map(
                                                (txn) => {
                                                    const postingOptions =
                                                        getRecategorizePostingOptions(
                                                            txn,
                                                        );
                                                    const selectedPostingIndex =
                                                        getSelectedRecategorizePostingIndex(
                                                            txn,
                                                        );
                                                    return (
                                                        <tr key={txn.id}>
                                                            <td className="mono">
                                                                {txn.date}
                                                            </td>
                                                            <td>
                                                                {
                                                                    txn.description
                                                                }
                                                            </td>
                                                            <td>
                                                                {postingOptions.length ===
                                                                0 ? (
                                                                    <span className="status">
                                                                        No
                                                                        candidate
                                                                        postings
                                                                    </span>
                                                                ) : (
                                                                    postingOptions.map(
                                                                        (
                                                                            posting,
                                                                            index,
                                                                        ) => (
                                                                            <label
                                                                                key={`${txn.id}:${posting.account}:${index}`}
                                                                                className="checkbox-field"
                                                                            >
                                                                                <input
                                                                                    type="checkbox"
                                                                                    disabled={
                                                                                        !posting.selectable
                                                                                    }
                                                                                    checked={
                                                                                        posting.selectable &&
                                                                                        selectedPostingIndex ===
                                                                                            posting.postingIndex
                                                                                    }
                                                                                    onChange={(
                                                                                        e,
                                                                                    ) => {
                                                                                        if (
                                                                                            !posting.selectable
                                                                                        ) {
                                                                                            return;
                                                                                        }
                                                                                        updateActiveRecategorizeTab(
                                                                                            (
                                                                                                tab,
                                                                                            ) => ({
                                                                                                ...tab,
                                                                                                selectedPostingIndexByTxn:
                                                                                                    {
                                                                                                        ...tab.selectedPostingIndexByTxn,
                                                                                                        [txn.id]:
                                                                                                            e
                                                                                                                .target
                                                                                                                .checked
                                                                                                                ? posting.postingIndex
                                                                                                                : null,
                                                                                                    },
                                                                                            }),
                                                                                        );
                                                                                    }}
                                                                                />
                                                                                <span className="mono">
                                                                                    {
                                                                                        posting.account
                                                                                    }
                                                                                </span>
                                                                            </label>
                                                                        ),
                                                                    )
                                                                )}
                                                            </td>
                                                            <td className="amount">
                                                                {formatTotals(
                                                                    txn.totals,
                                                                )}
                                                            </td>
                                                        </tr>
                                                    );
                                                },
                                            )
                                        )}
                                    </tbody>
                                </table>
                            </div>
                            <div className="txn-actions">
                                <button
                                    type="button"
                                    className="primary-button"
                                    disabled={
                                        !activeRecategorizeTab.plan.newAccount.trim() ||
                                        selectedRecategorizeEntries.length === 0
                                    }
                                    onClick={() => {
                                        applyRecategorizeSelection(
                                            selectedRecategorizeEntries,
                                            activeRecategorizeTab.plan.newAccount.trim(),
                                        );
                                    }}
                                >
                                    Apply
                                </button>
                            </div>
                            {recategorizeBulkConfirm !== null && (
                                <div
                                    className="modal-overlay"
                                    onClick={() => {
                                        setRecategorizeBulkConfirm(null);
                                    }}
                                >
                                    <div
                                        className="modal-dialog"
                                        onClick={(e) => {
                                            e.stopPropagation();
                                        }}
                                    >
                                        <div className="modal-header">
                                            <h3>Confirm bulk recategorize</h3>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    setRecategorizeBulkConfirm(
                                                        null,
                                                    );
                                                }}
                                            >
                                                Close
                                            </button>
                                        </div>
                                        <p>
                                            The selected rows have different
                                            current accounts. All will be
                                            changed to{' '}
                                            <strong>
                                                {
                                                    recategorizeBulkConfirm.newAccount
                                                }
                                            </strong>
                                            :
                                        </p>
                                        <ul>
                                            {[
                                                ...new Map(
                                                    recategorizeBulkConfirm.entries.map(
                                                        (entry) => [
                                                            entry.oldAccount,
                                                            0,
                                                        ],
                                                    ),
                                                ).keys(),
                                            ].map((account) => {
                                                const count =
                                                    recategorizeBulkConfirm.entries.filter(
                                                        (entry) =>
                                                            entry.oldAccount ===
                                                            account,
                                                    ).length;
                                                return (
                                                    <li key={account}>
                                                        {count} x {account}{' '}
                                                        {'->'}{' '}
                                                        {
                                                            recategorizeBulkConfirm.newAccount
                                                        }
                                                    </li>
                                                );
                                            })}
                                        </ul>
                                        <div
                                            style={{
                                                display: 'flex',
                                                gap: '0.5rem',
                                                justifyContent: 'flex-end',
                                            }}
                                        >
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    setRecategorizeBulkConfirm(
                                                        null,
                                                    );
                                                }}
                                            >
                                                Cancel
                                            </button>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    void handleBulkRecategorize(
                                                        recategorizeBulkConfirm.entries,
                                                        recategorizeBulkConfirm.newAccount,
                                                    );
                                                    closeActiveRecategorizeTab();
                                                }}
                                            >
                                                Confirm
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            )}
                        </section>
                    );
                })()}
            </div>
        );
    }

    return (
        <div className="transactions-panel">
            <div className="search-bar-row">
                <div className="search-bar-wrapper">
                    <input
                        ref={searchInputRef}
                        type="search"
                        placeholder="Search… (hledger query: desc:amazon acct:^Expenses date:thismonth)"
                        value={transactionsSearch}
                        onChange={(e) => {
                            const val = e.target.value;
                            setTransactionsSearch(val);
                            const cursorPos =
                                e.target.selectionStart ?? val.length;
                            const { token, start } = getCurrentToken(
                                val,
                                cursorPos,
                            );
                            const cursorOffsetInToken = cursorPos - start;
                            const sugs = getSearchSuggestions(
                                token,
                                cursorOffsetInToken,
                                ledger.accounts,
                            );
                            setAcSuggestions(sugs);
                            setAcActiveIndex(-1);
                        }}
                        onKeyDown={(e) => {
                            if (acSuggestions.length === 0) return;
                            if (e.key === 'ArrowDown') {
                                e.preventDefault();
                                setAcActiveIndex((i) =>
                                    Math.min(i + 1, acSuggestions.length - 1),
                                );
                            } else if (e.key === 'ArrowUp') {
                                e.preventDefault();
                                setAcActiveIndex((i) => Math.max(i - 1, 0));
                            } else if (
                                (e.key === 'Enter' || e.key === 'Tab') &&
                                acActiveIndex >= 0
                            ) {
                                e.preventDefault();
                                applySearchCompletion(
                                    acSuggestions[acActiveIndex] ?? '',
                                );
                            } else if (e.key === 'Escape') {
                                setAcSuggestions([]);
                                setAcActiveIndex(-1);
                            }
                        }}
                        onBlur={() => {
                            setTimeout(() => {
                                setAcSuggestions([]);
                                setAcActiveIndex(-1);
                            }, 150);
                        }}
                    />
                    {acSuggestions.length > 0 && (
                        <div className="search-autocomplete" role="listbox">
                            {acSuggestions.map((sug, i) => (
                                <div
                                    key={sug}
                                    className={`ac-item${i === acActiveIndex ? ' active' : ''}`}
                                    role="option"
                                    aria-selected={i === acActiveIndex}
                                    onMouseDown={(e) => {
                                        e.preventDefault();
                                        applySearchCompletion(sug);
                                    }}
                                >
                                    {sug}
                                </div>
                            ))}
                        </div>
                    )}
                </div>
                <label className="checkbox-field">
                    <input
                        type="checkbox"
                        checked={unpostedOnly}
                        onChange={(e) => {
                            setUnpostedOnly(e.target.checked);
                        }}
                    />
                    <span>Unposted only</span>
                </label>
                {(transactionsSearch.trim() || unpostedOnly) && (
                    <button
                        className="ghost-button"
                        onClick={() => {
                            setTransactionsSearch('');
                            setUnpostedOnly(false);
                            setQueryResults(null);
                            setQueryError(null);
                        }}
                    >
                        Clear filter
                    </button>
                )}
            </div>
            {queryError !== null && (
                <div className="query-error">{queryError}</div>
            )}
            <section className="txn-form">
                <button
                    className="txn-form-toggle"
                    aria-expanded={isNewTxnExpanded}
                    type="button"
                    onClick={() => {
                        setIsNewTxnExpandedOverride(!isNewTxnExpanded);
                    }}
                >
                    <span className="toggle-chevron">
                        {isNewTxnExpanded ? '▾' : '▸'}
                    </span>
                    New transaction
                </button>
                {isNewTxnExpanded && (
                    <div className="txn-form-body">
                        <div className="txn-form-header">
                            <div>
                                <p>
                                    Amounts accept hledger syntax (for costs or
                                    balance assertions); comments can hold tags.
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
                                            setTransactionDraft((current) => ({
                                                date: current.date,
                                                description:
                                                    last.descriptionRaw.trim()
                                                        .length > 0
                                                        ? last.descriptionRaw
                                                        : '',
                                                comment: last.comment,
                                                postings: last.postings.map(
                                                    (posting) => ({
                                                        account:
                                                            posting.account,
                                                        amount:
                                                            posting.amount ??
                                                            '',
                                                        comment:
                                                            posting.comment,
                                                    }),
                                                ),
                                            }));
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
                                            value={transactionDraft.date}
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
                                            value={transactionDraft.description}
                                            placeholder="Description"
                                            onChange={(event) => {
                                                const value =
                                                    event.target.value;
                                                setTransactionDraft(
                                                    (current) => ({
                                                        ...current,
                                                        description: value,
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
                                            value={transactionDraft.comment}
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
                                            .map((account) => account.name)
                                            .filter(
                                                (name, index, names) =>
                                                    names.indexOf(name) ===
                                                    index,
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
                                                    value={posting.account}
                                                    placeholder="Account"
                                                    list="account-options"
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
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
                                                        setAddStatus(null);
                                                        setDraftStatus(null);
                                                    }}
                                                />
                                                <input
                                                    type="text"
                                                    value={posting.amount}
                                                    placeholder="Amount (optional, supports assertions)"
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
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
                                                        setAddStatus(null);
                                                        setDraftStatus(null);
                                                    }}
                                                />
                                                <input
                                                    type="text"
                                                    value={posting.comment}
                                                    placeholder="Notes / tags"
                                                    onChange={(event) => {
                                                        const value =
                                                            event.target.value;
                                                        setTransactionDraft(
                                                            (current) => ({
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
                                                        setAddStatus(null);
                                                        setDraftStatus(null);
                                                    }}
                                                />
                                                <button
                                                    className="icon-button"
                                                    type="button"
                                                    disabled={
                                                        transactionDraft
                                                            .postings.length <=
                                                        2
                                                    }
                                                    onClick={() => {
                                                        if (
                                                            transactionDraft
                                                                .postings
                                                                .length <= 2
                                                        ) {
                                                            return;
                                                        }
                                                        setTransactionDraft(
                                                            (current) => ({
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
                                                        setAddStatus(null);
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
                                            setTransactionDraft((current) => ({
                                                ...current,
                                                postings: [
                                                    ...current.postings,
                                                    {
                                                        account: '',
                                                        amount: '',
                                                        comment: '',
                                                    },
                                                ],
                                            }));
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
                                            const value = event.target.value;
                                            setRawDraft(value);
                                            setAddStatus(null);
                                            setDraftStatus(null);
                                        }}
                                    />
                                </label>
                                <p className="hint">
                                    Accepts full hledger syntax (status, code,
                                    tags, balance assertions, virtual postings).
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
                                {isAdding ? 'Adding...' : 'Add transaction'}
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
                    </div>
                )}
            </section>
            <TransactionsTable
                transactions={filteredTransactions}
                ledgerPath={ledgerPath}
                accountNames={ledger.accounts.map((a) => a.name)}
                glCategorySuggestions={glCategorySuggestions}
                selectedTransactionIds={selectedTransactionIds}
                onSelectedTransactionIdsChange={setSelectedTransactionIds}
                initialScrollTop={transactionsTableScrollTopRef.current}
                onScrollTopChange={(scrollTop) => {
                    transactionsTableScrollTopRef.current = scrollTop;
                    sessionRef.current = {
                        ...sessionRef.current,
                        transactionsTableScrollTop: scrollTop,
                    };
                }}
                onRecategorize={(txnId, postingIndex, newAccount) => {
                    void handleRecategorizeGlTransaction(
                        txnId,
                        postingIndex,
                        newAccount,
                    );
                }}
                onMergeTransfer={(txnId1, txnId2) => {
                    void handleMergeGlTransfer(txnId1, txnId2);
                }}
                onOpenLinkTransfer={(txnId) => {
                    setGlTransferModalSearch('');
                    setGlTransferModalTxnId(txnId);
                }}
                onBulkRecategorize={(entries, newAccount) => {
                    void handleBulkRecategorize(entries, newAccount);
                }}
                onOpenSimilarRecategorize={handleOpenSimilarRecategorize}
                hideObviousAmounts={hideObviousAmounts}
                onAddSearchTerm={appendSearchTerm}
            />
            {glTransferModalTxnId !== null &&
                ((modalTxnId) => {
                    const q = glTransferModalSearch.toLowerCase();
                    const candidates = ledger.transactions
                        .filter(
                            (t) =>
                                t.id !== glTransferModalTxnId &&
                                t.postings.some(
                                    (p) => p.account === 'Expenses:Unknown',
                                ),
                        )
                        .filter(
                            (t) =>
                                !q ||
                                t.description.toLowerCase().includes(q) ||
                                t.date.includes(q),
                        );
                    return (
                        <div
                            className="modal-overlay"
                            onClick={() => {
                                setGlTransferModalTxnId(null);
                            }}
                        >
                            <div
                                className="modal-dialog"
                                onClick={(e) => {
                                    e.stopPropagation();
                                }}
                            >
                                <div className="modal-header">
                                    <h3>Link Transfer</h3>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            setGlTransferModalTxnId(null);
                                        }}
                                    >
                                        Close
                                    </button>
                                </div>
                                <input
                                    type="search"
                                    placeholder="Search transactions…"
                                    value={glTransferModalSearch}
                                    onChange={(e) => {
                                        setGlTransferModalSearch(
                                            e.target.value,
                                        );
                                    }}
                                />
                                <div className="table-wrap">
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Date</th>
                                                <th>Description</th>
                                                <th>Amount</th>
                                                <th />
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {candidates.length === 0 ? (
                                                <tr>
                                                    <td
                                                        colSpan={4}
                                                        className="table-empty"
                                                    >
                                                        No uncategorized
                                                        transactions found.
                                                    </td>
                                                </tr>
                                            ) : (
                                                candidates.map((t) => (
                                                    <tr key={t.id}>
                                                        <td className="mono">
                                                            {t.date}
                                                        </td>
                                                        <td>{t.description}</td>
                                                        <td className="amount">
                                                            {formatTotals(
                                                                t.totals,
                                                            )}
                                                        </td>
                                                        <td>
                                                            <button
                                                                type="button"
                                                                className="ghost-button"
                                                                onClick={() => {
                                                                    void handleMergeGlTransfer(
                                                                        modalTxnId,
                                                                        t.id,
                                                                    );
                                                                    setGlTransferModalTxnId(
                                                                        null,
                                                                    );
                                                                }}
                                                            >
                                                                Link
                                                            </button>
                                                        </td>
                                                    </tr>
                                                ))
                                            )}
                                        </tbody>
                                    </table>
                                </div>
                            </div>
                        </div>
                    );
                })(glTransferModalTxnId)}
        </div>
    );
}
