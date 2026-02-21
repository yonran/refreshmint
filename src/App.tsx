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
    type AccountSecretEntry,
    type AccountJournalEntry,
    type AmountStyleHint,
    type AmountTotal,
    addLoginSecret,
    addTransaction,
    addTransactionText,
    createLogin,
    deleteLogin,
    getLoginAccountJournal,
    getLoginConfig,
    getLoginAccountUnreconciled,
    getScrapeDebugSessionSocket,
    type LoginConfig,
    listLoginAccountDocuments,
    listLoginSecrets,
    listLogins,
    listScrapeExtensions,
    loadScrapeExtension,
    migrateLedger,
    openLedger,
    type AccountRow,
    type DocumentWithInfo,
    type LedgerView,
    type MigrationOutcome,
    type NewTransactionInput,
    type PostingRow,
    reconcileLoginAccountEntry,
    reconcileTransfer,
    reenterLoginSecret,
    removeLoginAccount,
    removeLoginSecret,
    runLoginAccountExtraction,
    runScrapeForLogin,
    setLoginAccount,
    setLoginExtension,
    startScrapeDebugSession,
    stopScrapeDebugSession,
    type SecretEntry,
    syncLoginSecretsForExtension,
    type TransactionRow,
    unreconcileLoginAccountEntry,
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

type SecretPromptState = {
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
};

type LoginAccountMapping = {
    loginName: string;
    label: string;
    extension: string;
};

function secretPairKey(domain: string, name: string): string {
    return `${domain}/${name}`;
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
    const [legacyMigrationPreview, setLegacyMigrationPreview] =
        useState<MigrationOutcome | null>(null);
    const [isCheckingLegacyMigration, setIsCheckingLegacyMigration] =
        useState(false);
    const [isMigratingLegacyLedger, setIsMigratingLegacyLedger] =
        useState(false);
    const [loginNames, setLoginNames] = useState<string[]>([]);
    const [loginConfigsByName, setLoginConfigsByName] = useState<
        Record<string, LoginConfig>
    >({});
    const [selectedLoginName, setSelectedLoginName] = useState('');
    const [selectedLoginExtensionDraft, setSelectedLoginExtensionDraft] =
        useState('');
    const [newLoginName, setNewLoginName] = useState('');
    const [newLoginExtension, setNewLoginExtension] = useState('');
    const [loginLabelDraft, setLoginLabelDraft] = useState('');
    const [loginGlAccountDraft, setLoginGlAccountDraft] = useState('');
    const [loginConfigStatus, setLoginConfigStatus] = useState<string | null>(
        null,
    );
    const [isLoadingLoginConfigs, setIsLoadingLoginConfigs] = useState(false);
    const [isSavingLoginConfig, setIsSavingLoginConfig] = useState(false);
    const [loginConfigsReloadToken, setLoginConfigsReloadToken] = useState(0);
    const [loginAccountMappings, setLoginAccountMappings] = useState<
        Record<string, LoginAccountMapping[]>
    >({});
    const [accountSecrets, setAccountSecrets] = useState<AccountSecretEntry[]>(
        [],
    );
    const [requiredSecretsForExtension, setRequiredSecretsForExtension] =
        useState<SecretEntry[]>([]);
    const [hasRequiredSecretsSync, setHasRequiredSecretsSync] = useState(false);
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
    const secretDomainRef = useRef('');
    const secretNameRef = useRef('');
    const secretPromptResolverRef = useRef<
        ((confirmed: boolean) => void) | null
    >(null);
    const ledgerPath = ledger?.path ?? null;
    const scrapeAccountOptions = ledger
        ? ledger.accounts
              .map((account) => account.name.trim())
              .filter(
                  (name, index, names) =>
                      name.length > 0 && names.indexOf(name) === index,
              )
        : [];
    const selectedScrapeAccount = scrapeAccount.trim();
    const selectedAccountMappings =
        selectedScrapeAccount.length === 0
            ? []
            : (loginAccountMappings[selectedScrapeAccount] ?? []);
    const selectedLoginMapping: LoginAccountMapping | null =
        selectedAccountMappings.length === 1
            ? (selectedAccountMappings[0] ?? null)
            : null;
    const selectedLoginMappingError =
        selectedScrapeAccount.length === 0
            ? null
            : selectedAccountMappings.length === 0
              ? `No login mapping found for account '${selectedScrapeAccount}'. Run migration and set a login account mapping.`
              : `Account '${selectedScrapeAccount}' has multiple login mappings. Resolve GL mapping conflicts first.`;
    const hasResolvedLoginMapping = selectedLoginMapping !== null;
    const selectedLoginMappingSummary =
        selectedLoginMapping === null
            ? null
            : `${selectedLoginMapping.loginName}/${selectedLoginMapping.label}`;
    const activeScrapeLoginName =
        selectedLoginMapping?.loginName ?? (selectedLoginName.trim() || null);
    const hasActiveScrapeLogin = activeScrapeLoginName !== null;
    const activeSecretsLoginName = activeScrapeLoginName;
    const hasActiveSecretsLogin = activeSecretsLoginName !== null;
    const selectedLoginConfig: LoginConfig | null =
        selectedLoginName.length === 0
            ? null
            : (loginConfigsByName[selectedLoginName] ?? null);
    const selectedLoginAccounts =
        selectedLoginConfig === null
            ? []
            : Object.entries(selectedLoginConfig.accounts).sort(([a], [b]) =>
                  a.localeCompare(b),
              );
    const conflictingGlAccountSet = new Set(
        ledger?.glAccountConflicts.map((conflict) => conflict.glAccount) ?? [],
    );
    const selectedScrapeAccountHasConflict =
        selectedScrapeAccount.length > 0 &&
        conflictingGlAccountSet.has(selectedScrapeAccount);
    const requestLoginConfigReload = useCallback(() => {
        setLoginConfigsReloadToken((current) => current + 1);
    }, []);
    const requiredSecretKeySet = new Set(
        requiredSecretsForExtension.map((entry) =>
            secretPairKey(entry.domain, entry.name),
        ),
    );
    const trimmedSecretDomain = secretDomain.trim();
    const trimmedSecretName = secretName.trim();
    const currentSecretEntry = accountSecrets.find(
        (entry) =>
            entry.domain === trimmedSecretDomain &&
            entry.name === trimmedSecretName,
    );
    const currentSecretPairExists = currentSecretEntry !== undefined;
    const currentSecretHasValue = currentSecretEntry?.hasValue ?? false;
    const secretValuePlaceholder = currentSecretHasValue ? '●●●●●●●●' : '';
    const extraSecretCount = hasRequiredSecretsSync
        ? accountSecrets.reduce((count, entry) => {
              const key = secretPairKey(entry.domain, entry.name);
              return requiredSecretKeySet.has(key) ? count : count + 1;
          }, 0)
        : 0;

    useEffect(() => {
        secretDomainRef.current = secretDomain;
    }, [secretDomain]);

    useEffect(() => {
        secretNameRef.current = secretName;
    }, [secretName]);

    useEffect(() => {
        return () => {
            if (secretPromptResolverRef.current !== null) {
                secretPromptResolverRef.current(false);
                secretPromptResolverRef.current = null;
            }
        };
    }, []);

    useEffect(() => {
        if (ledger) {
            setTransactionDraft(createTransactionDraft());
            setRawDraft('');
            setAddStatus(null);
            setDraftStatus(null);
            setScrapeStatus(null);
            setScrapeDebugSocket(null);
            setScrapeAccount('');
            setLoginNames([]);
            setLoginConfigsByName({});
            setSelectedLoginName('');
            setSelectedLoginExtensionDraft('');
            setNewLoginName('');
            setNewLoginExtension('');
            setLoginLabelDraft('');
            setLoginGlAccountDraft('');
            setLoginConfigStatus(null);
            setIsLoadingLoginConfigs(false);
            setIsSavingLoginConfig(false);
            setLoginConfigsReloadToken(0);
            setLoginAccountMappings({});
            setAccountSecrets([]);
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
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
            setLegacyMigrationPreview(null);
            setIsCheckingLegacyMigration(false);
            return;
        }

        let cancelled = false;
        setIsCheckingLegacyMigration(true);
        void migrateLedger(ledgerPath, true)
            .then((outcome) => {
                if (cancelled) {
                    return;
                }
                if (
                    outcome.migrated.length === 0 &&
                    outcome.skipped.length === 0
                ) {
                    setLegacyMigrationPreview(null);
                } else {
                    setLegacyMigrationPreview(outcome);
                }
            })
            .catch(() => {
                if (!cancelled) {
                    setLegacyMigrationPreview(null);
                }
            })
            .finally(() => {
                if (!cancelled) {
                    setIsCheckingLegacyMigration(false);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    useEffect(() => {
        if (ledgerPath === null) {
            setLoginNames([]);
            setLoginConfigsByName({});
            setSelectedLoginName('');
            setLoginAccountMappings({});
            setIsLoadingLoginConfigs(false);
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
                for (const { loginName, config } of configs) {
                    configMap[loginName] = config;
                    const extension = config.extension?.trim() ?? '';
                    for (const [label, mapping] of Object.entries(
                        config.accounts,
                    )) {
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
                setSelectedLoginName((current) => {
                    if (current.length > 0 && logins.includes(current)) {
                        return current;
                    }
                    return logins[0] ?? '';
                });
                setLoginAccountMappings(mappings);
            })
            .catch((error: unknown) => {
                if (!cancelled) {
                    setLoginNames([]);
                    setLoginConfigsByName({});
                    setSelectedLoginName('');
                    setLoginAccountMappings({});
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
    }, [ledgerPath, loginConfigsReloadToken]);

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
        const extension = selectedLoginConfig?.extension?.trim() ?? '';
        setSelectedLoginExtensionDraft(extension);
    }, [selectedLoginConfig]);

    useEffect(() => {
        if (ledgerPath === null) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
            return;
        }

        if (activeSecretsLoginName === null) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
            return;
        }

        const loginName = activeSecretsLoginName;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingAccountSecrets(true);
            void listLoginSecrets(loginName)
                .then((entries) => {
                    if (!cancelled) {
                        setAccountSecrets(entries);
                    }
                })
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setAccountSecrets([]);
                        setSecretsStatus(
                            `Failed to load login secrets: ${String(error)}`,
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
    }, [activeSecretsLoginName, ledgerPath]);

    useEffect(() => {
        if (ledgerPath === null) {
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
            return;
        }

        const extension = scrapeExtension.trim();
        if (activeSecretsLoginName === null || extension.length === 0) {
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
            return;
        }

        const loginName = activeSecretsLoginName;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingAccountSecrets(true);
            void syncLoginSecretsForExtension(ledgerPath, loginName, extension)
                .then((result) => {
                    if (cancelled) {
                        return;
                    }
                    setRequiredSecretsForExtension(result.required);
                    setHasRequiredSecretsSync(true);

                    const currentDomain = secretDomainRef.current.trim();
                    const currentName = secretNameRef.current.trim();
                    const currentHasPair =
                        currentDomain.length > 0 && currentName.length > 0;
                    const currentKey = secretPairKey(
                        currentDomain,
                        currentName,
                    );
                    const requiredKeySet = new Set(
                        result.required.map((entry) =>
                            secretPairKey(entry.domain, entry.name),
                        ),
                    );

                    if (currentHasPair && !requiredKeySet.has(currentKey)) {
                        setSecretDomain('');
                        setSecretName('');
                        setSecretValue('');
                    } else if (!currentHasPair && result.required.length > 0) {
                        const first = result.required[0];
                        if (first !== undefined) {
                            setSecretDomain(first.domain);
                            setSecretName(first.name);
                        }
                    }

                    const requiredCount = result.required.length;
                    const addedCount = result.added.length;
                    const existingCount = result.existingRequired.length;
                    const extraCount = result.extras.length;
                    if (requiredCount === 0) {
                        setSecretsStatus(
                            'No declared secrets for this extension.',
                        );
                    } else {
                        const extraSuffix =
                            extraCount > 0
                                ? ` ${extraCount} extra secret${extraCount === 1 ? '' : 's'} found.`
                                : '';
                        setSecretsStatus(
                            `Prepared ${requiredCount} required secret${requiredCount === 1 ? '' : 's'}: ${addedCount} added, ${existingCount} already stored.${extraSuffix}`,
                        );
                    }

                    return listLoginSecrets(loginName)
                        .then((entries) => {
                            if (!cancelled) {
                                setAccountSecrets(entries);
                            }
                        })
                        .catch((error: unknown) => {
                            if (!cancelled) {
                                setAccountSecrets([]);
                                setSecretsStatus(
                                    `Failed to load login secrets: ${String(error)}`,
                                );
                            }
                        });
                })
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setRequiredSecretsForExtension([]);
                        setHasRequiredSecretsSync(false);
                        setSecretsStatus(
                            `Failed to prepare secrets: ${String(error)}`,
                        );
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsLoadingAccountSecrets(false);
                    }
                });
        }, 200);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [activeSecretsLoginName, ledgerPath, scrapeExtension]);

    // Load login config and auto-populate extension when account mapping changes.
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtension('');
            return;
        }

        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeExtension('');
            return;
        }

        // Prevent stale extension state from bleeding across selections.
        setScrapeExtension('');

        let cancelled = false;
        const timer = window.setTimeout(() => {
            void getLoginConfig(ledgerPath, loginName)
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
    }, [activeScrapeLoginName, ledgerPath]);

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

        if (selectedLoginMapping === null) {
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

        const mapping = selectedLoginMapping;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingDocuments(true);
            setIsLoadingAccountJournal(true);
            setIsLoadingUnreconciled(true);
            void Promise.all([
                listLoginAccountDocuments(
                    ledgerPath,
                    mapping.loginName,
                    mapping.label,
                ),
                getLoginAccountJournal(
                    ledgerPath,
                    mapping.loginName,
                    mapping.label,
                ),
                getLoginAccountUnreconciled(
                    ledgerPath,
                    mapping.loginName,
                    mapping.label,
                ),
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
                                    : selectedScrapeAccount,
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
                            `Failed to load login pipeline data: ${String(error)}`,
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
    }, [ledgerPath, selectedLoginMapping, selectedScrapeAccount]);

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
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before creating a new ledger',
        );
        if (!canContinue) {
            return;
        }
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
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before opening another ledger',
        );
        if (!canContinue) {
            return;
        }
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
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before opening another ledger',
        );
        if (!canContinue) {
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
            // Save the loaded extension name in login config when a login is selected.
            const loginName = activeScrapeLoginName;
            if (loginName !== null) {
                try {
                    await setLoginExtension(
                        ledger.path,
                        loginName,
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

        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus(
                selectedLoginMappingError ?? 'Select a login first.',
            );
            return;
        }

        try {
            await setLoginExtension(ledger.path, loginName, source);
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
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus(selectedLoginMappingError ?? 'Login is required.');
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
                loginName,
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

    async function refreshLoginSecrets(loginNameInput: string) {
        const loginName = loginNameInput.trim();
        if (loginName.length === 0) {
            setAccountSecrets([]);
            setIsLoadingAccountSecrets(false);
            return;
        }
        setIsLoadingAccountSecrets(true);
        try {
            const entries = await listLoginSecrets(loginName);
            setAccountSecrets(entries);
        } finally {
            setIsLoadingAccountSecrets(false);
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

    async function confirmSaveOrDiscardSecretValue(context: string) {
        if (secretValue.length === 0) {
            return true;
        }

        const shouldSave = await promptSecretDecision({
            title: 'Unsaved secret value',
            message: `You have an unsaved secret value ${context}. Save it first?`,
            confirmLabel: 'Save',
            cancelLabel: 'Discard',
        });
        if (!shouldSave) {
            setSecretValue('');
            setSecretsStatus('Discarded unsaved secret value.');
            return true;
        }

        const mode: 'add' | 'reenter' = currentSecretPairExists
            ? 'reenter'
            : 'add';
        const saved = await handleSaveAccountSecret(mode);
        if (saved) {
            return true;
        }
        const shouldDiscardAfterFailedSave = await promptSecretDecision({
            title: 'Save failed',
            message: `Could not save the secret value ${context}. Discard it and continue?`,
            confirmLabel: 'Discard',
            cancelLabel: 'Keep editing',
        });

        if (shouldDiscardAfterFailedSave) {
            setSecretValue('');
            setSecretsStatus('Discarded unsaved secret value.');
            return true;
        }
        return false;
    }

    async function handleRefreshAccountSecrets() {
        if (activeSecretsLoginName === null) {
            setSecretsStatus(
                selectedLoginMappingError ??
                    'Select a login mapping or login first.',
            );
            return;
        }
        try {
            await refreshLoginSecrets(activeSecretsLoginName);
            setSecretsStatus(
                `Loaded login secrets for '${activeSecretsLoginName}'.`,
            );
        } catch (error) {
            setSecretsStatus(`Failed to load login secrets: ${String(error)}`);
        }
    }

    async function handleSaveAccountSecret(mode: 'add' | 'reenter') {
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSecretsStatus(
                selectedLoginMappingError ??
                    'Select a login mapping or login first.',
            );
            return false;
        }
        const domain = secretDomain.trim();
        if (domain.length === 0) {
            setSecretsStatus('Domain is required.');
            return false;
        }
        const name = secretName.trim();
        if (name.length === 0) {
            setSecretsStatus('Name is required.');
            return false;
        }
        const existingEntry = accountSecrets.find(
            (entry) => entry.domain === domain && entry.name === name,
        );
        const existingEntryHasValue = existingEntry?.hasValue === true;
        if (mode === 'add' && existingEntry !== undefined) {
            setSecretsStatus(
                `Secret pair ${domain}/${name} already exists. Use Set/Change value.`,
            );
            return false;
        }
        if (mode === 'reenter' && existingEntry === undefined) {
            setSecretsStatus(
                `Secret pair ${domain}/${name} does not exist. Use Add new pair first.`,
            );
            return false;
        }
        if (secretValue.length === 0) {
            setSecretsStatus('Value is required.');
            return false;
        }

        setIsSavingAccountSecret(true);
        try {
            if (mode === 'add') {
                await addLoginSecret(loginName, domain, name, secretValue);
            } else {
                await reenterLoginSecret(loginName, domain, name, secretValue);
            }
            await refreshLoginSecrets(loginName);
            setSecretValue('');
            setSecretsStatus(
                mode === 'add'
                    ? 'Secret pair added.'
                    : existingEntryHasValue
                      ? 'Secret value changed.'
                      : 'Secret value set.',
            );
            return true;
        } catch (error) {
            setSecretsStatus(
                `Failed to ${mode === 'add' ? 'add pair' : 'save secret value'}: ${String(error)}`,
            );
            return false;
        } finally {
            setIsSavingAccountSecret(false);
        }
    }

    async function handleRemoveAccountSecret(domain: string, name: string) {
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSecretsStatus(
                selectedLoginMappingError ??
                    'Select a login mapping or login first.',
            );
            return;
        }
        const key = `${domain}/${name}`;
        setBusySecretKey(key);
        try {
            await removeLoginSecret(loginName, domain, name);
            await refreshLoginSecrets(loginName);
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

    async function handleReenterPreset(
        domain: string,
        name: string,
        hasValue: boolean,
    ) {
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before selecting another secret pair',
        );
        if (!canContinue) {
            return;
        }
        setSecretDomain(domain);
        setSecretName(name);
        setSecretValue('');
        setSecretsStatus(
            `${hasValue ? 'Change' : 'Set'} value for ${domain}/${name}.`,
        );
    }

    async function handleScrapeAccountInputChange(nextAccount: string) {
        if (nextAccount === scrapeAccount) {
            return;
        }
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before changing account',
        );
        if (!canContinue) {
            return;
        }
        setScrapeAccount(nextAccount);
        setScrapeStatus(null);
        setSecretsStatus(null);
        setPipelineStatus(null);
    }

    async function handleScrapeExtensionChange(nextExtension: string) {
        if (nextExtension === scrapeExtension) {
            return;
        }
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before changing extension',
        );
        if (!canContinue) {
            return;
        }
        setScrapeExtension(nextExtension);
        setScrapeStatus(null);
        setPipelineStatus(null);
    }

    async function handleCreateLoginConfig() {
        if (!ledger) {
            return;
        }
        const loginName = newLoginName.trim();
        if (loginName.length === 0) {
            setLoginConfigStatus('Login name is required.');
            return;
        }

        setIsSavingLoginConfig(true);
        try {
            await createLogin(ledger.path, loginName, newLoginExtension.trim());
            setNewLoginName('');
            setNewLoginExtension('');
            setSelectedLoginName(loginName);
            setLoginConfigStatus(`Created login '${loginName}'.`);
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to create login: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
    }

    async function handleDeleteSelectedLoginConfig() {
        if (!ledger) {
            return;
        }
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            setLoginConfigStatus('Select a login to delete.');
            return;
        }
        const shouldDelete = await confirmDialog(
            `Delete login '${loginName}'? This fails if it still has documents or journal data.`,
            {
                title: 'Delete login?',
                kind: 'warning',
                okLabel: 'Delete',
                cancelLabel: 'Cancel',
            },
        );
        if (!shouldDelete) {
            setLoginConfigStatus('Delete login canceled.');
            return;
        }

        setIsSavingLoginConfig(true);
        try {
            await deleteLogin(ledger.path, loginName);
            setSelectedLoginName('');
            setLoginConfigStatus(`Deleted login '${loginName}'.`);
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to delete login: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
    }

    async function handleSaveSelectedLoginExtension() {
        if (!ledger) {
            return;
        }
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            setLoginConfigStatus('Select a login first.');
            return;
        }

        setIsSavingLoginConfig(true);
        try {
            await setLoginExtension(
                ledger.path,
                loginName,
                selectedLoginExtensionDraft.trim(),
            );
            setLoginConfigStatus(`Saved extension for '${loginName}'.`);
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to save extension: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
    }

    async function handleSetLoginAccountMapping() {
        if (!ledger) {
            return;
        }
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            setLoginConfigStatus('Select a login first.');
            return;
        }
        const label = loginLabelDraft.trim();
        if (label.length === 0) {
            setLoginConfigStatus('Label is required.');
            return;
        }
        const glAccount = loginGlAccountDraft.trim();

        setIsSavingLoginConfig(true);
        try {
            await setLoginAccount(
                ledger.path,
                loginName,
                label,
                glAccount.length === 0 ? null : glAccount,
            );
            setLoginConfigStatus(
                glAccount.length === 0
                    ? `Set '${loginName}/${label}' as ignored (no GL account).`
                    : `Mapped '${loginName}/${label}' to '${glAccount}'.`,
            );
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to set mapping: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
    }

    async function handleRemoveLoginAccountMapping(label: string) {
        if (!ledger) {
            return;
        }
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            setLoginConfigStatus('Select a login first.');
            return;
        }
        const shouldRemove = await confirmDialog(
            `Remove label '${label}' from login '${loginName}'?`,
            {
                title: 'Remove mapping?',
                kind: 'warning',
                okLabel: 'Remove',
                cancelLabel: 'Cancel',
            },
        );
        if (!shouldRemove) {
            return;
        }

        setIsSavingLoginConfig(true);
        try {
            await removeLoginAccount(ledger.path, loginName, label);
            setLoginConfigStatus(`Removed '${loginName}/${label}'.`);
            requestLoginConfigReload();
        } catch (error) {
            setLoginConfigStatus(`Failed to remove mapping: ${String(error)}`);
        } finally {
            setIsSavingLoginConfig(false);
        }
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

    function handleSubmitSecretForm(
        event: React.SyntheticEvent<HTMLFormElement>,
    ) {
        event.preventDefault();
        const mode: 'add' | 'reenter' = currentSecretPairExists
            ? 'reenter'
            : 'add';
        void handleSaveAccountSecret(mode);
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
        const mappings = loginAccountMappings[account] ?? [];
        if (mappings.length !== 1) {
            throw new Error(
                mappings.length === 0
                    ? `No login mapping found for account '${account}'.`
                    : `Multiple login mappings found for account '${account}'.`,
            );
        }
        const mapping = mappings[0];
        if (mapping === undefined) {
            throw new Error(`No login mapping found for account '${account}'.`);
        }

        setIsLoadingDocuments(true);
        setIsLoadingAccountJournal(true);
        setIsLoadingUnreconciled(true);
        try {
            const [fetchedDocuments, fetchedJournal, fetchedUnreconciled] =
                await Promise.all([
                    listLoginAccountDocuments(
                        ledger.path,
                        mapping.loginName,
                        mapping.label,
                    ),
                    getLoginAccountJournal(
                        ledger.path,
                        mapping.loginName,
                        mapping.label,
                    ),
                    getLoginAccountUnreconciled(
                        ledger.path,
                        mapping.loginName,
                        mapping.label,
                    ),
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
        const mappings = loginAccountMappings[account] ?? [];
        if (mappings.length !== 1) {
            setPipelineStatus(
                mappings.length === 0
                    ? `No login mapping found for account '${account}'.`
                    : `Multiple login mappings found for account '${account}'.`,
            );
            return;
        }
        const mapping = mappings[0];
        if (mapping === undefined) {
            setPipelineStatus(
                `No login mapping found for account '${account}'.`,
            );
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
            const newCount = await runLoginAccountExtraction(
                ledger.path,
                mapping.loginName,
                mapping.label,
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
        const mappings = loginAccountMappings[account] ?? [];
        if (mappings.length !== 1) {
            setPipelineStatus(
                mappings.length === 0
                    ? `No login mapping found for account '${account}'.`
                    : `Multiple login mappings found for account '${account}'.`,
            );
            return;
        }
        const mapping = mappings[0];
        if (mapping === undefined) {
            setPipelineStatus(
                `No login mapping found for account '${account}'.`,
            );
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
            const glId = await reconcileLoginAccountEntry(
                ledger.path,
                mapping.loginName,
                mapping.label,
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
        const mappings = loginAccountMappings[account] ?? [];
        if (mappings.length !== 1) {
            setPipelineStatus(
                mappings.length === 0
                    ? `No login mapping found for account '${account}'.`
                    : `Multiple login mappings found for account '${account}'.`,
            );
            return;
        }
        const mapping = mappings[0];
        if (mapping === undefined) {
            setPipelineStatus(
                `No login mapping found for account '${account}'.`,
            );
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
            await unreconcileLoginAccountEntry(
                ledger.path,
                mapping.loginName,
                mapping.label,
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

    async function handleMigrateLegacyLedger() {
        if (!ledger) {
            return;
        }
        setIsMigratingLegacyLedger(true);
        setScrapeStatus('Migrating legacy accounts layout...');
        try {
            const outcome = await migrateLedger(ledger.path, false);
            const reopened = await openLedger(ledger.path);
            setLedger(reopened);
            setLegacyMigrationPreview(null);
            setScrapeStatus(
                `Migration complete. Migrated ${outcome.migrated.length} account(s).`,
            );
        } catch (error) {
            setScrapeStatus(`Migration failed: ${String(error)}`);
        } finally {
            setIsMigratingLegacyLedger(false);
        }
    }

    async function handleRunScrape() {
        if (!ledger) {
            return;
        }
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus(selectedLoginMappingError ?? 'Login is required.');
            return;
        }
        const account = scrapeAccount.trim();
        const extension = scrapeExtension.trim();
        if (extension.length === 0) {
            setScrapeStatus('Extension is required.');
            return;
        }

        setIsRunningScrape(true);
        setScrapeStatus(`Running ${extension} for ${loginName}...`);
        try {
            await runScrapeForLogin(ledger.path, loginName, extension);
            setScrapeStatus(`Scrape completed for ${extension}.`);
            try {
                if (selectedLoginMapping !== null && account.length > 0) {
                    await refreshAccountPipelineData(account);
                }
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
                    {ledger.glAccountConflicts.length === 0 ? null : (
                        <section className="txn-form">
                            <div className="txn-form-header">
                                <div>
                                    <h2>GL mapping conflicts</h2>
                                    <p>
                                        Multiple login labels map to the same GL
                                        account. Resolve these before extraction
                                        or reconciliation.
                                    </p>
                                </div>
                            </div>
                            <p className="status">
                                {ledger.glAccountConflicts.length} conflicting
                                GL account mapping(s) detected.
                            </p>
                            <div className="table-wrap">
                                <table className="ledger-table">
                                    <thead>
                                        <tr>
                                            <th>GL Account</th>
                                            <th>Login/Label</th>
                                            <th>Action</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {ledger.glAccountConflicts.flatMap(
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
                                            Choose a GL account mapped to a
                                            login and extension, then run the
                                            same scraper pipeline as the CLI
                                            command.
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
                                {isCheckingLegacyMigration ? (
                                    <p className="status">
                                        Checking for legacy account layout...
                                    </p>
                                ) : null}
                                {legacyMigrationPreview === null ? null : (
                                    <section className="pipeline-panel">
                                        <div className="txn-form-header">
                                            <div>
                                                <h3>Migration available</h3>
                                                <p>
                                                    Legacy `accounts/` data is
                                                    present. Migrate to
                                                    login-scoped storage before
                                                    continuing.
                                                </p>
                                            </div>
                                            <div className="header-actions">
                                                <button
                                                    className="ghost-button"
                                                    type="button"
                                                    disabled={
                                                        isMigratingLegacyLedger
                                                    }
                                                    onClick={() => {
                                                        void handleMigrateLegacyLedger();
                                                    }}
                                                >
                                                    {isMigratingLegacyLedger
                                                        ? 'Migrating...'
                                                        : 'Run migration'}
                                                </button>
                                            </div>
                                        </div>
                                        <p className="status">
                                            {
                                                legacyMigrationPreview.migrated
                                                    .length
                                            }{' '}
                                            account(s) ready to migrate.{' '}
                                            {
                                                legacyMigrationPreview.skipped
                                                    .length
                                            }{' '}
                                            account(s) will be skipped.
                                        </p>
                                        {legacyMigrationPreview.warnings
                                            .length > 0 ? (
                                            <p className="status">
                                                Warnings:{' '}
                                                {
                                                    legacyMigrationPreview
                                                        .warnings.length
                                                }
                                                . Run CLI `refreshmint migrate
                                                --dry-run` for details.
                                            </p>
                                        ) : null}
                                    </section>
                                )}
                                <section className="pipeline-panel">
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Login mappings</h3>
                                            <p>
                                                Configure login names, extension
                                                defaults, and label to GL
                                                account mappings.
                                            </p>
                                        </div>
                                        <div className="header-actions">
                                            <button
                                                className="ghost-button"
                                                type="button"
                                                onClick={() => {
                                                    requestLoginConfigReload();
                                                }}
                                                disabled={
                                                    isLoadingLoginConfigs ||
                                                    isSavingLoginConfig
                                                }
                                            >
                                                {isLoadingLoginConfigs
                                                    ? 'Refreshing...'
                                                    : 'Refresh logins'}
                                            </button>
                                        </div>
                                    </div>
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Create login name</span>
                                            <input
                                                type="text"
                                                value={newLoginName}
                                                placeholder="chase-personal"
                                                onChange={(event) => {
                                                    setNewLoginName(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={isSavingLoginConfig}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>Initial extension</span>
                                            <input
                                                type="text"
                                                value={newLoginExtension}
                                                placeholder="optional"
                                                onChange={(event) => {
                                                    setNewLoginExtension(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={isSavingLoginConfig}
                                            />
                                        </label>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleCreateLoginConfig();
                                            }}
                                            disabled={isSavingLoginConfig}
                                        >
                                            {isSavingLoginConfig
                                                ? 'Saving...'
                                                : 'Create login'}
                                        </button>
                                    </div>
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Selected login</span>
                                            <select
                                                value={selectedLoginName}
                                                onChange={(event) => {
                                                    setSelectedLoginName(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={isSavingLoginConfig}
                                            >
                                                <option value="">
                                                    {isLoadingLoginConfigs
                                                        ? 'Loading logins...'
                                                        : 'Select login'}
                                                </option>
                                                {loginNames.map((loginName) => (
                                                    <option
                                                        key={loginName}
                                                        value={loginName}
                                                    >
                                                        {loginName}
                                                    </option>
                                                ))}
                                            </select>
                                        </label>
                                        <label className="field">
                                            <span>Login extension</span>
                                            <input
                                                type="text"
                                                value={
                                                    selectedLoginExtensionDraft
                                                }
                                                placeholder="optional"
                                                onChange={(event) => {
                                                    setSelectedLoginExtensionDraft(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={
                                                    selectedLoginName.length ===
                                                        0 || isSavingLoginConfig
                                                }
                                            />
                                        </label>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleSaveSelectedLoginExtension();
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            {isSavingLoginConfig
                                                ? 'Saving...'
                                                : 'Save login extension'}
                                        </button>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleDeleteSelectedLoginConfig();
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            {isSavingLoginConfig
                                                ? 'Saving...'
                                                : 'Delete login'}
                                        </button>
                                    </div>
                                    {selectedLoginConfig === null ? (
                                        <p className="hint">
                                            Select a login to manage its account
                                            labels.
                                        </p>
                                    ) : selectedLoginAccounts.length === 0 ? (
                                        <p className="hint">
                                            No labels configured for this login.
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Label</th>
                                                        <th>GL Account</th>
                                                        <th>Actions</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {selectedLoginAccounts.map(
                                                        ([label, config]) => (
                                                            <tr key={label}>
                                                                <td>
                                                                    <span className="mono">
                                                                        {label}
                                                                    </span>
                                                                </td>
                                                                <td>
                                                                    {config.glAccount ??
                                                                        '(ignored)'}
                                                                </td>
                                                                <td>
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        onClick={() => {
                                                                            void handleRemoveLoginAccountMapping(
                                                                                label,
                                                                            );
                                                                        }}
                                                                        disabled={
                                                                            isSavingLoginConfig
                                                                        }
                                                                    >
                                                                        Remove
                                                                    </button>
                                                                </td>
                                                            </tr>
                                                        ),
                                                    )}
                                                </tbody>
                                            </table>
                                        </div>
                                    )}
                                    <div className="txn-grid">
                                        <label className="field">
                                            <span>Label</span>
                                            <input
                                                type="text"
                                                value={loginLabelDraft}
                                                placeholder="checking"
                                                onChange={(event) => {
                                                    setLoginLabelDraft(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={
                                                    selectedLoginName.length ===
                                                        0 || isSavingLoginConfig
                                                }
                                            />
                                        </label>
                                        <label className="field">
                                            <span>GL account</span>
                                            <input
                                                type="text"
                                                value={loginGlAccountDraft}
                                                placeholder="Assets:Bank:Checking (blank = ignored)"
                                                list="scrape-account-options"
                                                onChange={(event) => {
                                                    setLoginGlAccountDraft(
                                                        event.target.value,
                                                    );
                                                    setLoginConfigStatus(null);
                                                }}
                                                disabled={
                                                    selectedLoginName.length ===
                                                        0 || isSavingLoginConfig
                                                }
                                            />
                                        </label>
                                    </div>
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                void handleSetLoginAccountMapping();
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            {isSavingLoginConfig
                                                ? 'Saving...'
                                                : 'Set mapping'}
                                        </button>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                setLoginLabelDraft('');
                                                setLoginGlAccountDraft(
                                                    selectedScrapeAccount,
                                                );
                                                setLoginConfigStatus(null);
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            Use selected account
                                        </button>
                                    </div>
                                    {loginConfigStatus === null ? null : (
                                        <p className="status">
                                            {loginConfigStatus}
                                        </p>
                                    )}
                                </section>
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Account</span>
                                        <input
                                            type="text"
                                            value={scrapeAccount}
                                            placeholder="Account name"
                                            list="scrape-account-options"
                                            onChange={(event) => {
                                                void handleScrapeAccountInputChange(
                                                    event.target.value,
                                                );
                                            }}
                                        />
                                    </label>
                                    <label className="field">
                                        <span>Extension</span>
                                        <select
                                            value={scrapeExtension}
                                            onChange={(event) => {
                                                void handleScrapeExtensionChange(
                                                    event.target.value,
                                                );
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
                                {selectedLoginMappingSummary !== null ? (
                                    <p className="hint mono">
                                        Login mapping:{' '}
                                        {selectedLoginMappingSummary}
                                    </p>
                                ) : selectedScrapeAccountHasConflict ? (
                                    <p className="status">
                                        {`Account '${selectedScrapeAccount}' has GL mapping conflicts. Use the conflict panel to load and edit a mapping.`}
                                    </p>
                                ) : hasActiveScrapeLogin ? (
                                    <p className="hint mono">
                                        Using login selection:{' '}
                                        {activeScrapeLoginName}
                                    </p>
                                ) : selectedScrapeAccount.length === 0 ? (
                                    <p className="hint">
                                        Choose a GL account or login to run
                                        scrape/debug.
                                    </p>
                                ) : (
                                    <p className="status">
                                        {selectedLoginMappingError}
                                    </p>
                                )}
                                <div className="txn-actions">
                                    <button
                                        type="button"
                                        className="primary-button"
                                        onClick={() => {
                                            void handleRunScrape();
                                        }}
                                        disabled={
                                            isRunningScrape ||
                                            !hasActiveScrapeLogin ||
                                            isLoadingScrapeExtensions ||
                                            isImportingScrapeExtension ||
                                            isMigratingLegacyLedger ||
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
                                            !hasActiveScrapeLogin ||
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
                                            <h3>Login secrets</h3>
                                            <p>
                                                Manage per-login keychain
                                                secrets for the active login
                                                selection.
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
                                                    !hasActiveSecretsLogin ||
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
                                    <form
                                        className="secret-form"
                                        onSubmit={handleSubmitSecretForm}
                                    >
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
                                                        !hasActiveSecretsLogin ||
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
                                                        !hasActiveSecretsLogin ||
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
                                                    placeholder={
                                                        secretValuePlaceholder
                                                    }
                                                    onChange={(event) => {
                                                        setSecretValue(
                                                            event.target.value,
                                                        );
                                                        setSecretsStatus(null);
                                                    }}
                                                    disabled={
                                                        !hasActiveSecretsLogin ||
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
                                                    !hasActiveSecretsLogin ||
                                                    (trimmedSecretDomain.length >
                                                        0 &&
                                                        trimmedSecretName.length >
                                                            0 &&
                                                        currentSecretPairExists) ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            >
                                                {isSavingAccountSecret
                                                    ? 'Saving...'
                                                    : 'Add new pair'}
                                            </button>
                                            <button
                                                type="submit"
                                                className="ghost-button"
                                                disabled={
                                                    !hasActiveSecretsLogin ||
                                                    !currentSecretPairExists ||
                                                    isSavingAccountSecret ||
                                                    busySecretKey !== null
                                                }
                                            >
                                                {isSavingAccountSecret
                                                    ? 'Saving...'
                                                    : currentSecretHasValue
                                                      ? 'Change value'
                                                      : 'Set value'}
                                            </button>
                                        </div>
                                        <p className="hint">
                                            Add new pair creates a new
                                            domain/name. Press Enter or use Set
                                            or Change value to save the value.
                                        </p>
                                    </form>
                                    {isLoadingAccountSecrets ? (
                                        <p className="status">
                                            Loading login secrets...
                                        </p>
                                    ) : accountSecrets.length === 0 ? (
                                        <p className="hint">
                                            {hasActiveSecretsLogin
                                                ? 'No secrets stored for this login.'
                                                : selectedScrapeAccount.length >
                                                        0 &&
                                                    selectedLoginMappingError !==
                                                        null
                                                  ? 'Resolve login mapping first, or select a login in Login mappings.'
                                                  : 'Select a login mapping or login to manage secrets.'}
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
                                                            const key =
                                                                secretPairKey(
                                                                    entry.domain,
                                                                    entry.name,
                                                                );
                                                            const isBusy =
                                                                busySecretKey ===
                                                                key;
                                                            const isExtra =
                                                                hasRequiredSecretsSync &&
                                                                !requiredSecretKeySet.has(
                                                                    key,
                                                                );
                                                            return (
                                                                <tr key={key}>
                                                                    <td>
                                                                        {
                                                                            entry.domain
                                                                        }
                                                                    </td>
                                                                    <td>
                                                                        <span>
                                                                            {
                                                                                entry.name
                                                                            }
                                                                        </span>
                                                                        {isExtra ? (
                                                                            <span className="secret-chip">
                                                                                extra
                                                                            </span>
                                                                        ) : null}
                                                                    </td>
                                                                    <td>
                                                                        <div className="txn-actions">
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    void handleReenterPreset(
                                                                                        entry.domain,
                                                                                        entry.name,
                                                                                        entry.hasValue,
                                                                                    );
                                                                                }}
                                                                                disabled={
                                                                                    isSavingAccountSecret ||
                                                                                    busySecretKey !==
                                                                                        null
                                                                                }
                                                                            >
                                                                                {entry.hasValue
                                                                                    ? 'Change value'
                                                                                    : 'Set value'}
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
                                    {hasRequiredSecretsSync &&
                                    extraSecretCount > 0 ? (
                                        <p className="hint">
                                            {extraSecretCount} secret
                                            {extraSecretCount === 1
                                                ? ''
                                                : 's'}{' '}
                                            stored for this login are not
                                            declared by the selected extension.
                                        </p>
                                    ) : null}
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
                                                    !hasResolvedLoginMapping ||
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
                                                !hasResolvedLoginMapping ||
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
                                            {selectedScrapeAccount.length === 0
                                                ? 'Choose a GL account to view documents.'
                                                : !hasResolvedLoginMapping
                                                  ? 'Resolve login mapping first to view documents.'
                                                  : 'No documents found for this login account mapping.'}
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
                                            {selectedScrapeAccount.length === 0
                                                ? 'Choose a GL account to view its journal.'
                                                : !hasResolvedLoginMapping
                                                  ? 'Resolve login mapping first to view journal entries.'
                                                  : 'No account journal entries found for this login mapping.'}
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
                                            {selectedScrapeAccount.length === 0
                                                ? 'Choose a GL account to view unreconciled entries.'
                                                : !hasResolvedLoginMapping
                                                  ? 'Resolve login mapping first to view unreconciled entries.'
                                                  : 'No unreconciled entries for this login mapping.'}
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
