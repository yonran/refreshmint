import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { Menu, MenuItem, Submenu } from '@tauri-apps/api/menu';
import { documentDir, join } from '@tauri-apps/api/path';
import {
    confirm as confirmDialog,
    open as openDialog,
    save as saveDialog,
} from '@tauri-apps/plugin-dialog';
import './App.css';
import {
    type ActiveTab,
    addRecentLedger,
    getLastActiveTab,
    getRecentLedgers,
    removeRecentLedger,
    setLastActiveTab,
    setRecentLedgers,
} from './store.ts';
import {
    type AmountStyleHint,
    type AmountTotal,
    addTransaction,
    addTransactionText,
    getLoginConfig,
    type LoginConfig,
    listLogins,
    openLedger,
    type AccountRow,
    type LedgerView,
    type NewTransactionInput,
    type PostingRow,
    readAttachmentDataUrl,
    setLoginAccount,
    suggestGlCategories,
    recategorizeGlTransaction,
    mergeGlTransfer,
    type GlCategoryResult,
    type TransactionRow,
    validateTransaction,
    validateTransactionText,
    queryTransactions,
} from './tauri-commands.ts';
import {
    checkAccountTypeChange,
    getAccountSuggestions,
    getCurrentToken,
    getSearchSuggestions,
    quoteHledgerValue,
} from './search-utils.ts';
import { PipelineTab } from './tabs/PipelineTab.tsx';
import { ReportsTab } from './tabs/ReportsTab.tsx';
import { ScrapeTab } from './tabs/ScrapeTab.tsx';
import {
    type TransactionDraft,
    type TransactionEntryMode,
    type SecretPromptState,
    type LoginAccountMapping,
    type LoginAccountRef,
    type SimilarRecategorizePlan,
    type RecategorizeTab,
    normalizeLoginConfig,
} from './types.ts';

type AppTab = ActiveTab | `recategorize:${number}`;

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
    const [activeTab, setActiveTab] = useState<AppTab>('accounts');
    const [hideObviousAmounts, setHideObviousAmounts] = useState(
        () => localStorage.getItem('pref:hideObviousAmounts') !== 'false',
    );
    const [selectedAccount, setSelectedAccount] = useState<string | null>(null);
    const [unpostedOnly, setUnpostedOnly] = useState(false);
    const [loginAccounts, setLoginAccounts] = useState<LoginAccountRef[]>([]);

    function handleSelectAccount(accountName: string) {
        setSelectedAccount((current) =>
            current === accountName ? null : accountName,
        );
        setActiveTab('transactions');
    }
    const [recentLedgers, setRecentLedgersState] = useState<string[]>([]);
    const [transactionDraft, setTransactionDraft] = useState<TransactionDraft>(
        createTransactionDraft,
    );
    const [rawDraft, setRawDraft] = useState('');
    const [entryMode, setEntryMode] = useState<TransactionEntryMode>('form');
    const [addStatus, setAddStatus] = useState<string | null>(null);
    const [isAdding, setIsAdding] = useState(false);
    const [draftStatus, setDraftStatus] = useState<string | null>(null);
    const [isValidatingDraft, setIsValidatingDraft] = useState(false);
    const [loginNames, setLoginNames] = useState<string[]>([]);
    const [loginConfigsByName, setLoginConfigsByName] = useState<
        Record<string, LoginConfig>
    >({});
    const [loginManagementTab, setLoginManagementTab] = useState<
        'select' | 'create'
    >('select');
    const [selectedLoginName, setSelectedLoginName] = useState('');
    const [loginLabelDraft, setLoginLabelDraft] = useState('');
    const [loginGlAccountDraft, setLoginGlAccountDraft] = useState('');
    const [loginConfigStatus, setLoginConfigStatus] = useState<string | null>(
        null,
    );
    const [isLoadingLoginConfigs, setIsLoadingLoginConfigs] = useState(false);
    const [isSavingLoginConfig, setIsSavingLoginConfig] = useState(false);
    const [hasLoadedLoginConfigs, setHasLoadedLoginConfigs] = useState(false);
    const [loginConfigsReloadToken, setLoginConfigsReloadToken] = useState(0);
    const [loginAccountMappings, setLoginAccountMappings] = useState<
        Record<string, LoginAccountMapping[]>
    >({});
    const [transactionsSearch, setTransactionsSearch] = useState('');
    const [queryResults, setQueryResults] = useState<TransactionRow[] | null>(
        null,
    );
    const [queryError, setQueryError] = useState<string | null>(null);
    const [recategorizeTabs, setRecategorizeTabs] = useState<RecategorizeTab[]>(
        [],
    );
    const [isNewTxnExpandedOverride, setIsNewTxnExpandedOverride] = useState<
        boolean | null
    >(null);
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
    >(null);
    const [glTransferModalSearch, setGlTransferModalSearch] = useState('');
    const [secretPrompt, setSecretPrompt] = useState<SecretPromptState | null>(
        null,
    );
    const updateRecentLedgers = useCallback(
        (updater: (current: string[]) => string[]) => {
            setRecentLedgersState((current) => {
                const next = updater(current);
                void setRecentLedgers(next);
                return next;
            });
        },
        [],
    );
    const recordRecentLedger = useCallback(
        (path: string) => {
            updateRecentLedgers((current) => addRecentLedger(current, path));
        },
        [updateRecentLedgers],
    );
    const pruneRecentLedger = useCallback(
        (path: string) => {
            updateRecentLedgers((current) => removeRecentLedger(current, path));
        },
        [updateRecentLedgers],
    );
    const menuHandlers = useRef({
        openLedger: () => {},
        newLedger: () => {},
        openRecent: (_path: string) => {},
    });
    const startupCancelledRef = useRef(false);
    const recategorizeTabIdRef = useRef(1);
    const searchInputRef = useRef<HTMLInputElement>(null);
    const similarSearchInputRef = useRef<HTMLInputElement>(null);
    const secretPromptResolverRef = useRef<
        ((confirmed: boolean) => void) | null
    >(null);
    const ledgerPath = ledger?.path ?? null;
    const activeRecategorizeTabId = activeTab.startsWith('recategorize:')
        ? Number.parseInt(activeTab.slice('recategorize:'.length), 10)
        : null;
    const activeRecategorizeTab =
        activeRecategorizeTabId === null
            ? null
            : (recategorizeTabs.find(
                  (tab) => tab.id === activeRecategorizeTabId,
              ) ?? null);

    const filteredTransactions = (() => {
        if (!ledger) return [];
        const base = queryResults ?? ledger.transactions;
        return base.filter((txn) => {
            if (
                selectedAccount !== null &&
                !txn.postings.some((p) => p.account === selectedAccount)
            ) {
                return false;
            }
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
        isNewTxnExpandedOverride ??
        (ledger !== null && ledger.transactions.length === 0);

    const selectedAccountRow =
        selectedAccount !== null
            ? ledger?.accounts.find((a) => a.name === selectedAccount)
            : null;

    const computedGlAccountConflicts = Object.entries(loginConfigsByName)
        .flatMap(([loginName, config]) =>
            Object.entries(normalizeLoginConfig(config).accounts)
                .map(([label, accountConfig]) => ({
                    loginName,
                    label,
                    glAccount: accountConfig.glAccount?.trim() ?? '',
                }))
                .filter((entry) => entry.glAccount.length > 0),
        )
        .reduce((map, entry) => {
            const current = map.get(entry.glAccount) ?? [];
            map.set(entry.glAccount, [
                ...current,
                { loginName: entry.loginName, label: entry.label },
            ]);
            return map;
        }, new Map<string, Array<{ loginName: string; label: string }>>());
    const glAccountConflicts = hasLoadedLoginConfigs
        ? Array.from(computedGlAccountConflicts.entries())
              .filter(([, entries]) => entries.length > 1)
              .map(([glAccount, entries]) => ({
                  glAccount,
                  entries: [...entries].sort((a, b) =>
                      a.loginName === b.loginName
                          ? a.label.localeCompare(b.label)
                          : a.loginName.localeCompare(b.loginName),
                  ),
              }))
              .sort((a, b) => a.glAccount.localeCompare(b.glAccount))
        : (ledger?.glAccountConflicts ?? []);
    const conflictingGlAccountSet = new Set(
        glAccountConflicts.map((conflict) => conflict.glAccount),
    );
    const requestLoginConfigReload = useCallback(() => {
        setLoginConfigsReloadToken((current) => current + 1);
    }, []);
    useEffect(() => {
        return () => {
            if (secretPromptResolverRef.current !== null) {
                secretPromptResolverRef.current(false);
                secretPromptResolverRef.current = null;
            }
        };
    }, []);

    useEffect(() => {
        if (ledgerPath !== null) {
            setTransactionDraft(createTransactionDraft());
            setRawDraft('');
            setAddStatus(null);
            setDraftStatus(null);
            setLoginNames([]);
            setLoginConfigsByName({});
            setLoginManagementTab('select');
            setSelectedLoginName('');
            setLoginLabelDraft('');
            setLoginGlAccountDraft('');
            setLoginConfigStatus(null);
            setIsLoadingLoginConfigs(false);
            setIsSavingLoginConfig(false);
            setHasLoadedLoginConfigs(false);
            setLoginConfigsReloadToken(0);
            setLoginAccountMappings({});
            setQueryResults(null);
            setQueryError(null);
            setIsNewTxnExpandedOverride(null);
            setTransactionsSearch('');
            setAcSuggestions([]);
            setAcActiveIndex(-1);
            setRecategorizeTabs([]);
            setSimilarAcSuggestions([]);
            setSimilarAcActiveIndex(-1);
        }
    }, [ledgerPath]);

    useEffect(() => {
        if (loginNames.length === 0) {
            setLoginManagementTab('create');
        }
    }, [loginNames]);

    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (
                (e.metaKey || e.ctrlKey) &&
                e.key === 'f' &&
                activeTab === 'transactions'
            ) {
                e.preventDefault();
                searchInputRef.current?.focus();
                searchInputRef.current?.select();
            }
            if ((e.metaKey || e.ctrlKey) && e.key === ',') {
                e.preventDefault();
                setActiveTab('preferences');
            }
        };
        window.addEventListener('keydown', handler);
        return () => {
            window.removeEventListener('keydown', handler);
        };
    }, [activeTab]);

    useEffect(() => {
        const q = transactionsSearch.trim();
        if (!q || !ledger) {
            setQueryResults(null);
            setQueryError(null);
            return;
        }
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const rows = await queryTransactions(ledger.path, q);
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
    }, [transactionsSearch, ledger]);

    useEffect(() => {
        if (activeRecategorizeTab === null || !ledger) {
            return;
        }
        const q = activeRecategorizeTab.plan.searchQuery.trim();
        if (!q) {
            setRecategorizeTabs((current) =>
                current.map((tab) =>
                    tab.id === activeRecategorizeTab.id
                        ? { ...tab, queryResults: null, queryError: null }
                        : tab,
                ),
            );
            return;
        }
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const rows = await queryTransactions(ledger.path, q);
                    setRecategorizeTabs((current) =>
                        current.map((tab) =>
                            tab.id === activeRecategorizeTab.id
                                ? {
                                      ...tab,
                                      queryResults: rows,
                                      queryError: null,
                                  }
                                : tab,
                        ),
                    );
                } catch (err) {
                    setRecategorizeTabs((current) =>
                        current.map((tab) =>
                            tab.id === activeRecategorizeTab.id
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
    }, [activeRecategorizeTab, ledger]);

    useEffect(() => {
        if (
            activeRecategorizeTabId !== null &&
            !recategorizeTabs.some((tab) => tab.id === activeRecategorizeTabId)
        ) {
            setActiveTab('transactions');
        }
    }, [activeRecategorizeTabId, recategorizeTabs]);

    useEffect(() => {
        if (ledgerPath === null) {
            setLoginNames([]);
            setLoginConfigsByName({});
            setSelectedLoginName('');
            setLoginAccountMappings({});
            setIsLoadingLoginConfigs(false);
            setHasLoadedLoginConfigs(false);
            return;
        }

        let cancelled = false;
        setIsLoadingLoginConfigs(true);
        void listLogins(ledgerPath)
            .then(async (logins) => {
                const configs = await Promise.all(
                    logins.map(async (loginName) => ({
                        loginName,
                        config: await getLoginConfig(ledgerPath, loginName),
                    })),
                );
                if (cancelled) {
                    return;
                }

                const configMap: Record<string, LoginConfig> = {};
                const mappings: Record<string, LoginAccountMapping[]> = {};
                const accounts: LoginAccountRef[] = [];
                for (const { loginName, config } of configs) {
                    const normalizedConfig = normalizeLoginConfig(config);
                    configMap[loginName] = normalizedConfig;
                    const extension = normalizedConfig.extension?.trim() ?? '';
                    for (const [label, mapping] of Object.entries(
                        normalizedConfig.accounts,
                    )) {
                        accounts.push({ loginName, label });
                        const glAccount = mapping.glAccount?.trim() ?? '';
                        if (glAccount.length === 0) {
                            continue;
                        }
                        const next: LoginAccountMapping = {
                            loginName,
                            label,
                            extension,
                        };
                        const current = mappings[glAccount] ?? [];
                        mappings[glAccount] = [...current, next];
                    }
                }
                setLoginNames(logins);
                setLoginConfigsByName(configMap);
                setLoginAccounts(
                    accounts.sort((a, b) =>
                        a.loginName === b.loginName
                            ? a.label.localeCompare(b.label)
                            : a.loginName.localeCompare(b.loginName),
                    ),
                );
                setSelectedLoginName((current) => {
                    if (current.length > 0 && logins.includes(current)) {
                        return current;
                    }
                    return '';
                });
                setLoginAccountMappings(mappings);
                setHasLoadedLoginConfigs(true);
            })
            .catch((error: unknown) => {
                if (!cancelled) {
                    setLoginNames([]);
                    setLoginConfigsByName({});
                    setSelectedLoginName('');
                    setLoginAccountMappings({});
                    setHasLoadedLoginConfigs(false);
                    setLoginConfigStatus(
                        `Failed to load login configs: ${String(error)}`,
                    );
                }
            })
            .finally(() => {
                if (!cancelled) {
                    setIsLoadingLoginConfigs(false);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [ledger, ledgerPath, loginConfigsReloadToken]);

    useEffect(() => {
        startupCancelledRef.current = false;
        const isStartupCancelled = () => startupCancelledRef.current;

        async function startup() {
            const storedTab = await getLastActiveTab();
            if (isStartupCancelled()) {
                return;
            }
            if (storedTab) {
                setActiveTab(storedTab);
            }

            const storedRecents = await getRecentLedgers();
            if (isStartupCancelled()) {
                return;
            }
            setRecentLedgersState(storedRecents);

            if (storedRecents.length === 0) {
                return;
            }

            setIsOpening(true);
            setOpenStatus('Opening recent ledger...');
            try {
                for (const path of storedRecents) {
                    try {
                        const opened = await openLedger(path);
                        if (isStartupCancelled()) {
                            return;
                        }
                        setLedger(opened);
                        recordRecentLedger(path);
                        setOpenStatus(`Opened ${opened.path}`);
                        return;
                    } catch {
                        if (!isStartupCancelled()) {
                            pruneRecentLedger(path);
                        }
                    }
                }
                setOpenStatus(null);
            } finally {
                if (!isStartupCancelled()) {
                    setIsOpening(false);
                }
            }
        }

        void startup();

        return () => {
            startupCancelledRef.current = true;
        };
    }, [pruneRecentLedger, recordRecentLedger]);

    useEffect(() => {
        if (!ledger) {
            return;
        }
        const persistedTab: ActiveTab =
            activeTab === 'accounts' ||
            activeTab === 'transactions' ||
            activeTab === 'scrape' ||
            activeTab === 'pipeline' ||
            activeTab === 'reports' ||
            activeTab === 'preferences'
                ? activeTab
                : 'transactions';
        void setLastActiveTab(persistedTab);
    }, [activeTab, ledger]);

    // Load GL category suggestions whenever the Transactions tab is active.
    useEffect(() => {
        if (activeTab !== 'transactions' || !ledger) {
            return;
        }
        const ledgerPath = ledger.path;
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
    }, [activeTab, ledger]);

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
            const path: string | null = await openDialog({
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
            });
            if (path === null) {
                setOpenStatus(null);
                return;
            }
            if (path.length === 0) {
                setOpenStatus('Open canceled.');
                return;
            }
            setOpenStatus('Opening ledger...');
            const opened = await openLedger(path);
            setLedger(opened);
            recordRecentLedger(path);
            setOpenStatus(`Opened ${opened.path}`);
        } catch (error) {
            setOpenStatus(`Failed to open ledger: ${String(error)}`);
        } finally {
            setIsOpening(false);
        }
    }

    async function handleOpenRecent(path: string) {
        if (path.length === 0) {
            return;
        }
        setIsOpening(true);
        setOpenStatus('Opening ledger...');
        try {
            const opened = await openLedger(path);
            setLedger(opened);
            recordRecentLedger(path);
            setOpenStatus(`Opened ${opened.path}`);
        } catch (error) {
            pruneRecentLedger(path);
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

    function promptSecretDecision(prompt: SecretPromptState): Promise<boolean> {
        if (secretPromptResolverRef.current !== null) {
            secretPromptResolverRef.current(false);
            secretPromptResolverRef.current = null;
        }
        setSecretPrompt(prompt);
        return new Promise((resolve) => {
            secretPromptResolverRef.current = resolve;
        });
    }

    function resolveSecretPrompt(confirmed: boolean) {
        const resolve = secretPromptResolverRef.current;
        secretPromptResolverRef.current = null;
        setSecretPrompt(null);
        resolve?.(confirmed);
    }

    function handleLedgerRefresh() {
        if (!ledger) return;
        openLedger(ledger.path)
            .then((reloaded) => {
                setLedger(reloaded);
            })
            .catch((err: unknown) => {
                console.error('ledger reload failed:', err);
            });
    }

    function handleLoadConflictMapping(
        loginName: string,
        label: string,
        glAccount: string,
    ) {
        setActiveTab('scrape');
        setSelectedLoginName(loginName);
        setLoginLabelDraft(label);
        setLoginGlAccountDraft(glAccount);
        setLoginConfigStatus(
            `Loaded '${loginName}/${label}' from conflicts. Update GL account or clear it to resolve.`,
        );
    }

    async function handleIgnoreLoginAccountMapping(
        loginName: string,
        label: string,
        glAccount: string,
    ) {
        if (!ledger) {
            return;
        }
        const shouldIgnore = await confirmDialog(
            `Set '${loginName}/${label}' to ignored for '${glAccount}'?`,
            {
                title: 'Ignore mapping?',
                kind: 'warning',
                okLabel: 'Ignore',
                cancelLabel: 'Cancel',
            },
        );
        if (!shouldIgnore) {
            return;
        }

        setIsSavingLoginConfig(true);
        try {
            await setLoginAccount(ledger.path, loginName, label, null);
            setActiveTab('scrape');
            setSelectedLoginName(loginName);
            setLoginLabelDraft(label);
            setLoginGlAccountDraft('');
            setLoginConfigStatus(
                `Set '${loginName}/${label}' to ignored (removed GL account '${glAccount}').`,
            );
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to ignore mapping: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
    }

    async function handleRecategorizeGlTransaction(
        txnId: string,
        oldAccount: string,
        newAccount: string,
    ) {
        if (!ledger) return;
        try {
            await recategorizeGlTransaction(
                ledger.path,
                txnId,
                oldAccount,
                newAccount,
            );
            const [reloaded, suggestions] = await Promise.all([
                openLedger(ledger.path),
                suggestGlCategories(ledger.path),
            ]);
            setLedger(reloaded);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('recategorize failed:', error);
        }
    }

    async function handleMergeGlTransfer(txnId1: string, txnId2: string) {
        if (!ledger) return;
        try {
            await mergeGlTransfer(ledger.path, txnId1, txnId2);
            const [reloaded, suggestions] = await Promise.all([
                openLedger(ledger.path),
                suggestGlCategories(ledger.path),
            ]);
            setLedger(reloaded);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('merge transfer failed:', error);
        }
    }

    async function handleBulkRecategorize(
        entries: Array<{ txnId: string; oldAccount: string }>,
        newAccount: string,
    ) {
        if (!ledger) return;
        try {
            for (const { txnId, oldAccount } of entries) {
                await recategorizeGlTransaction(
                    ledger.path,
                    txnId,
                    oldAccount,
                    newAccount,
                );
            }
            const [reloaded, suggestions] = await Promise.all([
                openLedger(ledger.path),
                suggestGlCategories(ledger.path),
            ]);
            setLedger(reloaded);
            setGlCategorySuggestions(suggestions);
        } catch (error) {
            console.error('bulk recategorize failed:', error);
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

    menuHandlers.current = {
        openLedger: () => {
            void handleOpenLedger();
        },
        newLedger: () => {
            void handleNewLedger();
        },
        openRecent: (path: string) => {
            void handleOpenRecent(path);
        },
    };

    useEffect(() => {
        let cancelled = false;

        async function setupMenu() {
            const newItem = await MenuItem.new({
                text: 'New...',
                accelerator: 'CmdOrCtrl+N',
                action: () => {
                    menuHandlers.current.newLedger();
                },
            });
            const openItem = await MenuItem.new({
                text: 'Open...',
                accelerator: 'CmdOrCtrl+O',
                action: () => {
                    menuHandlers.current.openLedger();
                },
            });
            const recentItems = await Promise.all(
                recentLedgers.slice(0, 10).map((path) =>
                    MenuItem.new({
                        text: path,
                        action: () => {
                            menuHandlers.current.openRecent(path);
                        },
                    }),
                ),
            );
            if (recentItems.length === 0) {
                recentItems.push(
                    await MenuItem.new({
                        text: 'No recent files',
                        enabled: false,
                    }),
                );
            }
            const openRecent = await Submenu.new({
                text: 'Open Recent',
                items: recentItems,
            });
            const menu = await Menu.default();
            const topLevelItems = await menu.items();
            let fileMenu: Submenu | null = null;
            for (const item of topLevelItems) {
                if (item instanceof Submenu && (await item.text()) === 'File') {
                    fileMenu = item;
                    break;
                }
            }
            if (fileMenu !== null) {
                await fileMenu.insert([newItem, openItem, openRecent], 0);
            } else {
                const fallbackFileMenu = await Submenu.new({
                    text: 'File',
                    items: [newItem, openItem, openRecent],
                });
                await menu.append(fallbackFileMenu);
            }

            if (!cancelled) {
                await menu.setAsAppMenu();
            }
        }

        void setupMenu();

        return () => {
            cancelled = true;
        };
    }, [recentLedgers]);

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
            // Replacing just the prefix; keep the value after the colon
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
        setRecategorizeTabs((current) =>
            current.map((tab) =>
                tab.id === activeRecategorizeTab.id
                    ? {
                          ...tab,
                          plan: { ...tab.plan, searchQuery: newValue },
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

    return (
        <div
            className="app"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
        >
            <header className="app-header">
                <div>
                    <p className="app-eyebrow">Refreshmint</p>
                    <h1>Ledger workspace</h1>
                    <p className="app-subtitle">
                        Open a <span>.refreshmint</span> directory to review
                        accounts, transactions, and scraping extensions.
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
                    {glAccountConflicts.length === 0 ? null : (
                        <section className="txn-form">
                            <div className="txn-form-header">
                                <div>
                                    <h2>GL mapping conflicts</h2>
                                    <p>
                                        Multiple login labels map to the same GL
                                        account. Resolve these before extraction
                                        or posting.
                                    </p>
                                </div>
                            </div>
                            <p className="status">
                                {glAccountConflicts.length} conflicting GL
                                account mapping(s) detected.
                            </p>
                            <div className="table-wrap">
                                <table className="ledger-table">
                                    <thead>
                                        <tr>
                                            <th>GL Account</th>
                                            <th>Login/Label</th>
                                            <th>Actions</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {glAccountConflicts.flatMap(
                                            (conflict) =>
                                                conflict.entries.map(
                                                    (entry, index) => (
                                                        <tr
                                                            key={`${conflict.glAccount}/${entry.loginName}/${entry.label}`}
                                                        >
                                                            <td className="mono">
                                                                {index === 0
                                                                    ? conflict.glAccount
                                                                    : ''}
                                                            </td>
                                                            <td className="mono">
                                                                {
                                                                    entry.loginName
                                                                }
                                                                /{entry.label}
                                                            </td>
                                                            <td>
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    onClick={() => {
                                                                        handleLoadConflictMapping(
                                                                            entry.loginName,
                                                                            entry.label,
                                                                            conflict.glAccount,
                                                                        );
                                                                    }}
                                                                >
                                                                    Load in
                                                                    mappings
                                                                </button>
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    disabled={
                                                                        isSavingLoginConfig
                                                                    }
                                                                    onClick={() => {
                                                                        void handleIgnoreLoginAccountMapping(
                                                                            entry.loginName,
                                                                            entry.label,
                                                                            conflict.glAccount,
                                                                        );
                                                                    }}
                                                                >
                                                                    Ignore
                                                                    mapping
                                                                </button>
                                                            </td>
                                                        </tr>
                                                    ),
                                                ),
                                        )}
                                    </tbody>
                                </table>
                            </div>
                        </section>
                    )}
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
                        {recategorizeTabs.map((tab) => {
                            const tabKey = `recategorize:${tab.id}` as const;
                            return (
                                <button
                                    key={tab.id}
                                    className={
                                        activeTab === tabKey
                                            ? 'tab active'
                                            : 'tab'
                                    }
                                    onClick={() => {
                                        setActiveTab(tabKey);
                                    }}
                                    type="button"
                                >
                                    Recategorize {tab.id}
                                </button>
                            );
                        })}
                        <button
                            className={
                                activeTab === 'pipeline' ? 'tab active' : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('pipeline');
                            }}
                            type="button"
                        >
                            Pipeline
                        </button>
                        <button
                            className={
                                activeTab === 'reports' ? 'tab active' : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('reports');
                            }}
                            type="button"
                        >
                            Reports
                        </button>
                        <button
                            className={
                                activeTab === 'preferences'
                                    ? 'tab active'
                                    : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('preferences');
                            }}
                            type="button"
                        >
                            Preferences
                        </button>
                        <button
                            className={
                                activeTab === 'scrape' ? 'tab active' : 'tab'
                            }
                            onClick={() => {
                                setActiveTab('scrape');
                            }}
                            type="button"
                        >
                            Scraping
                        </button>
                    </div>

                    {activeTab === 'accounts' ? (
                        <div className="table-wrap">
                            <AccountsTable
                                accounts={ledger.accounts}
                                onSelectAccount={handleSelectAccount}
                            />
                        </div>
                    ) : activeTab === 'transactions' ? (
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
                                            setAcSuggestions(sugs);
                                            setAcActiveIndex(-1);
                                        }}
                                        onKeyDown={(e) => {
                                            if (acSuggestions.length === 0)
                                                return;
                                            if (e.key === 'ArrowDown') {
                                                e.preventDefault();
                                                setAcActiveIndex((i) =>
                                                    Math.min(
                                                        i + 1,
                                                        acSuggestions.length -
                                                            1,
                                                    ),
                                                );
                                            } else if (e.key === 'ArrowUp') {
                                                e.preventDefault();
                                                setAcActiveIndex((i) =>
                                                    Math.max(i - 1, 0),
                                                );
                                            } else if (
                                                (e.key === 'Enter' ||
                                                    e.key === 'Tab') &&
                                                acActiveIndex >= 0
                                            ) {
                                                e.preventDefault();
                                                applySearchCompletion(
                                                    acSuggestions[
                                                        acActiveIndex
                                                    ] ?? '',
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
                                        <div
                                            className="search-autocomplete"
                                            role="listbox"
                                        >
                                            {acSuggestions.map((sug, i) => (
                                                <div
                                                    key={sug}
                                                    className={`ac-item${i === acActiveIndex ? ' active' : ''}`}
                                                    role="option"
                                                    aria-selected={
                                                        i === acActiveIndex
                                                    }
                                                    onMouseDown={(e) => {
                                                        e.preventDefault();
                                                        applySearchCompletion(
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
                                {(transactionsSearch.trim() ||
                                    unpostedOnly ||
                                    selectedAccount !== null) && (
                                    <button
                                        className="ghost-button"
                                        onClick={() => {
                                            setTransactionsSearch('');
                                            setUnpostedOnly(false);
                                            setSelectedAccount(null);
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
                            {selectedAccount !== null ? (
                                <div className="filter-header">
                                    <div className="filter-info">
                                        <span className="filter-label">
                                            Filtered by account:
                                        </span>
                                        <span className="filter-value mono">
                                            {selectedAccount}
                                        </span>
                                        {selectedAccountRow &&
                                        selectedAccountRow.unpostedCount > 0 ? (
                                            <span className="secret-chip warning">
                                                {
                                                    selectedAccountRow.unpostedCount
                                                }{' '}
                                                unposted
                                            </span>
                                        ) : null}
                                    </div>
                                    <div className="filter-actions">
                                        <button
                                            className="ghost-button"
                                            onClick={() => {
                                                setSelectedAccount(null);
                                            }}
                                        >
                                            Clear account filter
                                        </button>
                                    </div>
                                </div>
                            ) : null}
                            <section className="txn-form">
                                <button
                                    className="txn-form-toggle"
                                    aria-expanded={isNewTxnExpanded}
                                    type="button"
                                    onClick={() => {
                                        setIsNewTxnExpandedOverride(
                                            !isNewTxnExpanded,
                                        );
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
                                                    Amounts accept hledger
                                                    syntax (for costs or balance
                                                    assertions); comments can
                                                    hold tags.
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
                                                            setEntryMode(
                                                                'form',
                                                            );
                                                            setAddStatus(null);
                                                            setDraftStatus(
                                                                null,
                                                            );
                                                            setIsValidatingDraft(
                                                                false,
                                                            );
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
                                                            setDraftStatus(
                                                                null,
                                                            );
                                                            setIsValidatingDraft(
                                                                false,
                                                            );
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
                                                                            .length >
                                                                        0
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
                                                            setDraftStatus(
                                                                null,
                                                            );
                                                        }}
                                                    >
                                                        Copy last
                                                    </button>
                                                ) : null}
                                                <button
                                                    className="ghost-button"
                                                    type="button"
                                                    onClick={() => {
                                                        if (
                                                            entryMode === 'raw'
                                                        ) {
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
                                                                        date: value,
                                                                    }),
                                                                );
                                                                setAddStatus(
                                                                    null,
                                                                );
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
                                                                        description:
                                                                            value,
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
                                                    </label>
                                                    <label className="field">
                                                        <span>
                                                            Notes / tags
                                                        </span>
                                                        <input
                                                            type="text"
                                                            value={
                                                                transactionDraft.comment
                                                            }
                                                            placeholder="tag:food, note:..."
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
                                                                        comment:
                                                                            value,
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
                                                                (
                                                                    name,
                                                                    index,
                                                                    names,
                                                                ) =>
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
                                                                            event
                                                                                .target
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
                                                                            event
                                                                                .target
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
                                                                            event
                                                                                .target
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
                                                                            .length <=
                                                                        2
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
                                                                            account:
                                                                                '',
                                                                            amount: '',
                                                                            comment:
                                                                                '',
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
                                                                event.target
                                                                    .value;
                                                            setRawDraft(value);
                                                            setAddStatus(null);
                                                            setDraftStatus(
                                                                null,
                                                            );
                                                        }}
                                                    />
                                                </label>
                                                <p className="hint">
                                                    Accepts full hledger syntax
                                                    (status, code, tags, balance
                                                    assertions, virtual
                                                    postings).
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
                                            <p className="status">
                                                Checking draft...
                                            </p>
                                        ) : null}
                                        {draftStatus === null ? null : (
                                            <p className="status">
                                                {draftStatus}
                                            </p>
                                        )}
                                        {addStatus === null ? null : (
                                            <p className="status">
                                                {addStatus}
                                            </p>
                                        )}
                                    </div>
                                )}
                            </section>
                            <TransactionsTable
                                transactions={filteredTransactions}
                                allTransactions={ledger.transactions}
                                ledgerPath={ledgerPath}
                                accountNames={ledger.accounts.map(
                                    (a) => a.name,
                                )}
                                glCategorySuggestions={glCategorySuggestions}
                                onRecategorize={(
                                    txnId,
                                    oldAccount,
                                    newAccount,
                                ) => {
                                    void handleRecategorizeGlTransaction(
                                        txnId,
                                        oldAccount,
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
                                    void handleBulkRecategorize(
                                        entries,
                                        newAccount,
                                    );
                                }}
                                onOpenSimilarRecategorize={(plan) => {
                                    const id = recategorizeTabIdRef.current++;
                                    setRecategorizeTabs((current) => [
                                        ...current,
                                        {
                                            id,
                                            plan,
                                            queryResults: null,
                                            queryError: null,
                                        },
                                    ]);
                                    setSimilarAcSuggestions([]);
                                    setSimilarAcActiveIndex(-1);
                                    setActiveTab(`recategorize:${id}`);
                                }}
                                hideObviousAmounts={hideObviousAmounts}
                                onAddSearchTerm={appendSearchTerm}
                            />
                            {glTransferModalTxnId !== null &&
                                ((modalTxnId) => {
                                    const q =
                                        glTransferModalSearch.toLowerCase();
                                    const candidates = ledger.transactions
                                        .filter(
                                            (t) =>
                                                t.id !== glTransferModalTxnId &&
                                                t.postings.some(
                                                    (p) =>
                                                        p.account ===
                                                        'Expenses:Unknown',
                                                ),
                                        )
                                        .filter(
                                            (t) =>
                                                !q ||
                                                t.description
                                                    .toLowerCase()
                                                    .includes(q) ||
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
                                                            setGlTransferModalTxnId(
                                                                null,
                                                            );
                                                        }}
                                                    >
                                                        Close
                                                    </button>
                                                </div>
                                                <input
                                                    type="search"
                                                    placeholder="Search transactions…"
                                                    value={
                                                        glTransferModalSearch
                                                    }
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
                                                                <th>
                                                                    Description
                                                                </th>
                                                                <th>Amount</th>
                                                                <th />
                                                            </tr>
                                                        </thead>
                                                        <tbody>
                                                            {candidates.length ===
                                                            0 ? (
                                                                <tr>
                                                                    <td
                                                                        colSpan={
                                                                            4
                                                                        }
                                                                        className="table-empty"
                                                                    >
                                                                        No
                                                                        uncategorized
                                                                        transactions
                                                                        found.
                                                                    </td>
                                                                </tr>
                                                            ) : (
                                                                candidates.map(
                                                                    (t) => (
                                                                        <tr
                                                                            key={
                                                                                t.id
                                                                            }
                                                                        >
                                                                            <td className="mono">
                                                                                {
                                                                                    t.date
                                                                                }
                                                                            </td>
                                                                            <td>
                                                                                {
                                                                                    t.description
                                                                                }
                                                                            </td>
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
                                                                    ),
                                                                )
                                                            )}
                                                        </tbody>
                                                    </table>
                                                </div>
                                            </div>
                                        </div>
                                    );
                                })(glTransferModalTxnId)}
                        </div>
                    ) : activeRecategorizeTab !== null ? (
                        <div className="transactions-panel">
                            {(() => {
                                const baseEntries = activeRecategorizeTab.plan
                                    .includeAll
                                    ? activeRecategorizeTab.plan.allEntries
                                    : activeRecategorizeTab.plan.entries;
                                const similarQueryResultIds = new Set(
                                    (
                                        activeRecategorizeTab.queryResults ?? []
                                    ).map((txn) => txn.id),
                                );
                                const matchingEntries =
                                    activeRecategorizeTab.queryResults === null
                                        ? baseEntries
                                        : baseEntries.filter((entry) =>
                                              similarQueryResultIds.has(
                                                  entry.txnId,
                                              ),
                                          );
                                const txnById = new Map(
                                    ledger.transactions.map((txn) => [
                                        txn.id,
                                        txn,
                                    ]),
                                );
                                return (
                                    <section className="txn-form">
                                        <div className="txn-form-header">
                                            <div>
                                                <h2>Recategorize</h2>
                                                <p>
                                                    Refine query and
                                                    destination, then apply to
                                                    matching rows.
                                                </p>
                                            </div>
                                            <div className="header-actions">
                                                <button
                                                    type="button"
                                                    className="ghost-button"
                                                    onClick={() => {
                                                        setActiveTab(
                                                            'transactions',
                                                        );
                                                    }}
                                                >
                                                    Back to Transactions
                                                </button>
                                                <button
                                                    type="button"
                                                    className="ghost-button"
                                                    onClick={() => {
                                                        setRecategorizeTabs(
                                                            (current) =>
                                                                current.filter(
                                                                    (tab) =>
                                                                        tab.id !==
                                                                        activeRecategorizeTab.id,
                                                                ),
                                                        );
                                                        setActiveTab(
                                                            'transactions',
                                                        );
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
                                                    activeRecategorizeTab.plan
                                                        .searchQuery
                                                }
                                                onChange={(e) => {
                                                    const val = e.target.value;
                                                    setRecategorizeTabs(
                                                        (current) =>
                                                            current.map(
                                                                (tab) =>
                                                                    tab.id ===
                                                                    activeRecategorizeTab.id
                                                                        ? {
                                                                              ...tab,
                                                                              plan: {
                                                                                  ...tab.plan,
                                                                                  searchQuery:
                                                                                      val,
                                                                              },
                                                                          }
                                                                        : tab,
                                                            ),
                                                    );
                                                    const cursorPos =
                                                        e.target
                                                            .selectionStart ??
                                                        val.length;
                                                    const { token, start } =
                                                        getCurrentToken(
                                                            val,
                                                            cursorPos,
                                                        );
                                                    const cursorOffsetInToken =
                                                        cursorPos - start;
                                                    const sugs =
                                                        getSearchSuggestions(
                                                            token,
                                                            cursorOffsetInToken,
                                                            ledger.accounts,
                                                        );
                                                    setSimilarAcSuggestions(
                                                        sugs,
                                                    );
                                                    setSimilarAcActiveIndex(-1);
                                                }}
                                                onKeyDown={(e) => {
                                                    if (
                                                        similarAcSuggestions.length ===
                                                        0
                                                    )
                                                        return;
                                                    if (e.key === 'ArrowDown') {
                                                        e.preventDefault();
                                                        setSimilarAcActiveIndex(
                                                            (i) =>
                                                                Math.min(
                                                                    i + 1,
                                                                    similarAcSuggestions.length -
                                                                        1,
                                                                ),
                                                        );
                                                    } else if (
                                                        e.key === 'ArrowUp'
                                                    ) {
                                                        e.preventDefault();
                                                        setSimilarAcActiveIndex(
                                                            (i) =>
                                                                Math.max(
                                                                    i - 1,
                                                                    0,
                                                                ),
                                                        );
                                                    } else if (
                                                        (e.key === 'Enter' ||
                                                            e.key === 'Tab') &&
                                                        similarAcActiveIndex >=
                                                            0
                                                    ) {
                                                        e.preventDefault();
                                                        applySimilarSearchCompletion(
                                                            similarAcSuggestions[
                                                                similarAcActiveIndex
                                                            ] ?? '',
                                                        );
                                                    } else if (
                                                        e.key === 'Escape'
                                                    ) {
                                                        setSimilarAcSuggestions(
                                                            [],
                                                        );
                                                        setSimilarAcActiveIndex(
                                                            -1,
                                                        );
                                                    }
                                                }}
                                                onBlur={() => {
                                                    setTimeout(() => {
                                                        setSimilarAcSuggestions(
                                                            [],
                                                        );
                                                        setSimilarAcActiveIndex(
                                                            -1,
                                                        );
                                                    }, 150);
                                                }}
                                            />
                                            {similarAcSuggestions.length >
                                                0 && (
                                                <div
                                                    className="search-autocomplete"
                                                    role="listbox"
                                                >
                                                    {similarAcSuggestions.map(
                                                        (sug, i) => (
                                                            <div
                                                                key={sug}
                                                                className={`ac-item${i === similarAcActiveIndex ? ' active' : ''}`}
                                                                role="option"
                                                                aria-selected={
                                                                    i ===
                                                                    similarAcActiveIndex
                                                                }
                                                                onMouseDown={(
                                                                    e,
                                                                ) => {
                                                                    e.preventDefault();
                                                                    applySimilarSearchCompletion(
                                                                        sug,
                                                                    );
                                                                }}
                                                            >
                                                                {sug}
                                                            </div>
                                                        ),
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                        {activeRecategorizeTab.queryError !==
                                            null && (
                                            <div className="query-error">
                                                {
                                                    activeRecategorizeTab.queryError
                                                }
                                            </div>
                                        )}
                                        <p>
                                            Recategorize{' '}
                                            <strong>
                                                {matchingEntries.length}
                                            </strong>{' '}
                                            transactions matching the current
                                            query to{' '}
                                            <strong>
                                                {activeRecategorizeTab.plan.newAccount.trim() ||
                                                    '(choose category)'}
                                            </strong>
                                            ?
                                        </p>
                                        <AccountInput
                                            value={
                                                activeRecategorizeTab.plan
                                                    .newAccount
                                            }
                                            onChange={(v) => {
                                                setRecategorizeTabs((current) =>
                                                    current.map((tab) =>
                                                        tab.id ===
                                                        activeRecategorizeTab.id
                                                            ? {
                                                                  ...tab,
                                                                  plan: {
                                                                      ...tab.plan,
                                                                      newAccount:
                                                                          v,
                                                                  },
                                                              }
                                                            : tab,
                                                    ),
                                                );
                                            }}
                                            accounts={ledger.accounts.map(
                                                (a) => a.name,
                                            )}
                                            oldAccount={matchingEntries.map(
                                                (entry) => entry.oldAccount,
                                            )}
                                            placeholder="Destination category…"
                                        />
                                        {activeRecategorizeTab.plan.allEntries
                                            .length >
                                            activeRecategorizeTab.plan.entries
                                                .length && (
                                            <label className="checkbox-field">
                                                <input
                                                    type="checkbox"
                                                    checked={
                                                        activeRecategorizeTab
                                                            .plan.includeAll
                                                    }
                                                    onChange={(e) => {
                                                        const checked =
                                                            e.target.checked;
                                                        setRecategorizeTabs(
                                                            (current) =>
                                                                current.map(
                                                                    (tab) =>
                                                                        tab.id ===
                                                                        activeRecategorizeTab.id
                                                                            ? {
                                                                                  ...tab,
                                                                                  plan: {
                                                                                      ...tab.plan,
                                                                                      includeAll:
                                                                                          checked,
                                                                                  },
                                                                              }
                                                                            : tab,
                                                                ),
                                                        );
                                                    }}
                                                />
                                                <span>
                                                    Include{' '}
                                                    {activeRecategorizeTab.plan
                                                        .allEntries.length -
                                                        activeRecategorizeTab
                                                            .plan.entries
                                                            .length}{' '}
                                                    more transactions not in
                                                    current filter
                                                </span>
                                            </label>
                                        )}
                                        <div className="table-wrap similar-confirm-table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Date</th>
                                                        <th>Description</th>
                                                        <th>From Account</th>
                                                        <th>Amount</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {matchingEntries.length ===
                                                    0 ? (
                                                        <tr>
                                                            <td
                                                                colSpan={4}
                                                                className="table-empty"
                                                            >
                                                                No transactions
                                                                match the
                                                                current query.
                                                            </td>
                                                        </tr>
                                                    ) : (
                                                        matchingEntries.map(
                                                            (entry) => {
                                                                const txn =
                                                                    txnById.get(
                                                                        entry.txnId,
                                                                    );
                                                                if (!txn)
                                                                    return null;
                                                                return (
                                                                    <tr
                                                                        key={
                                                                            entry.txnId
                                                                        }
                                                                    >
                                                                        <td className="mono">
                                                                            {
                                                                                txn.date
                                                                            }
                                                                        </td>
                                                                        <td>
                                                                            {
                                                                                txn.description
                                                                            }
                                                                        </td>
                                                                        <td>
                                                                            {
                                                                                entry.oldAccount
                                                                            }
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
                                                    matchingEntries.length === 0
                                                }
                                                onClick={() => {
                                                    void handleBulkRecategorize(
                                                        matchingEntries,
                                                        activeRecategorizeTab.plan.newAccount.trim(),
                                                    );
                                                    setRecategorizeTabs(
                                                        (current) =>
                                                            current.filter(
                                                                (tab) =>
                                                                    tab.id !==
                                                                    activeRecategorizeTab.id,
                                                            ),
                                                    );
                                                    setActiveTab(
                                                        'transactions',
                                                    );
                                                }}
                                            >
                                                Apply
                                            </button>
                                        </div>
                                    </section>
                                );
                            })()}
                        </div>
                    ) : activeTab === 'pipeline' ? (
                        <PipelineTab
                            ledger={ledger}
                            isActive={true}
                            loginAccounts={loginAccounts}
                            loginConfigsByName={loginConfigsByName}
                            hasLoadedLoginConfigs={hasLoadedLoginConfigs}
                            hideObviousAmounts={hideObviousAmounts}
                            onLedgerRefresh={handleLedgerRefresh}
                            onLoginConfigChanged={requestLoginConfigReload}
                            onViewGlTransaction={(id) => {
                                setTransactionsSearch(id);
                                setActiveTab('transactions');
                            }}
                        />
                    ) : activeTab === 'reports' ? (
                        <ReportsTab
                            ledger={ledger.path}
                            accounts={ledger.accounts}
                        />
                    ) : activeTab === 'preferences' ? (
                        <div className="preferences-panel">
                            <section className="preferences-section">
                                <h3>Transactions</h3>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={hideObviousAmounts}
                                        onChange={(e) => {
                                            const v = e.target.checked;
                                            setHideObviousAmounts(v);
                                            localStorage.setItem(
                                                'pref:hideObviousAmounts',
                                                String(v),
                                            );
                                        }}
                                    />
                                    <span>
                                        Collapse obvious posting amounts
                                    </span>
                                </label>
                                <p className="pref-description">
                                    When a transaction has exactly 2 postings
                                    and exactly one is an asset or liability,
                                    hide the amounts from the postings list —
                                    they are derivable from the Amount column.
                                </p>
                            </section>
                        </div>
                    ) : (
                        <ScrapeTab
                            ledger={ledger}
                            loginNames={loginNames}
                            loginConfigsByName={loginConfigsByName}
                            loginAccountMappings={loginAccountMappings}
                            isLoadingLoginConfigs={isLoadingLoginConfigs}
                            conflictingGlAccountSet={conflictingGlAccountSet}
                            selectedLoginName={selectedLoginName}
                            onSelectedLoginNameChange={setSelectedLoginName}
                            loginManagementTab={loginManagementTab}
                            onLoginManagementTabChange={setLoginManagementTab}
                            loginLabelDraft={loginLabelDraft}
                            onLoginLabelDraftChange={setLoginLabelDraft}
                            loginGlAccountDraft={loginGlAccountDraft}
                            onLoginGlAccountDraftChange={setLoginGlAccountDraft}
                            loginConfigStatus={loginConfigStatus}
                            onLoginConfigStatusChange={setLoginConfigStatus}
                            isSavingLoginConfig={isSavingLoginConfig}
                            onIsSavingLoginConfigChange={setIsSavingLoginConfig}
                            onLoginConfigChanged={requestLoginConfigReload}
                            onLedgerRefresh={handleLedgerRefresh}
                            onSecretPrompt={promptSecretDecision}
                            onIgnoreLoginAccountMapping={
                                handleIgnoreLoginAccountMapping
                            }
                        />
                    )}
                </section>
            )}
            {secretPrompt === null ? null : (
                <div className="secret-prompt-overlay">
                    <div
                        className="secret-prompt"
                        role="dialog"
                        aria-modal="true"
                    >
                        <h3>{secretPrompt.title}</h3>
                        <p>{secretPrompt.message}</p>
                        <div className="txn-actions">
                            <button
                                type="button"
                                className="primary-button"
                                onClick={() => {
                                    resolveSecretPrompt(true);
                                }}
                            >
                                {secretPrompt.confirmLabel}
                            </button>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    resolveSecretPrompt(false);
                                }}
                            >
                                {secretPrompt.cancelLabel}
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}

function AccountsTable({
    accounts,
    onSelectAccount,
}: {
    accounts: AccountRow[];
    onSelectAccount: (name: string) => void;
}) {
    return (
        <table className="ledger-table">
            <thead>
                <tr>
                    <th>Account</th>
                    <th>Balance</th>
                    <th>Extraction</th>
                </tr>
            </thead>
            <tbody>
                {accounts.length === 0 ? (
                    <tr>
                        <td colSpan={3} className="table-empty">
                            No accounts found.
                        </td>
                    </tr>
                ) : (
                    accounts.map((account) => (
                        <tr key={account.name}>
                            <td>
                                <button
                                    className="link-button mono"
                                    onClick={() => {
                                        onSelectAccount(account.name);
                                    }}
                                >
                                    {account.name}
                                </button>
                            </td>
                            <td className="amount">
                                {formatTotals(account.totals)}
                            </td>
                            <td>
                                {account.unpostedCount > 0 ? (
                                    <span className="secret-chip warning">
                                        {account.unpostedCount} unposted
                                    </span>
                                ) : (
                                    <span className="status-dim">
                                        up to date
                                    </span>
                                )}
                            </td>
                        </tr>
                    ))
                )}
            </tbody>
        </table>
    );
}

const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp'];

function isImageAttachmentRef(ref: string): boolean {
    if (!ref.endsWith('#attachment')) return false;
    const filename = ref.slice(0, -'#attachment'.length);
    return IMAGE_EXTENSIONS.some((ext) => filename.toLowerCase().endsWith(ext));
}

function attachmentFilename(ref: string): string {
    return ref.endsWith('#attachment')
        ? ref.slice(0, -'#attachment'.length)
        : ref;
}

// A transaction's posting amounts are "obvious" — and therefore redundant to
// display — when there are exactly 2 postings and exactly 1 is a balance-sheet
// account (Assets/Liabilities). In that case both amounts equal the net-worth
// effect shown in the Amount column (one is it, the other is its negation),
// so showing them clutters the Postings column without adding information.
function hasObviousAmounts(txn: TransactionRow): boolean {
    if (txn.postings.length !== 2) return false;
    const balanceSheetCount = txn.postings.filter(
        (p) =>
            p.account.startsWith('Assets:') ||
            p.account.startsWith('Liabilities:'),
    ).length;
    return balanceSheetCount === 1;
}

function singleNonBalancingPosting(txn: TransactionRow): PostingRow | null {
    const candidates = txn.postings.filter(
        (p) =>
            !p.account.startsWith('Assets:') &&
            !p.account.startsWith('Liabilities:'),
    );
    return candidates.length === 1 ? (candidates[0] ?? null) : null;
}

function similarityGroupKey(txn: TransactionRow): string | null {
    if (!singleNonBalancingPosting(txn)) return null;
    const balancing = txn.postings.find(
        (p) =>
            p.account.startsWith('Assets:') ||
            p.account.startsWith('Liabilities:'),
    );
    if (!balancing) return null;
    return `${txn.description}\0${balancing.account}`;
}

/** Key for grouping "similar" uncategorized transactions: same description + same balancing account. */
function similarKey(txn: TransactionRow): string | null {
    const posting = singleNonBalancingPosting(txn);
    if (!posting || posting.account !== 'Expenses:Unknown') return null;
    return similarityGroupKey(txn);
}

function TransactionsTable({
    transactions,
    allTransactions,
    ledgerPath,
    accountNames = [],
    glCategorySuggestions = {},
    onRecategorize,
    onMergeTransfer,
    onOpenLinkTransfer,
    onBulkRecategorize,
    onOpenSimilarRecategorize,
    hideObviousAmounts = true,
    onAddSearchTerm,
}: {
    transactions: TransactionRow[];
    allTransactions: TransactionRow[];
    ledgerPath: string | null;
    accountNames?: string[];
    glCategorySuggestions?: Record<string, GlCategoryResult>;
    onRecategorize?: (
        txnId: string,
        oldAccount: string,
        newAccount: string,
    ) => void;
    onMergeTransfer?: (txnId1: string, txnId2: string) => void;
    onOpenLinkTransfer?: (txnId: string) => void;
    onBulkRecategorize?: (
        entries: Array<{ txnId: string; oldAccount: string }>,
        newAccount: string,
    ) => void;
    onOpenSimilarRecategorize?: (plan: SimilarRecategorizePlan) => void;
    hideObviousAmounts?: boolean;
    onAddSearchTerm?: (term: string) => void;
}) {
    const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
    const [lightboxFilename, setLightboxFilename] = useState<string | null>(
        null,
    );
    const [lightboxLoading, setLightboxLoading] = useState(false);
    const [lightboxError, setLightboxError] = useState<string | null>(null);
    const [editingKey, setEditingKey] = useState<string | null>(null); // `${txnId}:${oldAccount}`
    const [categoryDraft, setCategoryDraft] = useState('');
    const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(
        new Set(),
    );
    const [bulkDraft, setBulkDraft] = useState('');
    const [bulkConfirm, setBulkConfirm] = useState<{
        entries: Array<{ txnId: string; oldAccount: string }>;
        newAccount: string;
    } | null>(null);

    const similarGroupIds = useMemo(() => {
        const map = new Map<string, string[]>();
        for (const txn of transactions) {
            const key = similarKey(txn);
            if (key == null) continue;
            let arr = map.get(key);
            if (!arr) {
                arr = [];
                map.set(key, arr);
            }
            arr.push(txn.id);
        }
        return map;
    }, [transactions]);

    const allSimilarGroupIds = useMemo(() => {
        const map = new Map<string, string[]>();
        for (const txn of allTransactions) {
            const key = similarKey(txn);
            if (key == null) continue;
            let arr = map.get(key);
            if (!arr) {
                arr = [];
                map.set(key, arr);
            }
            arr.push(txn.id);
        }
        return map;
    }, [allTransactions]);

    type ContextMenuItem = { label: string; action: () => void };
    const [contextMenu, setContextMenu] = useState<{
        x: number;
        y: number;
        items: ContextMenuItem[];
    } | null>(null);

    function openContextMenu(e: React.MouseEvent, items: ContextMenuItem[]) {
        e.preventDefault();
        e.stopPropagation();
        setContextMenu({ x: e.clientX, y: e.clientY, items });
    }

    useEffect(() => {
        if (!contextMenu) return;
        const close = () => {
            setContextMenu(null);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') setContextMenu(null);
        };
        document.addEventListener('mousedown', close);
        document.addEventListener('keydown', onKey);
        return () => {
            document.removeEventListener('mousedown', close);
            document.removeEventListener('keydown', onKey);
        };
    }, [contextMenu]);

    async function handleAttachmentClick(filename: string) {
        if (ledgerPath === null) return;
        setLightboxFilename(filename);
        setLightboxSrc(null);
        setLightboxError(null);
        setLightboxLoading(true);
        try {
            const src = await readAttachmentDataUrl(ledgerPath, filename);
            setLightboxSrc(src);
        } catch (e) {
            setLightboxError(String(e));
        } finally {
            setLightboxLoading(false);
        }
    }

    function closeLightbox() {
        setLightboxSrc(null);
        setLightboxFilename(null);
        setLightboxError(null);
        setLightboxLoading(false);
    }

    const hasActions =
        onRecategorize !== undefined ||
        onMergeTransfer !== undefined ||
        onOpenLinkTransfer !== undefined;
    const hasCheckbox = onBulkRecategorize !== undefined;

    useEffect(() => {
        if (selectedIds.size === 0 || bulkDraft !== '') return;
        const tally = new Map<string, number>();
        for (const id of selectedIds) {
            const s = glCategorySuggestions[id]?.suggested;
            if (s != null) tally.set(s, (tally.get(s) ?? 0) + 1);
        }
        if (tally.size === 0) return;
        const best = [...tally.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
        if (best !== undefined) setBulkDraft(best);
    }, [selectedIds]); // eslint-disable-line react-hooks/exhaustive-deps

    function applyBulk(
        entries: Array<{ txnId: string; oldAccount: string }>,
        newAccount: string,
    ) {
        if (onBulkRecategorize === undefined) return;
        const accounts = new Set(entries.map((e) => e.oldAccount));
        if (accounts.size <= 1) {
            onBulkRecategorize(entries, newAccount);
            setSelectedIds(new Set());
            setBulkDraft('');
        } else {
            setBulkConfirm({ entries, newAccount });
        }
    }

    const eligibleIds = transactions
        .filter((t) => singleNonBalancingPosting(t) !== null)
        .map((t) => t.id);
    const allSelected =
        eligibleIds.length > 0 &&
        eligibleIds.every((id) => selectedIds.has(id));
    const someSelected = eligibleIds.some((id) => selectedIds.has(id));
    const bulkEntries = [...selectedIds].flatMap((id) => {
        const txn = transactions.find((t) => t.id === id);
        if (!txn) return [];
        const p = singleNonBalancingPosting(txn);
        return p ? [{ txnId: id, oldAccount: p.account }] : [];
    });
    const colCount = 5 + (hasCheckbox ? 1 : 0);

    function toRecategorizeEntries(
        ids: string[],
        sourceTransactions: TransactionRow[],
    ): Array<{ txnId: string; oldAccount: string }> {
        return ids.flatMap((id) => {
            const txn = sourceTransactions.find((t) => t.id === id);
            if (!txn) return [];
            const posting = singleNonBalancingPosting(txn);
            if (!posting || posting.account !== 'Expenses:Unknown') return [];
            return [{ txnId: id, oldAccount: posting.account }];
        });
    }
    function openSimilarConfirmForTxn(
        txn: TransactionRow,
        targetAccount: string,
    ) {
        const key = similarityGroupKey(txn);
        if (key === null || onOpenSimilarRecategorize === undefined) return;
        const filteredSimilarIds = similarGroupIds.get(key) ?? [];
        const allSimilarIds = allSimilarGroupIds.get(key) ?? [];
        const entries = toRecategorizeEntries(filteredSimilarIds, transactions);
        if (entries.length <= 1) return;
        const allEntries = toRecategorizeEntries(
            allSimilarIds,
            allTransactions,
        );
        const balancingAccount =
            txn.postings.find(
                (posting) =>
                    posting.account.startsWith('Assets:') ||
                    posting.account.startsWith('Liabilities:'),
            )?.account ?? '';
        onOpenSimilarRecategorize({
            entries,
            allEntries:
                allEntries.length >= entries.length ? allEntries : entries,
            newAccount: targetAccount,
            searchQuery: `desc:${quoteHledgerValue(txn.description)} acct:${quoteHledgerValue(balancingAccount)} acct:Expenses:Unknown`,
            description: txn.description,
            balancingAccount,
            includeAll: false,
        });
    }

    return (
        <>
            {hasCheckbox && selectedIds.size > 0 && (
                <div className="bulk-action-bar">
                    <span className="count-label">
                        {selectedIds.size} selected
                        {bulkEntries.length < selectedIds.size &&
                            ` (${bulkEntries.length} eligible)`}
                    </span>
                    <AccountInput
                        value={bulkDraft}
                        onChange={(v) => {
                            setBulkDraft(v);
                        }}
                        onKeyDown={(e) => {
                            if (
                                e.key === 'Enter' &&
                                bulkDraft.trim() &&
                                bulkEntries.length > 0
                            ) {
                                applyBulk(bulkEntries, bulkDraft.trim());
                            } else if (e.key === 'Escape') {
                                setSelectedIds(new Set());
                                setBulkDraft('');
                            }
                        }}
                        accounts={accountNames}
                        oldAccount={bulkEntries.map((e) => e.oldAccount)}
                        placeholder="New account…"
                    />
                    <button
                        type="button"
                        className="ghost-button"
                        disabled={!bulkDraft.trim() || bulkEntries.length === 0}
                        onClick={() => {
                            applyBulk(bulkEntries, bulkDraft.trim());
                        }}
                    >
                        Set Category
                    </button>
                    <button
                        type="button"
                        className="ghost-button"
                        onClick={() => {
                            setSelectedIds(new Set());
                            setBulkDraft('');
                        }}
                    >
                        Clear
                    </button>
                </div>
            )}
            <div className="table-wrap">
                <table className="ledger-table">
                    <thead>
                        <tr>
                            {hasCheckbox && (
                                <th>
                                    <input
                                        type="checkbox"
                                        checked={allSelected}
                                        ref={(el) => {
                                            if (el)
                                                el.indeterminate =
                                                    someSelected &&
                                                    !allSelected;
                                        }}
                                        onChange={() => {
                                            setSelectedIds(
                                                allSelected
                                                    ? new Set()
                                                    : new Set(eligibleIds),
                                            );
                                        }}
                                    />
                                </th>
                            )}
                            <th>Date</th>
                            <th>Description</th>
                            <th>Postings</th>
                            <th>Amount</th>
                            <th>Attachments</th>
                        </tr>
                    </thead>
                    <tbody>
                        {transactions.length === 0 ? (
                            <tr>
                                <td colSpan={colCount} className="table-empty">
                                    No transactions found.
                                </td>
                            </tr>
                        ) : (
                            transactions.map((txn) => {
                                const isUncategorized = txn.postings.some(
                                    (p) => p.account === 'Expenses:Unknown',
                                );
                                const glSuggestion =
                                    glCategorySuggestions[txn.id];
                                const transferMatch =
                                    glSuggestion?.transferMatch ?? null;
                                const suggested =
                                    glSuggestion?.suggested ?? null;
                                const eligible =
                                    singleNonBalancingPosting(txn) !== null;
                                return (
                                    <tr
                                        key={txn.id}
                                        className={
                                            isUncategorized
                                                ? 'row-uncategorized'
                                                : undefined
                                        }
                                    >
                                        {hasCheckbox && (
                                            <td>
                                                {eligible && (
                                                    <input
                                                        type="checkbox"
                                                        checked={selectedIds.has(
                                                            txn.id,
                                                        )}
                                                        onChange={() => {
                                                            setSelectedIds(
                                                                (prev) => {
                                                                    const next =
                                                                        new Set(
                                                                            prev,
                                                                        );
                                                                    if (
                                                                        next.has(
                                                                            txn.id,
                                                                        )
                                                                    )
                                                                        next.delete(
                                                                            txn.id,
                                                                        );
                                                                    else
                                                                        next.add(
                                                                            txn.id,
                                                                        );
                                                                    return next;
                                                                },
                                                            );
                                                        }}
                                                    />
                                                )}
                                            </td>
                                        )}
                                        <td
                                            className="mono"
                                            onContextMenu={(e) => {
                                                openContextMenu(e, [
                                                    {
                                                        label: `Filter: date:${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:${txn.date}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: date:>=${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:>=${txn.date}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: date:<=${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:<=${txn.date}`,
                                                            ),
                                                    },
                                                ]);
                                            }}
                                        >
                                            {txn.date}
                                        </td>
                                        <td
                                            onContextMenu={(e) => {
                                                const key = similarKey(txn);
                                                const similarIds =
                                                    key !== null
                                                        ? (similarGroupIds.get(
                                                              key,
                                                          ) ?? [])
                                                        : [];
                                                const balancingAccount =
                                                    txn.postings.find(
                                                        (p) =>
                                                            p.account.startsWith(
                                                                'Assets:',
                                                            ) ||
                                                            p.account.startsWith(
                                                                'Liabilities:',
                                                            ),
                                                    )?.account ?? '';
                                                const items: ContextMenuItem[] =
                                                    [
                                                        {
                                                            label: `Filter: desc:${quoteHledgerValue(txn.description)}`,
                                                            action: () =>
                                                                onAddSearchTerm?.(
                                                                    `desc:${quoteHledgerValue(txn.description)}`,
                                                                ),
                                                        },
                                                    ];
                                                if (
                                                    hasCheckbox &&
                                                    similarIds.length > 1
                                                ) {
                                                    items.push({
                                                        label: `Check ${similarIds.length} uncategorized ${txn.description} transactions from ${balancingAccount}`,
                                                        action: () => {
                                                            setSelectedIds(
                                                                new Set(
                                                                    similarIds,
                                                                ),
                                                            );
                                                        },
                                                    });
                                                }
                                                openContextMenu(e, [...items]);
                                            }}
                                        >
                                            {txn.description}
                                        </td>
                                        <td>
                                            {hasActions ? (
                                                <div className="postings-list">
                                                    {txn.postings.map((p) => {
                                                        const key = `${txn.id}:${p.account}`;
                                                        const isEditing =
                                                            editingKey === key;
                                                        const isUnknown =
                                                            p.account ===
                                                            'Expenses:Unknown';
                                                        const isNonBalanceSheet =
                                                            !p.account.startsWith(
                                                                'Assets:',
                                                            ) &&
                                                            !p.account.startsWith(
                                                                'Liabilities:',
                                                            );
                                                        const postingMenuItems: ContextMenuItem[] =
                                                            [
                                                                {
                                                                    label: `Filter: acct:${quoteHledgerValue(p.account)}`,
                                                                    action: () =>
                                                                        onAddSearchTerm?.(
                                                                            `acct:${quoteHledgerValue(p.account)}`,
                                                                        ),
                                                                },
                                                            ];
                                                        if (isNonBalanceSheet) {
                                                            postingMenuItems.push(
                                                                {
                                                                    label: 'Set Category',
                                                                    action: () => {
                                                                        setCategoryDraft(
                                                                            isUnknown &&
                                                                                suggested !==
                                                                                    null
                                                                                ? suggested
                                                                                : '',
                                                                        );
                                                                        setEditingKey(
                                                                            key,
                                                                        );
                                                                    },
                                                                },
                                                            );
                                                            const keyForSimilarMenu =
                                                                similarityGroupKey(
                                                                    txn,
                                                                );
                                                            const similarIdsForMenu =
                                                                keyForSimilarMenu !==
                                                                null
                                                                    ? (similarGroupIds.get(
                                                                          keyForSimilarMenu,
                                                                      ) ?? [])
                                                                    : [];
                                                            if (
                                                                p.account !==
                                                                    'Expenses:Unknown' &&
                                                                onOpenSimilarRecategorize !==
                                                                    undefined &&
                                                                similarIdsForMenu.length >
                                                                    1
                                                            ) {
                                                                postingMenuItems.push(
                                                                    {
                                                                        label: `Categorize ${similarIdsForMenu.length} similar transactions to ${p.account}`,
                                                                        action: () => {
                                                                            openSimilarConfirmForTxn(
                                                                                txn,
                                                                                p.account,
                                                                            );
                                                                        },
                                                                    },
                                                                );
                                                            }
                                                        }
                                                        if (
                                                            isUncategorized &&
                                                            transferMatch !==
                                                                null &&
                                                            onOpenLinkTransfer !==
                                                                undefined
                                                        ) {
                                                            postingMenuItems.push(
                                                                {
                                                                    label: 'Link Transfer',
                                                                    action: () => {
                                                                        onOpenLinkTransfer(
                                                                            txn.id,
                                                                        );
                                                                    },
                                                                },
                                                            );
                                                        }
                                                        const hideAmounts =
                                                            hideObviousAmounts &&
                                                            hasObviousAmounts(
                                                                txn,
                                                            );
                                                        const keyForSimilar =
                                                            similarKey(txn);
                                                        const filteredSimilarIds =
                                                            keyForSimilar !==
                                                            null
                                                                ? (similarGroupIds.get(
                                                                      keyForSimilar,
                                                                  ) ?? [])
                                                                : [];
                                                        const canShowSimilarPill =
                                                            onOpenSimilarRecategorize !==
                                                                undefined &&
                                                            filteredSimilarIds.length >
                                                                1;
                                                        return (
                                                            <div
                                                                key={p.account}
                                                                className="postings-item"
                                                            >
                                                                {isEditing ? (
                                                                    <>
                                                                        <AccountInput
                                                                            value={
                                                                                categoryDraft
                                                                            }
                                                                            onChange={(
                                                                                v,
                                                                            ) => {
                                                                                setCategoryDraft(
                                                                                    v,
                                                                                );
                                                                            }}
                                                                            onKeyDown={(
                                                                                e,
                                                                            ) => {
                                                                                if (
                                                                                    e.key ===
                                                                                        'Enter' &&
                                                                                    categoryDraft.trim()
                                                                                ) {
                                                                                    onRecategorize?.(
                                                                                        txn.id,
                                                                                        p.account,
                                                                                        categoryDraft.trim(),
                                                                                    );
                                                                                    setEditingKey(
                                                                                        null,
                                                                                    );
                                                                                } else if (
                                                                                    e.key ===
                                                                                    'Escape'
                                                                                ) {
                                                                                    setEditingKey(
                                                                                        null,
                                                                                    );
                                                                                }
                                                                            }}
                                                                            accounts={
                                                                                accountNames
                                                                            }
                                                                            oldAccount={
                                                                                p.account
                                                                            }
                                                                            autoFocus
                                                                        />
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            disabled={
                                                                                !categoryDraft.trim()
                                                                            }
                                                                            onClick={() => {
                                                                                if (
                                                                                    categoryDraft.trim()
                                                                                ) {
                                                                                    onRecategorize?.(
                                                                                        txn.id,
                                                                                        p.account,
                                                                                        categoryDraft.trim(),
                                                                                    );
                                                                                    setEditingKey(
                                                                                        null,
                                                                                    );
                                                                                }
                                                                            }}
                                                                        >
                                                                            Set
                                                                        </button>
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            onClick={() => {
                                                                                setEditingKey(
                                                                                    null,
                                                                                );
                                                                            }}
                                                                        >
                                                                            Cancel
                                                                        </button>
                                                                        {canShowSimilarPill && (
                                                                            <button
                                                                                type="button"
                                                                                className="similar-count-pill"
                                                                                disabled={
                                                                                    !categoryDraft.trim()
                                                                                }
                                                                                onClick={() => {
                                                                                    if (
                                                                                        categoryDraft.trim()
                                                                                    ) {
                                                                                        openSimilarConfirmForTxn(
                                                                                            txn,
                                                                                            categoryDraft.trim(),
                                                                                        );
                                                                                    }
                                                                                }}
                                                                            >
                                                                                ×
                                                                                {
                                                                                    filteredSimilarIds.length
                                                                                }{' '}
                                                                                similar
                                                                            </button>
                                                                        )}
                                                                    </>
                                                                ) : isUnknown ? (
                                                                    <button
                                                                        type="button"
                                                                        className="posting-account posting-account-unknown"
                                                                        title="Click to set category"
                                                                        onClick={() => {
                                                                            setCategoryDraft(
                                                                                suggested !==
                                                                                    null
                                                                                    ? suggested
                                                                                    : '',
                                                                            );
                                                                            setEditingKey(
                                                                                key,
                                                                            );
                                                                        }}
                                                                        onContextMenu={(
                                                                            e,
                                                                        ) => {
                                                                            openContextMenu(
                                                                                e,
                                                                                postingMenuItems,
                                                                            );
                                                                        }}
                                                                    >
                                                                        {
                                                                            p.account
                                                                        }
                                                                    </button>
                                                                ) : (
                                                                    <span
                                                                        onContextMenu={(
                                                                            e,
                                                                        ) => {
                                                                            openContextMenu(
                                                                                e,
                                                                                postingMenuItems,
                                                                            );
                                                                        }}
                                                                    >
                                                                        {
                                                                            p.account
                                                                        }
                                                                    </span>
                                                                )}
                                                                {isUnknown &&
                                                                    !isEditing &&
                                                                    transferMatch !==
                                                                        null &&
                                                                    onMergeTransfer !==
                                                                        undefined && (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            onClick={() => {
                                                                                onMergeTransfer(
                                                                                    txn.id,
                                                                                    transferMatch.txnId,
                                                                                );
                                                                            }}
                                                                        >
                                                                            ↔{' '}
                                                                            {
                                                                                transferMatch.date
                                                                            }{' '}
                                                                            {
                                                                                transferMatch.description
                                                                            }
                                                                        </button>
                                                                    )}
                                                                {isUnknown &&
                                                                    !isEditing &&
                                                                    suggested !==
                                                                        null &&
                                                                    transferMatch ===
                                                                        null &&
                                                                    onRecategorize !==
                                                                        undefined && (
                                                                        <div className="categorize-chip">
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    onRecategorize(
                                                                                        txn.id,
                                                                                        'Expenses:Unknown',
                                                                                        suggested,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                {
                                                                                    suggested
                                                                                }
                                                                            </button>
                                                                            {canShowSimilarPill && (
                                                                                <button
                                                                                    type="button"
                                                                                    className="similar-count-pill"
                                                                                    onClick={() => {
                                                                                        openSimilarConfirmForTxn(
                                                                                            txn,
                                                                                            suggested,
                                                                                        );
                                                                                    }}
                                                                                >
                                                                                    ×
                                                                                    {
                                                                                        filteredSimilarIds.length
                                                                                    }{' '}
                                                                                    similar
                                                                                </button>
                                                                            )}
                                                                        </div>
                                                                    )}
                                                                {!hideAmounts && (
                                                                    <span className="amount">
                                                                        {formatTotals(
                                                                            p.totals,
                                                                        )}
                                                                    </span>
                                                                )}
                                                            </div>
                                                        );
                                                    })}
                                                </div>
                                            ) : (
                                                <PostingsList
                                                    postings={txn.postings}
                                                    hideAmounts={
                                                        hideObviousAmounts &&
                                                        hasObviousAmounts(txn)
                                                    }
                                                />
                                            )}
                                        </td>
                                        <td
                                            className="amount"
                                            onContextMenu={(e) => {
                                                const totals = txn.totals;
                                                if (
                                                    totals == null ||
                                                    totals.length === 0
                                                )
                                                    return;
                                                const t = totals[0];
                                                if (t == null) return;
                                                const total = formatScaled(
                                                    t.mantissa,
                                                    t.scale,
                                                );
                                                openContextMenu(e, [
                                                    {
                                                        label: `Filter: amt:${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:${total}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: amt:>=${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:>=${total}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: amt:<=${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:<=${total}`,
                                                            ),
                                                    },
                                                ]);
                                            }}
                                        >
                                            {formatTotals(txn.totals)}
                                        </td>
                                        <td>
                                            {txn.evidence.length === 0 ? (
                                                <span className="text-muted">
                                                    -
                                                </span>
                                            ) : (
                                                <div className="evidence-list">
                                                    {txn.evidence.map(
                                                        (evidenceRef) =>
                                                            isImageAttachmentRef(
                                                                evidenceRef,
                                                            ) ? (
                                                                <button
                                                                    key={`${txn.id}-${evidenceRef}`}
                                                                    className="evidence-chip evidence-chip-image"
                                                                    type="button"
                                                                    onClick={() => {
                                                                        void handleAttachmentClick(
                                                                            attachmentFilename(
                                                                                evidenceRef,
                                                                            ),
                                                                        );
                                                                    }}
                                                                >
                                                                    {attachmentFilename(
                                                                        evidenceRef,
                                                                    )}
                                                                </button>
                                                            ) : (
                                                                <span
                                                                    key={`${txn.id}-${evidenceRef}`}
                                                                    className="evidence-chip"
                                                                    title={
                                                                        evidenceRef
                                                                    }
                                                                >
                                                                    {
                                                                        evidenceRef
                                                                    }
                                                                </span>
                                                            ),
                                                    )}
                                                </div>
                                            )}
                                        </td>
                                    </tr>
                                );
                            })
                        )}
                    </tbody>
                </table>
            </div>
            {(lightboxLoading ||
                lightboxSrc !== null ||
                lightboxError !== null) && (
                <div
                    className="modal-overlay"
                    onClick={closeLightbox}
                    role="dialog"
                    aria-modal="true"
                    aria-label={lightboxFilename ?? 'Attachment'}
                >
                    <div
                        className="modal-dialog attachment-lightbox"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>{lightboxFilename}</h3>
                            <button
                                type="button"
                                onClick={closeLightbox}
                                className="ghost-button"
                            >
                                Close
                            </button>
                        </div>
                        {lightboxLoading ? (
                            <p className="status">Loading…</p>
                        ) : lightboxError !== null ? (
                            <p className="status">{lightboxError}</p>
                        ) : lightboxSrc !== null ? (
                            <img
                                src={lightboxSrc}
                                alt={lightboxFilename ?? 'attachment'}
                                className="attachment-image"
                            />
                        ) : null}
                    </div>
                </div>
            )}
            {bulkConfirm !== null && (
                <div
                    className="modal-overlay"
                    onClick={() => {
                        setBulkConfirm(null);
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
                                    setBulkConfirm(null);
                                }}
                            >
                                Close
                            </button>
                        </div>
                        <p>
                            The selected rows have different current accounts.
                            All will be changed to{' '}
                            <strong>{bulkConfirm.newAccount}</strong>:
                        </p>
                        <ul>
                            {[
                                ...new Map(
                                    bulkConfirm.entries.map((e) => [
                                        e.oldAccount,
                                        0,
                                    ]),
                                ).entries(),
                            ].map(([acct]) => {
                                const count = bulkConfirm.entries.filter(
                                    (e) => e.oldAccount === acct,
                                ).length;
                                return (
                                    <li key={acct}>
                                        {count} × {acct} →{' '}
                                        {bulkConfirm.newAccount}
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
                                    setBulkConfirm(null);
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    onBulkRecategorize?.(
                                        bulkConfirm.entries,
                                        bulkConfirm.newAccount,
                                    );
                                    setSelectedIds(new Set());
                                    setBulkDraft('');
                                    setBulkConfirm(null);
                                }}
                            >
                                Confirm
                            </button>
                        </div>
                    </div>
                </div>
            )}
            {contextMenu && (
                <div
                    className="context-menu"
                    style={{ left: contextMenu.x, top: contextMenu.y }}
                    onMouseDown={(e) => {
                        e.stopPropagation();
                    }}
                >
                    {contextMenu.items.map((item, i) => (
                        <button
                            key={i}
                            type="button"
                            className="context-menu-item"
                            onClick={() => {
                                item.action();
                                setContextMenu(null);
                            }}
                        >
                            {item.label}
                        </button>
                    ))}
                </div>
            )}
        </>
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

function AccountInput({
    value,
    onChange,
    onKeyDown,
    accounts,
    oldAccount,
    autoFocus,
    placeholder = 'Account name…',
}: {
    value: string;
    onChange: (value: string) => void;
    onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
    accounts: string[];
    /** Old account(s) being replaced — used to filter suggestions and show type-change warning. */
    oldAccount?: string | string[];
    autoFocus?: boolean;
    placeholder?: string;
}) {
    const [suggestions, setSuggestions] = useState<string[]>([]);
    const [activeIndex, setActiveIndex] = useState(-1);

    const warning =
        value.trim() !== ''
            ? checkAccountTypeChange(oldAccount ?? [], value.trim())
            : null;

    function computeSuggestions(draft: string) {
        setSuggestions(getAccountSuggestions(draft, accounts, oldAccount));
        setActiveIndex(-1);
    }

    function applyCompletion(sug: string) {
        onChange(sug);
        // Re-compute for the chosen value (e.g. "Expenses:" → show sub-accounts)
        const next = getAccountSuggestions(sug, accounts, oldAccount);
        setSuggestions(next);
        setActiveIndex(-1);
    }

    // ArrowDown/Up navigate, Enter/Tab (with active item) select, Escape dismisses.
    // Unhandled keys pass through to onKeyDown so the parent's Enter (commit)
    // and Escape (cancel editing) still fire when no suggestion is active.
    function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
        if (suggestions.length === 0) {
            onKeyDown?.(e);
            return;
        }
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setActiveIndex((i) => Math.min(i + 1, suggestions.length - 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setActiveIndex((i) => Math.max(i - 1, 0));
        } else if ((e.key === 'Enter' || e.key === 'Tab') && activeIndex >= 0) {
            e.preventDefault();
            const sug = suggestions[activeIndex];
            if (sug !== undefined) applyCompletion(sug);
        } else if (e.key === 'Escape') {
            // First Escape dismisses suggestions; second Escape reaches parent.
            setSuggestions([]);
            setActiveIndex(-1);
        } else {
            onKeyDown?.(e);
        }
    }

    return (
        <div className="account-input-wrap">
            <input
                type="text"
                value={value}
                placeholder={placeholder}
                autoFocus={autoFocus}
                onFocus={(e) => {
                    e.target.select();
                    computeSuggestions(e.target.value);
                }}
                onChange={(e) => {
                    onChange(e.target.value);
                    computeSuggestions(e.target.value);
                }}
                onKeyDown={handleKeyDown}
                onBlur={() => {
                    // Allow mousedown on a suggestion item to fire before blur
                    // closes the list.
                    setTimeout(() => {
                        setSuggestions([]);
                        setActiveIndex(-1);
                    }, 150);
                }}
            />
            {suggestions.length > 0 && (
                <div className="account-suggestions" role="listbox">
                    {suggestions.map((sug, i) => (
                        <div
                            key={sug}
                            className={`ac-item${i === activeIndex ? ' active' : ''}`}
                            role="option"
                            aria-selected={i === activeIndex}
                            onMouseDown={(e) => {
                                e.preventDefault(); // keep input focus
                                applyCompletion(sug);
                            }}
                        >
                            {sug}
                        </div>
                    ))}
                </div>
            )}
            {warning != null && warning !== '' && (
                <div className="account-warning">{warning}</div>
            )}
        </div>
    );
}

function PostingsList({
    postings,
    hideAmounts = false,
}: {
    postings: PostingRow[];
    hideAmounts?: boolean;
}) {
    return (
        <div className="postings-list">
            {postings.map((posting) => (
                <div key={posting.account} className="postings-item">
                    <span>{posting.account}</span>
                    {!hideAmounts && (
                        <span className="amount">
                            {formatTotals(posting.totals)}
                        </span>
                    )}
                </div>
            ))}
        </div>
    );
}

export default App;
