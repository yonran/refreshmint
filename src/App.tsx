import { useCallback, useEffect, useRef, useState } from 'react';
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
    addAccountSecret,
    type AccountJournalEntry,
    type AmountStyleHint,
    type AmountTotal,
    addTransaction,
    addTransactionText,
    getAccountConfig,
    getAccountJournal,
    getScrapeDebugSessionSocket,
    getUnreconciled,
    listDocuments,
    listAccountSecrets,
    listScrapeExtensions,
    loadScrapeExtension,
    openLedger,
    type AccountRow,
    type DocumentWithInfo,
    type LedgerView,
    type NewTransactionInput,
    type PostingRow,
    reconcileEntry,
    reconcileTransfer,
    runExtraction,
    setAccountExtension,
    startScrapeDebugSession,
    stopScrapeDebugSession,
    runScrape,
    reenterAccountSecret,
    removeAccountSecret,
    type SecretEntry,
    type TransactionRow,
    unreconcileEntry,
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

type ReconcileDraft = {
    counterpartAccount: string;
    postingIndex: string;
};

type TransferDraft = {
    account1: string;
    entryId1: string;
    account2: string;
    entryId2: string;
};

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
    const [activeTab, setActiveTab] = useState<ActiveTab>('accounts');
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
    const [scrapeAccount, setScrapeAccount] = useState('');
    const [scrapeExtension, setScrapeExtension] = useState('');
    const [scrapeExtensions, setScrapeExtensions] = useState<string[]>([]);
    const [scrapeStatus, setScrapeStatus] = useState<string | null>(null);
    const [scrapeDebugSocket, setScrapeDebugSocket] = useState<string | null>(
        null,
    );
    const [accountSecrets, setAccountSecrets] = useState<SecretEntry[]>([]);
    const [secretDomain, setSecretDomain] = useState('');
    const [secretName, setSecretName] = useState('');
    const [secretValue, setSecretValue] = useState('');
    const [secretsStatus, setSecretsStatus] = useState<string | null>(null);
    const [isRunningScrape, setIsRunningScrape] = useState(false);
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isImportingScrapeExtension, setIsImportingScrapeExtension] =
        useState(false);
    const [isStartingScrapeDebug, setIsStartingScrapeDebug] = useState(false);
    const [isStoppingScrapeDebug, setIsStoppingScrapeDebug] = useState(false);
    const [isLoadingAccountSecrets, setIsLoadingAccountSecrets] =
        useState(false);
    const [isSavingAccountSecret, setIsSavingAccountSecret] = useState(false);
    const [busySecretKey, setBusySecretKey] = useState<string | null>(null);
    const [documents, setDocuments] = useState<DocumentWithInfo[]>([]);
    const [selectedDocumentNames, setSelectedDocumentNames] = useState<
        string[]
    >([]);
    const [accountJournalEntries, setAccountJournalEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [unreconciledEntries, setUnreconciledEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [pipelineStatus, setPipelineStatus] = useState<string | null>(null);
    const [isLoadingDocuments, setIsLoadingDocuments] = useState(false);
    const [isRunningExtraction, setIsRunningExtraction] = useState(false);
    const [isLoadingAccountJournal, setIsLoadingAccountJournal] =
        useState(false);
    const [isLoadingUnreconciled, setIsLoadingUnreconciled] = useState(false);
    const [reconcileDrafts, setReconcileDrafts] = useState<
        Record<string, ReconcileDraft>
    >({});
    const [busyReconcileEntryId, setBusyReconcileEntryId] = useState<
        string | null
    >(null);
    const [unreconcileEntryId, setUnreconcileEntryId] = useState('');
    const [unreconcilePostingIndex, setUnreconcilePostingIndex] = useState('');
    const [isUnreconcilingEntry, setIsUnreconcilingEntry] = useState(false);
    const [transferDraft, setTransferDraft] = useState<TransferDraft>({
        account1: '',
        entryId1: '',
        account2: '',
        entryId2: '',
    });
    const [isReconcilingTransfer, setIsReconcilingTransfer] = useState(false);
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
    const ledgerPath = ledger?.path ?? null;
    const scrapeAccountOptions = ledger
        ? ledger.accounts
              .map((account) => account.name.trim())
              .filter(
                  (name, index, names) =>
                      name.length > 0 && names.indexOf(name) === index,
              )
        : [];

    useEffect(() => {
        if (ledger) {
            setTransactionDraft(createTransactionDraft());
            setRawDraft('');
            setAddStatus(null);
            setDraftStatus(null);
            setScrapeStatus(null);
            setScrapeDebugSocket(null);
            setScrapeAccount('');
            setAccountSecrets([]);
            setSecretDomain('');
            setSecretName('');
            setSecretValue('');
            setSecretsStatus(null);
            setIsLoadingAccountSecrets(false);
            setIsSavingAccountSecret(false);
            setBusySecretKey(null);
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnreconciledEntries([]);
            setPipelineStatus(null);
            setIsLoadingDocuments(false);
            setIsRunningExtraction(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnreconciled(false);
            setReconcileDrafts({});
            setBusyReconcileEntryId(null);
            setUnreconcileEntryId('');
            setUnreconcilePostingIndex('');
            setIsUnreconcilingEntry(false);
            setTransferDraft({
                account1: '',
                entryId1: '',
                account2: '',
                entryId2: '',
            });
            setIsReconcilingTransfer(false);
        }
    }, [ledger]);

    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtensions([]);
            setScrapeExtension('');
            setScrapeDebugSocket(null);
            setIsLoadingScrapeExtensions(false);
            return;
        }

        let cancelled = false;
        setIsLoadingScrapeExtensions(true);
        setScrapeStatus(null);
        void listScrapeExtensions(ledgerPath)
            .then((extensions) => {
                if (cancelled) {
                    return;
                }
                setScrapeExtensions(extensions);
                setScrapeExtension((current) => {
                    if (current.length > 0 && extensions.includes(current)) {
                        return current;
                    }
                    return extensions[0] ?? '';
                });
            })
            .catch((error: unknown) => {
                if (!cancelled) {
                    setScrapeExtensions([]);
                    setScrapeExtension('');
                    setScrapeStatus(
                        `Failed to load scrape extensions: ${String(error)}`,
                    );
                }
            })
            .finally(() => {
                if (!cancelled) {
                    setIsLoadingScrapeExtensions(false);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeDebugSocket(null);
            return;
        }

        let cancelled = false;
        void getScrapeDebugSessionSocket()
            .then((socket) => {
                if (!cancelled) {
                    setScrapeDebugSocket(socket);
                }
            })
            .catch(() => {
                if (!cancelled) {
                    setScrapeDebugSocket(null);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    useEffect(() => {
        if (ledgerPath === null) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            return;
        }

        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            return;
        }

        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingAccountSecrets(true);
            void listAccountSecrets(account)
                .then((entries) => {
                    if (!cancelled) {
                        setAccountSecrets(entries);
                    }
                })
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setAccountSecrets([]);
                        setSecretsStatus(
                            `Failed to load account secrets: ${String(error)}`,
                        );
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsLoadingAccountSecrets(false);
                    }
                });
        }, 250);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledgerPath, scrapeAccount]);

    // Load account config and auto-populate extension when account changes
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtension('');
            return;
        }

        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setScrapeExtension('');
            return;
        }

        // Prevent stale extension state from bleeding across accounts.
        setScrapeExtension('');

        let cancelled = false;
        const timer = window.setTimeout(() => {
            void getAccountConfig(ledgerPath, account)
                .then((config) => {
                    if (cancelled) {
                        return;
                    }
                    setScrapeExtension(config.extension?.trim() ?? '');
                })
                .catch(() => {
                    if (!cancelled) {
                        setScrapeExtension('');
                    }
                });
        }, 100);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledgerPath, scrapeAccount]);

    useEffect(() => {
        if (ledgerPath === null) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnreconciledEntries([]);
            setReconcileDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnreconciled(false);
            return;
        }

        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnreconciledEntries([]);
            setReconcileDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnreconciled(false);
            return;
        }

        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingDocuments(true);
            setIsLoadingAccountJournal(true);
            setIsLoadingUnreconciled(true);
            void Promise.all([
                listDocuments(ledgerPath, account),
                getAccountJournal(ledgerPath, account),
                getUnreconciled(ledgerPath, account),
            ])
                .then(
                    ([
                        fetchedDocuments,
                        fetchedJournal,
                        fetchedUnreconciled,
                    ]) => {
                        if (cancelled) {
                            return;
                        }
                        setDocuments(fetchedDocuments);
                        setSelectedDocumentNames((current) =>
                            current.filter((name) =>
                                fetchedDocuments.some(
                                    (doc) => doc.filename === name,
                                ),
                            ),
                        );
                        setAccountJournalEntries(fetchedJournal);
                        setUnreconciledEntries(fetchedUnreconciled);
                        setReconcileDrafts((current) => {
                            const next: Record<string, ReconcileDraft> = {};
                            for (const entry of fetchedUnreconciled) {
                                next[entry.id] = current[entry.id] ?? {
                                    counterpartAccount: '',
                                    postingIndex: '',
                                };
                            }
                            return next;
                        });
                        setTransferDraft((current) => ({
                            ...current,
                            account1:
                                current.account1.trim().length > 0
                                    ? current.account1
                                    : account,
                        }));
                    },
                )
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setDocuments([]);
                        setSelectedDocumentNames([]);
                        setAccountJournalEntries([]);
                        setUnreconciledEntries([]);
                        setReconcileDrafts({});
                        setPipelineStatus(
                            `Failed to load account pipeline data: ${String(error)}`,
                        );
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsLoadingDocuments(false);
                        setIsLoadingAccountJournal(false);
                        setIsLoadingUnreconciled(false);
                    }
                });
        }, 250);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledgerPath, scrapeAccount]);

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
        void setLastActiveTab(activeTab);
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

    async function reloadScrapeExtensions(
        path: string,
        preferredExtension: string,
    ) {
        setIsLoadingScrapeExtensions(true);
        try {
            const extensions = await listScrapeExtensions(path);
            setScrapeExtensions(extensions);
            setScrapeExtension((current) => {
                if (extensions.includes(preferredExtension)) {
                    return preferredExtension;
                }
                if (current.length > 0 && extensions.includes(current)) {
                    return current;
                }
                return extensions[0] ?? '';
            });
        } finally {
            setIsLoadingScrapeExtensions(false);
        }
    }

    async function handleLoadScrapeExtension(sourceType: 'zip' | 'directory') {
        if (!ledger) {
            return;
        }

        const selection = (await openDialog({
            directory: sourceType === 'directory',
            multiple: false,
            title:
                sourceType === 'directory'
                    ? 'Load extension from directory'
                    : 'Load extension from zip',
            ...(sourceType === 'zip'
                ? { filters: [{ name: 'ZIP archive', extensions: ['zip'] }] }
                : {}),
        })) as string | string[] | null;
        if (selection === null) {
            return;
        }
        const source = Array.isArray(selection) ? selection[0] : selection;
        if (typeof source !== 'string' || source.length === 0) {
            setScrapeStatus('Extension load canceled.');
            return;
        }

        setIsImportingScrapeExtension(true);
        setScrapeStatus('Loading extension...');
        try {
            let loadedExtensionName: string;
            try {
                loadedExtensionName = await loadScrapeExtension(
                    ledger.path,
                    source,
                    false,
                );
            } catch (error) {
                const message = String(error);
                if (!message.toLowerCase().includes('already exists')) {
                    throw error;
                }

                const shouldReplace = await confirmDialog(
                    `Extension already exists. Replace it?\n\n${message}`,
                    {
                        title: 'Replace extension?',
                        kind: 'warning',
                        okLabel: 'Replace',
                        cancelLabel: 'Cancel',
                    },
                );
                if (!shouldReplace) {
                    setScrapeStatus('Extension load canceled.');
                    return;
                }

                loadedExtensionName = await loadScrapeExtension(
                    ledger.path,
                    source,
                    true,
                );
            }

            await reloadScrapeExtensions(ledger.path, loadedExtensionName);
            // Save the loaded extension name in account config
            const account = scrapeAccount.trim();
            if (account.length > 0) {
                try {
                    await setAccountExtension(
                        ledger.path,
                        account,
                        loadedExtensionName,
                    );
                } catch {
                    // Non-fatal
                }
            }
            setScrapeStatus(`Loaded extension '${loadedExtensionName}'.`);
        } catch (error) {
            setScrapeStatus(`Failed to load extension: ${String(error)}`);
        } finally {
            setIsImportingScrapeExtension(false);
        }
    }

    async function handleLoadUnpackedExtension() {
        if (!ledger) {
            return;
        }

        const selection = (await openDialog({
            directory: true,
            multiple: false,
            title: 'Load unpacked extension directory',
        })) as string | string[] | null;
        if (selection === null) {
            return;
        }
        const source = Array.isArray(selection) ? selection[0] : selection;
        if (typeof source !== 'string' || source.length === 0) {
            setScrapeStatus('Extension load canceled.');
            return;
        }

        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setScrapeStatus('Select an account first.');
            return;
        }

        try {
            await setAccountExtension(ledger.path, account, source);
            setScrapeExtension(source);
            setScrapeStatus(`Set unpacked extension: ${source}`);
        } catch (error) {
            setScrapeStatus(
                `Failed to set unpacked extension: ${String(error)}`,
            );
        }
    }

    async function handleStartScrapeDebug() {
        if (!ledger) {
            return;
        }
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setScrapeStatus('Account is required.');
            return;
        }
        const extension = scrapeExtension.trim();
        if (extension.length === 0) {
            setScrapeStatus('Extension is required.');
            return;
        }

        setIsStartingScrapeDebug(true);
        setScrapeStatus(`Starting debug session for ${extension}...`);
        try {
            const socket = await startScrapeDebugSession(
                ledger.path,
                account,
                extension,
            );
            setScrapeDebugSocket(socket);
            setScrapeStatus(`Debug session started. Socket: ${socket}`);
        } catch (error) {
            setScrapeStatus(`Failed to start debug session: ${String(error)}`);
        } finally {
            setIsStartingScrapeDebug(false);
        }
    }

    async function handleStopScrapeDebug() {
        setIsStoppingScrapeDebug(true);
        try {
            await stopScrapeDebugSession();
            setScrapeDebugSocket(null);
            setScrapeStatus('Debug session stopped.');
        } catch (error) {
            setScrapeStatus(`Failed to stop debug session: ${String(error)}`);
        } finally {
            setIsStoppingScrapeDebug(false);
        }
    }

    async function handleCopyDebugSocket() {
        if (scrapeDebugSocket === null) {
            return;
        }
        try {
            await navigator.clipboard.writeText(scrapeDebugSocket);
            setScrapeStatus('Debug socket copied to clipboard.');
        } catch (error) {
            setScrapeStatus(`Failed to copy socket: ${String(error)}`);
        }
    }

    async function refreshAccountSecrets(accountInput: string) {
        const account = accountInput.trim();
        if (account.length === 0) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            return;
        }
        setIsLoadingAccountSecrets(true);
        try {
            const entries = await listAccountSecrets(account);
            setAccountSecrets(entries);
        } finally {
            setIsLoadingAccountSecrets(false);
        }
    }

    async function handleRefreshAccountSecrets() {
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setSecretsStatus('Account is required.');
            return;
        }
        try {
            await refreshAccountSecrets(account);
            setSecretsStatus(`Loaded secrets for ${account}.`);
        } catch (error) {
            setSecretsStatus(
                `Failed to load account secrets: ${String(error)}`,
            );
        }
    }

    async function handleSaveAccountSecret(mode: 'add' | 'reenter') {
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setSecretsStatus('Account is required.');
            return;
        }
        const domain = secretDomain.trim();
        if (domain.length === 0) {
            setSecretsStatus('Domain is required.');
            return;
        }
        const name = secretName.trim();
        if (name.length === 0) {
            setSecretsStatus('Name is required.');
            return;
        }
        if (secretValue.length === 0) {
            setSecretsStatus('Value is required.');
            return;
        }

        setIsSavingAccountSecret(true);
        try {
            if (mode === 'add') {
                await addAccountSecret(account, domain, name, secretValue);
            } else {
                await reenterAccountSecret(account, domain, name, secretValue);
            }
            await refreshAccountSecrets(account);
            setSecretValue('');
            setSecretsStatus(
                mode === 'add' ? 'Secret added.' : 'Secret re-entered.',
            );
        } catch (error) {
            setSecretsStatus(
                `Failed to ${mode === 'add' ? 'add' : 're-enter'} secret: ${String(error)}`,
            );
        } finally {
            setIsSavingAccountSecret(false);
        }
    }

    async function handleRemoveAccountSecret(domain: string, name: string) {
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setSecretsStatus('Account is required.');
            return;
        }
        const key = `${domain}/${name}`;
        setBusySecretKey(key);
        try {
            await removeAccountSecret(account, domain, name);
            await refreshAccountSecrets(account);
            if (secretDomain === domain && secretName === name) {
                setSecretValue('');
            }
            setSecretsStatus(`Removed ${key}.`);
        } catch (error) {
            setSecretsStatus(`Failed to remove ${key}: ${String(error)}`);
        } finally {
            setBusySecretKey(null);
        }
    }

    function handleReenterPreset(domain: string, name: string) {
        setSecretDomain(domain);
        setSecretName(name);
        setSecretValue('');
        setSecretsStatus(`Re-enter value for ${domain}/${name}.`);
    }

    function parseOptionalIndex(raw: string): {
        value: number | null;
        error: string | null;
    } {
        const trimmed = raw.trim();
        if (trimmed.length === 0) {
            return { value: null, error: null };
        }
        const parsed = Number.parseInt(trimmed, 10);
        if (!Number.isFinite(parsed) || Number.isNaN(parsed) || parsed < 0) {
            return {
                value: null,
                error: 'Posting index must be a non-negative integer.',
            };
        }
        return { value: parsed, error: null };
    }

    async function refreshAccountPipelineData(accountInput: string) {
        if (!ledger) {
            return;
        }
        const account = accountInput.trim();
        if (account.length === 0) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnreconciledEntries([]);
            setReconcileDrafts({});
            return;
        }

        setIsLoadingDocuments(true);
        setIsLoadingAccountJournal(true);
        setIsLoadingUnreconciled(true);
        try {
            const [fetchedDocuments, fetchedJournal, fetchedUnreconciled] =
                await Promise.all([
                    listDocuments(ledger.path, account),
                    getAccountJournal(ledger.path, account),
                    getUnreconciled(ledger.path, account),
                ]);
            setDocuments(fetchedDocuments);
            setSelectedDocumentNames((current) =>
                current.filter((name) =>
                    fetchedDocuments.some((doc) => doc.filename === name),
                ),
            );
            setAccountJournalEntries(fetchedJournal);
            setUnreconciledEntries(fetchedUnreconciled);
            setReconcileDrafts((current) => {
                const next: Record<string, ReconcileDraft> = {};
                for (const entry of fetchedUnreconciled) {
                    next[entry.id] = current[entry.id] ?? {
                        counterpartAccount: '',
                        postingIndex: '',
                    };
                }
                return next;
            });
            setTransferDraft((current) => ({
                ...current,
                account1:
                    current.account1.trim().length > 0
                        ? current.account1
                        : account,
            }));
        } finally {
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnreconciled(false);
        }
    }

    async function handleRefreshAccountPipelineData() {
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setPipelineStatus('Account is required.');
            return;
        }
        try {
            await refreshAccountPipelineData(account);
            setPipelineStatus(`Loaded documents and journals for ${account}.`);
        } catch (error) {
            setPipelineStatus(
                `Failed to refresh account pipeline data: ${String(error)}`,
            );
        }
    }

    function handleToggleDocumentSelection(filename: string, checked: boolean) {
        setSelectedDocumentNames((current) => {
            if (checked) {
                if (current.includes(filename)) {
                    return current;
                }
                return [...current, filename];
            }
            return current.filter((name) => name !== filename);
        });
        setPipelineStatus(null);
    }

    async function handleRunExtraction() {
        if (!ledger) {
            return;
        }
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setPipelineStatus('Account is required.');
            return;
        }
        const extension = scrapeExtension.trim();
        if (extension.length === 0) {
            setPipelineStatus('Extension is required.');
            return;
        }

        const documentNames =
            selectedDocumentNames.length > 0
                ? selectedDocumentNames
                : documents.map((doc) => doc.filename);
        if (documentNames.length === 0) {
            setPipelineStatus('No documents selected.');
            return;
        }

        setIsRunningExtraction(true);
        setPipelineStatus(
            `Running extraction for ${documentNames.length} document(s)...`,
        );
        try {
            const newCount = await runExtraction(
                ledger.path,
                account,
                extension,
                documentNames,
            );
            await refreshAccountPipelineData(account);
            setPipelineStatus(
                `Extraction complete. Added ${newCount} new transaction(s).`,
            );
        } catch (error) {
            setPipelineStatus(`Extraction failed: ${String(error)}`);
        } finally {
            setIsRunningExtraction(false);
        }
    }

    function handleSetReconcileDraft(
        entryId: string,
        patch: Partial<ReconcileDraft>,
    ) {
        setReconcileDrafts((current) => ({
            ...current,
            [entryId]: {
                counterpartAccount: '',
                postingIndex: '',
                ...current[entryId],
                ...patch,
            },
        }));
        setPipelineStatus(null);
    }

    async function handleReconcileAccountEntry(entryId: string) {
        if (!ledger) {
            return;
        }
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setPipelineStatus('Account is required.');
            return;
        }
        const draft = reconcileDrafts[entryId] ?? {
            counterpartAccount: '',
            postingIndex: '',
        };
        const counterpartAccount = draft.counterpartAccount.trim();
        if (counterpartAccount.length === 0) {
            setPipelineStatus('Counterpart account is required.');
            return;
        }

        const postingIndex = parseOptionalIndex(draft.postingIndex);
        if (postingIndex.error !== null) {
            setPipelineStatus(postingIndex.error);
            return;
        }

        setBusyReconcileEntryId(entryId);
        try {
            const glId = await reconcileEntry(
                ledger.path,
                account,
                entryId,
                counterpartAccount,
                postingIndex.value,
            );
            await refreshAccountPipelineData(account);
            setUnreconcileEntryId(entryId);
            setPipelineStatus(`Reconciled ${entryId} to ${glId}.`);
        } catch (error) {
            setPipelineStatus(`Reconcile failed: ${String(error)}`);
        } finally {
            setBusyReconcileEntryId(null);
        }
    }

    function handlePrepareUnreconcile(entryId: string) {
        setUnreconcileEntryId(entryId);
        setPipelineStatus(null);
    }

    async function handleUnreconcileAccountEntry() {
        if (!ledger) {
            return;
        }
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setPipelineStatus('Account is required.');
            return;
        }
        const entryId = unreconcileEntryId.trim();
        if (entryId.length === 0) {
            setPipelineStatus('Entry ID is required for unreconcile.');
            return;
        }

        const postingIndex = parseOptionalIndex(unreconcilePostingIndex);
        if (postingIndex.error !== null) {
            setPipelineStatus(postingIndex.error);
            return;
        }

        setIsUnreconcilingEntry(true);
        try {
            await unreconcileEntry(
                ledger.path,
                account,
                entryId,
                postingIndex.value,
            );
            await refreshAccountPipelineData(account);
            setPipelineStatus(`Unreconciled ${entryId}.`);
        } catch (error) {
            setPipelineStatus(`Unreconcile failed: ${String(error)}`);
        } finally {
            setIsUnreconcilingEntry(false);
        }
    }

    async function handleReconcileTransferPair() {
        if (!ledger) {
            return;
        }
        const account1 = transferDraft.account1.trim();
        const entryId1 = transferDraft.entryId1.trim();
        const account2 = transferDraft.account2.trim();
        const entryId2 = transferDraft.entryId2.trim();
        if (account1.length === 0 || entryId1.length === 0) {
            setPipelineStatus('Transfer account1 and entryId1 are required.');
            return;
        }
        if (account2.length === 0 || entryId2.length === 0) {
            setPipelineStatus('Transfer account2 and entryId2 are required.');
            return;
        }

        setIsReconcilingTransfer(true);
        try {
            const glId = await reconcileTransfer(
                ledger.path,
                account1,
                entryId1,
                account2,
                entryId2,
            );
            if (scrapeAccount.trim().length > 0) {
                await refreshAccountPipelineData(scrapeAccount);
            }
            setPipelineStatus(
                `Transfer reconciliation complete: ${entryId1} ↔ ${entryId2} (${glId}).`,
            );
        } catch (error) {
            setPipelineStatus(`Transfer reconcile failed: ${String(error)}`);
        } finally {
            setIsReconcilingTransfer(false);
        }
    }

    async function handleRunScrape() {
        if (!ledger) {
            return;
        }
        const account = scrapeAccount.trim();
        if (account.length === 0) {
            setScrapeStatus('Account is required.');
            return;
        }
        const extension = scrapeExtension.trim();
        if (extension.length === 0) {
            setScrapeStatus('Extension is required.');
            return;
        }

        setIsRunningScrape(true);
        setScrapeStatus(`Running ${extension} for ${account}...`);
        try {
            await runScrape(ledger.path, account, extension);
            setScrapeStatus(`Scrape completed for ${extension}.`);
            try {
                await refreshAccountPipelineData(account);
            } catch {
                // Surface scrape success first; pipeline reload errors are non-fatal here.
            }
        } catch (error) {
            setScrapeStatus(`Scrape failed: ${String(error)}`);
        } finally {
            setIsRunningScrape(false);
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
                            <AccountsTable accounts={ledger.accounts} />
                        </div>
                    ) : activeTab === 'transactions' ? (
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
                    ) : (
                        <div className="transactions-panel">
                            <section className="txn-form">
                                <div className="txn-form-header">
                                    <div>
                                        <h2>Run scrape</h2>
                                        <p>
                                            Choose an account and extension,
                                            then run the same scraper pipeline
                                            as the CLI command.
                                        </p>
                                    </div>
                                    <div className="header-actions">
                                        <button
                                            className="ghost-button"
                                            type="button"
                                            disabled={
                                                isImportingScrapeExtension
                                            }
                                            onClick={() => {
                                                void handleLoadScrapeExtension(
                                                    'zip',
                                                );
                                            }}
                                        >
                                            Load .zip...
                                        </button>
                                        <button
                                            className="ghost-button"
                                            type="button"
                                            disabled={
                                                isImportingScrapeExtension
                                            }
                                            onClick={() => {
                                                void handleLoadScrapeExtension(
                                                    'directory',
                                                );
                                            }}
                                        >
                                            Load directory...
                                        </button>
                                        <button
                                            className="ghost-button"
                                            type="button"
                                            disabled={
                                                isImportingScrapeExtension
                                            }
                                            onClick={() => {
                                                void handleLoadUnpackedExtension();
                                            }}
                                        >
                                            Load unpacked...
                                        </button>
                                    </div>
                                </div>
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Account</span>
                                        <input
                                            type="text"
                                            value={scrapeAccount}
                                            placeholder="Account name"
                                            list="scrape-account-options"
                                            onChange={(event) => {
                                                setScrapeAccount(
                                                    event.target.value,
                                                );
                                                setScrapeStatus(null);
                                                setSecretsStatus(null);
                                                setPipelineStatus(null);
                                            }}
                                        />
                                    </label>
                                    <label className="field">
                                        <span>Extension</span>
                                        <select
                                            value={scrapeExtension}
                                            onChange={(event) => {
                                                setScrapeExtension(
                                                    event.target.value,
                                                );
                                                setScrapeStatus(null);
                                                setPipelineStatus(null);
                                            }}
                                            disabled={isLoadingScrapeExtensions}
                                        >
                                            <option value="">
                                                {isLoadingScrapeExtensions
                                                    ? 'Loading extensions...'
                                                    : 'Select extension'}
                                            </option>
                                            {scrapeExtensions.map((name) => (
                                                <option key={name} value={name}>
                                                    {name}
                                                </option>
                                            ))}
                                            {scrapeExtension.length > 0 &&
                                            !scrapeExtensions.includes(
                                                scrapeExtension,
                                            ) ? (
                                                <option value={scrapeExtension}>
                                                    {scrapeExtension.includes(
                                                        '/',
                                                    ) ||
                                                    scrapeExtension.includes(
                                                        '\\',
                                                    )
                                                        ? `(unpacked) ${scrapeExtension.split('/').pop() ?? scrapeExtension}`
                                                        : scrapeExtension}
                                                </option>
                                            ) : null}
                                        </select>
                                    </label>
                                </div>
                                <datalist id="scrape-account-options">
                                    {scrapeAccountOptions.map((name) => (
                                        <option key={name} value={name} />
                                    ))}
                                </datalist>
                                <div className="txn-actions">
                                    <button
                                        type="button"
                                        className="primary-button"
                                        onClick={() => {
                                            void handleRunScrape();
                                        }}
                                        disabled={
                                            isRunningScrape ||
                                            isLoadingScrapeExtensions ||
                                            isImportingScrapeExtension ||
                                            isStartingScrapeDebug ||
                                            isStoppingScrapeDebug
                                        }
                                    >
                                        {isRunningScrape
                                            ? 'Running scrape...'
                                            : 'Run scrape'}
                                    </button>
                                </div>
                                <div className="txn-actions">
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleStartScrapeDebug();
                                        }}
                                        disabled={
                                            scrapeDebugSocket !== null ||
                                            isStartingScrapeDebug ||
                                            isStoppingScrapeDebug
                                        }
                                    >
                                        {isStartingScrapeDebug
                                            ? 'Starting debug...'
                                            : 'Start debug session'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleStopScrapeDebug();
                                        }}
                                        disabled={
                                            scrapeDebugSocket === null ||
                                            isStoppingScrapeDebug
                                        }
                                    >
                                        {isStoppingScrapeDebug
                                            ? 'Stopping debug...'
                                            : 'Stop debug session'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleCopyDebugSocket();
                                        }}
                                        disabled={scrapeDebugSocket === null}
                                    >
                                        Copy socket
                                    </button>
                                </div>
                                {scrapeDebugSocket === null ? null : (
                                    <p className="hint mono">
                                        Debug socket: {scrapeDebugSocket}
                                    </p>
                                )}
                                <section className="secrets-panel">
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Account secrets</h3>
                                            <p>
                                                Manage per-account keychain
                                                secrets for the selected scrape
                                                account.
                                            </p>
                                        </div>
                                        <div className="header-actions">
                                            <button
                                                className="ghost-button"
                                                type="button"
                                                onClick={() => {
                                                    void handleRefreshAccountSecrets();
                                                }}
                                                disabled={
                                                    scrapeAccount.trim()
                                                        .length === 0 ||
                                                    isLoadingAccountSecrets ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            >
                                                {isLoadingAccountSecrets
                                                    ? 'Refreshing...'
                                                    : 'Refresh secrets'}
                                            </button>
                                        </div>
                                    </div>
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Domain</span>
                                            <input
                                                type="text"
                                                value={secretDomain}
                                                placeholder="example.com"
                                                onChange={(event) => {
                                                    setSecretDomain(
                                                        event.target.value,
                                                    );
                                                    setSecretsStatus(null);
                                                }}
                                                disabled={
                                                    scrapeAccount.trim()
                                                        .length === 0 ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Name</span>
                                            <input
                                                type="text"
                                                value={secretName}
                                                placeholder="password"
                                                onChange={(event) => {
                                                    setSecretName(
                                                        event.target.value,
                                                    );
                                                    setSecretsStatus(null);
                                                }}
                                                disabled={
                                                    scrapeAccount.trim()
                                                        .length === 0 ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Value</span>
                                            <input
                                                type="password"
                                                autoComplete="new-password"
                                                value={secretValue}
                                                placeholder="Secret value"
                                                onChange={(event) => {
                                                    setSecretValue(
                                                        event.target.value,
                                                    );
                                                    setSecretsStatus(null);
                                                }}
                                                disabled={
                                                    scrapeAccount.trim()
                                                        .length === 0 ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            />
                                        </label>
                                    </div>
                                    <div className="txn-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleSaveAccountSecret(
                                                    'add',
                                                );
                                            }}
                                            disabled={
                                                scrapeAccount.trim().length ===
                                                    0 ||
                                                isSavingAccountSecret ||
                                                busySecretKey !== null
                                            }
                                        >
                                            {isSavingAccountSecret
                                                ? 'Saving...'
                                                : 'Add secret'}
                                        </button>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleSaveAccountSecret(
                                                    'reenter',
                                                );
                                            }}
                                            disabled={
                                                scrapeAccount.trim().length ===
                                                    0 ||
                                                isSavingAccountSecret ||
                                                busySecretKey !== null
                                            }
                                        >
                                            {isSavingAccountSecret
                                                ? 'Saving...'
                                                : 'Re-enter secret'}
                                        </button>
                                    </div>
                                    {isLoadingAccountSecrets ? (
                                        <p className="status">
                                            Loading account secrets...
                                        </p>
                                    ) : accountSecrets.length === 0 ? (
                                        <p className="hint">
                                            {scrapeAccount.trim().length === 0
                                                ? 'Choose an account to manage secrets.'
                                                : 'No secrets stored for this account.'}
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Domain</th>
                                                        <th>Name</th>
                                                        <th>Actions</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {accountSecrets.map(
                                                        (entry) => {
                                                            const key = `${entry.domain}/${entry.name}`;
                                                            const isBusy =
                                                                busySecretKey ===
                                                                key;
                                                            return (
                                                                <tr key={key}>
                                                                    <td>
                                                                        {
                                                                            entry.domain
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        {
                                                                            entry.name
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        <div className="txn-actions">
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    handleReenterPreset(
                                                                                        entry.domain,
                                                                                        entry.name,
                                                                                    );
                                                                                }}
                                                                                disabled={
                                                                                    isSavingAccountSecret ||
                                                                                    busySecretKey !==
                                                                                        null
                                                                                }
                                                                            >
                                                                                Re-enter
                                                                            </button>
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    void handleRemoveAccountSecret(
                                                                                        entry.domain,
                                                                                        entry.name,
                                                                                    );
                                                                                }}
                                                                                disabled={
                                                                                    isSavingAccountSecret ||
                                                                                    busySecretKey !==
                                                                                        null
                                                                                }
                                                                            >
                                                                                {isBusy
                                                                                    ? 'Removing...'
                                                                                    : 'Remove'}
                                                                            </button>
                                                                        </div>
                                                                    </td>
                                                                </tr>
                                                            );
                                                        },
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                    {secretsStatus === null ? null : (
                                        <p className="status">
                                            {secretsStatus}
                                        </p>
                                    )}
                                </section>
                                <section className="pipeline-panel">
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Extraction pipeline</h3>
                                            <p>
                                                Select documents, run
                                                extraction, and review
                                                account-level journal and
                                                reconciliation state.
                                            </p>
                                        </div>
                                        <div className="header-actions">
                                            <button
                                                className="ghost-button"
                                                type="button"
                                                onClick={() => {
                                                    void handleRefreshAccountPipelineData();
                                                }}
                                                disabled={
                                                    scrapeAccount.trim()
                                                        .length === 0 ||
                                                    isLoadingDocuments ||
                                                    isLoadingAccountJournal ||
                                                    isLoadingUnreconciled ||
                                                    isRunningExtraction ||
                                                    busyReconcileEntryId !==
                                                        null ||
                                                    isUnreconcilingEntry ||
                                                    isReconcilingTransfer
                                                }
                                            >
                                                {isLoadingDocuments ||
                                                isLoadingAccountJournal ||
                                                isLoadingUnreconciled
                                                    ? 'Refreshing...'
                                                    : 'Refresh pipeline'}
                                            </button>
                                        </div>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setSelectedDocumentNames(
                                                    documents.map(
                                                        (doc) => doc.filename,
                                                    ),
                                                );
                                                setPipelineStatus(null);
                                            }}
                                            disabled={
                                                isLoadingDocuments ||
                                                documents.length === 0
                                            }
                                        >
                                            Select all docs
                                        </button>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setSelectedDocumentNames([]);
                                                setPipelineStatus(null);
                                            }}
                                            disabled={
                                                isLoadingDocuments ||
                                                selectedDocumentNames.length ===
                                                    0
                                            }
                                        >
                                            Clear selection
                                        </button>
                                        <button
                                            type="button"
                                            className="primary-button"
                                            onClick={() => {
                                                void handleRunExtraction();
                                            }}
                                            disabled={
                                                isRunningExtraction ||
                                                scrapeAccount.trim().length ===
                                                    0 ||
                                                scrapeExtension.trim()
                                                    .length === 0
                                            }
                                        >
                                            {isRunningExtraction
                                                ? 'Running extraction...'
                                                : `Run extraction (${selectedDocumentNames.length > 0 ? selectedDocumentNames.length : documents.length})`}
                                        </button>
                                    </div>
                                    {isLoadingDocuments ? (
                                        <p className="status">
                                            Loading documents...
                                        </p>
                                    ) : documents.length === 0 ? (
                                        <p className="hint">
                                            {scrapeAccount.trim().length === 0
                                                ? 'Choose an account to view documents.'
                                                : 'No documents found for this account.'}
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Select</th>
                                                        <th>Document</th>
                                                        <th>Coverage End</th>
                                                        <th>Scrape Session</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {documents.map(
                                                        (document) => (
                                                            <tr
                                                                key={
                                                                    document.filename
                                                                }
                                                            >
                                                                <td>
                                                                    <input
                                                                        type="checkbox"
                                                                        checked={selectedDocumentNames.includes(
                                                                            document.filename,
                                                                        )}
                                                                        onChange={(
                                                                            event,
                                                                        ) => {
                                                                            handleToggleDocumentSelection(
                                                                                document.filename,
                                                                                event
                                                                                    .target
                                                                                    .checked,
                                                                            );
                                                                        }}
                                                                    />
                                                                </td>
                                                                <td>
                                                                    <span className="mono">
                                                                        {
                                                                            document.filename
                                                                        }
                                                                    </span>
                                                                </td>
                                                                <td className="mono">
                                                                    {document
                                                                        .info
                                                                        ?.coverageEndDate ??
                                                                        '-'}
                                                                </td>
                                                                <td className="mono">
                                                                    {document
                                                                        .info
                                                                        ?.scrapeSessionId ??
                                                                        '-'}
                                                                </td>
                                                            </tr>
                                                        ),
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Account journal</h3>
                                            <p>
                                                Entries extracted for this
                                                account with evidence and
                                                reconciliation status.
                                            </p>
                                        </div>
                                    </div>
                                    {isLoadingAccountJournal ? (
                                        <p className="status">
                                            Loading account journal...
                                        </p>
                                    ) : accountJournalEntries.length === 0 ? (
                                        <p className="hint">
                                            {scrapeAccount.trim().length === 0
                                                ? 'Choose an account to view its journal.'
                                                : 'No account journal entries found.'}
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Date</th>
                                                        <th>Status</th>
                                                        <th>ID</th>
                                                        <th>Description</th>
                                                        <th>Evidence</th>
                                                        <th>Reconciled</th>
                                                        <th>Actions</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {accountJournalEntries.map(
                                                        (entry) => (
                                                            <tr key={entry.id}>
                                                                <td className="mono">
                                                                    {entry.date}
                                                                </td>
                                                                <td>
                                                                    {
                                                                        entry.status
                                                                    }
                                                                </td>
                                                                <td className="mono">
                                                                    {entry.id}
                                                                </td>
                                                                <td>
                                                                    {
                                                                        entry.description
                                                                    }
                                                                </td>
                                                                <td className="mono">
                                                                    {
                                                                        entry
                                                                            .evidence
                                                                            .length
                                                                    }
                                                                </td>
                                                                <td className="mono">
                                                                    {entry.reconciled ??
                                                                        '-'}
                                                                </td>
                                                                <td>
                                                                    <div className="pipeline-row-actions">
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            onClick={() => {
                                                                                handlePrepareUnreconcile(
                                                                                    entry.id,
                                                                                );
                                                                            }}
                                                                        >
                                                                            Set
                                                                            for
                                                                            unreconcile
                                                                        </button>
                                                                    </div>
                                                                </td>
                                                            </tr>
                                                        ),
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Reconciliation queue</h3>
                                            <p>
                                                Assign counterpart accounts for
                                                unreconciled entries.
                                            </p>
                                        </div>
                                    </div>
                                    {isLoadingUnreconciled ? (
                                        <p className="status">
                                            Loading unreconciled entries...
                                        </p>
                                    ) : unreconciledEntries.length === 0 ? (
                                        <p className="hint">
                                            {scrapeAccount.trim().length === 0
                                                ? 'Choose an account to view unreconciled entries.'
                                                : 'No unreconciled entries for this account.'}
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Date</th>
                                                        <th>ID</th>
                                                        <th>Description</th>
                                                        <th>Counterpart</th>
                                                        <th>Posting Index</th>
                                                        <th>Actions</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {unreconciledEntries.map(
                                                        (entry) => {
                                                            const draft =
                                                                reconcileDrafts[
                                                                    entry.id
                                                                ] ?? {
                                                                    counterpartAccount:
                                                                        '',
                                                                    postingIndex:
                                                                        '',
                                                                };
                                                            const isBusy =
                                                                busyReconcileEntryId ===
                                                                entry.id;
                                                            return (
                                                                <tr
                                                                    key={
                                                                        entry.id
                                                                    }
                                                                >
                                                                    <td className="mono">
                                                                        {
                                                                            entry.date
                                                                        }
                                                                    </td>
                                                                    <td className="mono">
                                                                        {
                                                                            entry.id
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        {
                                                                            entry.description
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        <input
                                                                            type="text"
                                                                            value={
                                                                                draft.counterpartAccount
                                                                            }
                                                                            placeholder="Expenses:Food"
                                                                            onChange={(
                                                                                event,
                                                                            ) => {
                                                                                handleSetReconcileDraft(
                                                                                    entry.id,
                                                                                    {
                                                                                        counterpartAccount:
                                                                                            event
                                                                                                .target
                                                                                                .value,
                                                                                    },
                                                                                );
                                                                            }}
                                                                        />
                                                                    </td>
                                                                    <td>
                                                                        <input
                                                                            type="text"
                                                                            value={
                                                                                draft.postingIndex
                                                                            }
                                                                            placeholder="optional"
                                                                            onChange={(
                                                                                event,
                                                                            ) => {
                                                                                handleSetReconcileDraft(
                                                                                    entry.id,
                                                                                    {
                                                                                        postingIndex:
                                                                                            event
                                                                                                .target
                                                                                                .value,
                                                                                    },
                                                                                );
                                                                            }}
                                                                        />
                                                                    </td>
                                                                    <td>
                                                                        <div className="pipeline-row-actions">
                                                                            <button
                                                                                type="button"
                                                                                className="primary-button"
                                                                                onClick={() => {
                                                                                    void handleReconcileAccountEntry(
                                                                                        entry.id,
                                                                                    );
                                                                                }}
                                                                                disabled={
                                                                                    isBusy ||
                                                                                    isUnreconcilingEntry ||
                                                                                    isReconcilingTransfer
                                                                                }
                                                                            >
                                                                                {isBusy
                                                                                    ? 'Reconciling...'
                                                                                    : 'Reconcile'}
                                                                            </button>
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    setTransferDraft(
                                                                                        (
                                                                                            current,
                                                                                        ) => ({
                                                                                            ...current,
                                                                                            account1:
                                                                                                scrapeAccount,
                                                                                            entryId1:
                                                                                                entry.id,
                                                                                        }),
                                                                                    );
                                                                                    setPipelineStatus(
                                                                                        null,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                Use
                                                                                as
                                                                                A
                                                                            </button>
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    setTransferDraft(
                                                                                        (
                                                                                            current,
                                                                                        ) => ({
                                                                                            ...current,
                                                                                            account2:
                                                                                                scrapeAccount,
                                                                                            entryId2:
                                                                                                entry.id,
                                                                                        }),
                                                                                    );
                                                                                    setPipelineStatus(
                                                                                        null,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                Use
                                                                                as
                                                                                B
                                                                            </button>
                                                                        </div>
                                                                    </td>
                                                                </tr>
                                                            );
                                                        },
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Unreconcile entry ID</span>
                                            <input
                                                type="text"
                                                value={unreconcileEntryId}
                                                placeholder="entry id"
                                                onChange={(event) => {
                                                    setUnreconcileEntryId(
                                                        event.target.value,
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>
                                                Unreconcile posting index
                                                (optional)
                                            </span>
                                            <input
                                                type="text"
                                                value={unreconcilePostingIndex}
                                                placeholder="0"
                                                onChange={(event) => {
                                                    setUnreconcilePostingIndex(
                                                        event.target.value,
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleUnreconcileAccountEntry();
                                            }}
                                            disabled={isUnreconcilingEntry}
                                        >
                                            {isUnreconcilingEntry
                                                ? 'Unreconciling...'
                                                : 'Unreconcile entry'}
                                        </button>
                                    </div>
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Transfer reconciliation</h3>
                                            <p>
                                                Match two entries across
                                                accounts as a transfer.
                                            </p>
                                        </div>
                                    </div>
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Account 1</span>
                                            <input
                                                type="text"
                                                value={transferDraft.account1}
                                                placeholder="account1"
                                                onChange={(event) => {
                                                    setTransferDraft(
                                                        (current) => ({
                                                            ...current,
                                                            account1:
                                                                event.target
                                                                    .value,
                                                        }),
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Entry ID 1</span>
                                            <input
                                                type="text"
                                                value={transferDraft.entryId1}
                                                placeholder="entry id"
                                                onChange={(event) => {
                                                    setTransferDraft(
                                                        (current) => ({
                                                            ...current,
                                                            entryId1:
                                                                event.target
                                                                    .value,
                                                        }),
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Account 2</span>
                                            <input
                                                type="text"
                                                value={transferDraft.account2}
                                                placeholder="account2"
                                                onChange={(event) => {
                                                    setTransferDraft(
                                                        (current) => ({
                                                            ...current,
                                                            account2:
                                                                event.target
                                                                    .value,
                                                        }),
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Entry ID 2</span>
                                            <input
                                                type="text"
                                                value={transferDraft.entryId2}
                                                placeholder="entry id"
                                                onChange={(event) => {
                                                    setTransferDraft(
                                                        (current) => ({
                                                            ...current,
                                                            entryId2:
                                                                event.target
                                                                    .value,
                                                        }),
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleReconcileTransferPair();
                                            }}
                                            disabled={isReconcilingTransfer}
                                        >
                                            {isReconcilingTransfer
                                                ? 'Reconciling transfer...'
                                                : 'Reconcile transfer'}
                                        </button>
                                    </div>
                                </section>
                                {scrapeExtensions.length === 0 &&
                                !isLoadingScrapeExtensions ? (
                                    <p className="hint">
                                        No runnable extensions found in
                                        extensions/*/driver.mjs.
                                    </p>
                                ) : null}
                                {scrapeStatus === null ? null : (
                                    <p className="status">{scrapeStatus}</p>
                                )}
                                {pipelineStatus === null ? null : (
                                    <p className="status">{pipelineStatus}</p>
                                )}
                            </section>
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
