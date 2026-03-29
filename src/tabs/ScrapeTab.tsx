import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import {
    confirm as confirmDialog,
    open as openDialog,
} from '@tauri-apps/plugin-dialog';
import {
    type AccountJournalEntry,
    type DomainSecretEntry,
    type DocumentWithInfo,
    type LedgerView,
    type LoginConfig,
    type MigrationOutcome,
    createLogin,
    deleteLogin,
    deleteLoginAccount,
    getLoginAccountJournal,
    getLoginAccountUnposted,
    getLoginConfig,
    getLoginUsername,
    getScrapeDebugSessionSocket,
    listLoginAccountDocuments,
    listLoginSecrets,
    listScrapeExtensions,
    loadScrapeExtension,
    migrateLedger,
    migrateLoginSecrets,
    postLoginAccountEntry,
    postTransfer,
    removeLoginDomain,
    repairLoginAccountLabels,
    runLoginAccountExtraction,
    runScrapeForLogin,
    setLoginAccount,
    setLoginCredentials,
    setLoginExtension,
    setLoginPassword,
    setLoginUsername,
    startScrapeDebugSessionForLogin,
    stopScrapeDebugSession,
    syncLoginSecretsForExtension,
    unpostLoginAccountEntry,
} from '../tauri-commands.ts';
import {
    type LoginAccountMapping,
    type PostDraft,
    type SecretPromptState,
    type TransferDraft,
    normalizeLoginConfig,
} from '../types.ts';
import {
    appendScrapeLog,
    readScrapeLog,
    type ScrapeLogEntry,
} from '../scrapeLog.ts';

interface ScrapeTabProps {
    ledger: LedgerView | null;
    // Shared login config data (loaded by App for the global conflicts panel)
    loginNames: string[];
    loginConfigsByName: Record<string, LoginConfig>;
    loginAccountMappings: Record<string, LoginAccountMapping[]>;
    isLoadingLoginConfigs: boolean;
    conflictingGlAccountSet: Set<string>;
    // Login selection state lifted to App (needed by handleIgnoreLoginAccountMapping /
    // handleLoadConflictMapping which run from the global conflicts panel in App)
    selectedLoginName: string;
    onSelectedLoginNameChange: (name: string) => void;
    loginManagementTab: 'select' | 'create';
    onLoginManagementTabChange: (tab: 'select' | 'create') => void;
    editingMappingLabel: string | null;
    onEditingMappingLabelChange: (label: string | null) => void;
    editingMappingGlAccountDraft: string;
    onEditingMappingGlAccountDraftChange: (account: string) => void;
    loginConfigStatus: string | null;
    onLoginConfigStatusChange: (status: string | null) => void;
    isSavingLoginConfig: boolean;
    onIsSavingLoginConfigChange: (saving: boolean) => void;
    // Callbacks
    onLoginConfigChanged: () => void;
    onLedgerRefresh: () => void;
    onSecretPrompt: (p: SecretPromptState) => Promise<boolean>;
    onIgnoreLoginAccountMapping: (
        loginName: string,
        label: string,
        glAccount: string,
    ) => Promise<void>;
    scrapeLogVersion: number;
    onScrapeComplete: (loginName: string) => Promise<void>;
    onScrapeAll: () => void;
    autoScrapeActive: string | null;
}

function secretDomainKey(domain: string): string {
    return domain;
}

export function ScrapeTab({
    ledger,
    loginNames,
    loginConfigsByName,
    isLoadingLoginConfigs,
    conflictingGlAccountSet,
    selectedLoginName,
    onSelectedLoginNameChange,
    loginManagementTab,
    onLoginManagementTabChange,
    editingMappingLabel,
    onEditingMappingLabelChange,
    editingMappingGlAccountDraft,
    onEditingMappingGlAccountDraftChange,
    loginConfigStatus,
    onLoginConfigStatusChange,
    isSavingLoginConfig,
    onIsSavingLoginConfigChange,
    onLoginConfigChanged,
    onLedgerRefresh,
    onSecretPrompt,
    onIgnoreLoginAccountMapping,
    scrapeLogVersion,
    onScrapeComplete,
    onScrapeAll,
    autoScrapeActive,
}: ScrapeTabProps) {
    const [selectedPipelineLabel, setSelectedPipelineLabel] = useState<
        string | null
    >(null);
    const [scrapeExtension, setScrapeExtension] = useState('');
    const [scrapeExtensions, setScrapeExtensions] = useState<string[]>([]);
    const [scrapeStatus, setScrapeStatus] = useState<string | null>(null);
    const [scrapeLogEntries, setScrapeLogEntries] = useState<ScrapeLogEntry[]>(
        [],
    );
    const [scrapeDebugSocket, setScrapeDebugSocket] = useState<string | null>(
        null,
    );
    const [legacyMigrationPreview, setLegacyMigrationPreview] =
        useState<MigrationOutcome | null>(null);
    const [isCheckingLegacyMigration, setIsCheckingLegacyMigration] =
        useState(false);
    const [isMigratingLegacyLedger, setIsMigratingLegacyLedger] =
        useState(false);
    const [selectedLoginExtensionDraft, setSelectedLoginExtensionDraft] =
        useState('');
    const [newLoginName, setNewLoginName] = useState('');
    const [newLoginExtension, setNewLoginExtension] = useState('');
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isImportingScrapeExtension, setIsImportingScrapeExtension] =
        useState(false);
    const [extensionLoadStatus, setExtensionLoadStatus] = useState<
        string | null
    >(null);
    const [isStartingScrapeDebug, setIsStartingScrapeDebug] = useState(false);
    const [isStoppingScrapeDebug, setIsStoppingScrapeDebug] = useState(false);
    const [accountSecrets, setAccountSecrets] = useState<DomainSecretEntry[]>(
        [],
    );
    const [requiredSecretsForExtension, setRequiredSecretsForExtension] =
        useState<DomainSecretEntry[]>([]);
    const [hasRequiredSecretsSync, setHasRequiredSecretsSync] = useState(false);
    const [secretDomain, setSecretDomain] = useState('');
    const [secretUsername, setSecretUsername] = useState('');
    const [secretPassword, setSecretPassword] = useState('');
    const [isSecretsPanelExpanded, setIsSecretsPanelExpanded] = useState(false);
    const [secretsStatus, setSecretsStatus] = useState<string | null>(null);
    const [isLoadingAccountSecrets, setIsLoadingAccountSecrets] =
        useState(false);
    const [isSavingAccountSecret, setIsSavingAccountSecret] = useState(false);
    const [busySecretKey, setBusySecretKey] = useState<string | null>(null);
    const [isRunningScrape, setIsRunningScrape] = useState(false);
    const [documents, setDocuments] = useState<DocumentWithInfo[]>([]);
    const [selectedDocumentNames, setSelectedDocumentNames] = useState<
        string[]
    >([]);
    const [unpostedEntries, setUnpostedEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [pipelineStatus, setPipelineStatus] = useState<string | null>(null);
    const [isLoadingDocuments, setIsLoadingDocuments] = useState(false);
    const [isRunningExtraction, setIsRunningExtraction] = useState(false);
    const [isLoadingAccountJournal, setIsLoadingAccountJournal] =
        useState(false);
    const [isLoadingUnposted, setIsLoadingUnposted] = useState(false);
    const [postDrafts, setPostDrafts] = useState<Record<string, PostDraft>>({});
    const [busyPostEntryId, setBusyPostEntryId] = useState<string | null>(null);
    const [unpostEntryId, setUnpostEntryId] = useState('');
    const [unpostPostingIndex, setUnpostPostingIndex] = useState('');
    const [isUnpostingEntry, setIsUnpostingEntry] = useState(false);
    const [transferDraft, setTransferDraft] = useState<TransferDraft>({
        account1: '',
        entryId1: '',
        account2: '',
        entryId2: '',
    });
    const [isPostingTransfer, setIsPostingTransfer] = useState(false);

    const secretDomainRef = useRef('');
    const ledgerPath = ledger?.path ?? null;

    // ─── Computed values ────────────────────────────────────────────────────────

    const scrapeAccountOptions = ledger
        ? ledger.accounts
              .map((account) => account.name.trim())
              .filter(
                  (name, index, names) =>
                      name.length > 0 && names.indexOf(name) === index,
              )
        : [];
    const activeScrapeLoginName = selectedLoginName.trim() || null;
    const hasActiveScrapeLogin = activeScrapeLoginName !== null;
    const activeSecretsLoginName = activeScrapeLoginName;
    const hasActiveSecretsLogin = activeSecretsLoginName !== null;

    const selectedLoginConfig: LoginConfig | null =
        selectedLoginName.length === 0
            ? null
            : (loginConfigsByName[selectedLoginName] ?? null);
    const selectedLoginAccounts = useMemo(
        () =>
            selectedLoginConfig === null
                ? []
                : Object.entries(
                      normalizeLoginConfig(selectedLoginConfig).accounts,
                  ).sort(([a], [b]) => a.localeCompare(b)),
        [selectedLoginConfig],
    );
    // Labels for the selected login that have a GL account mapping.
    const selectedLoginMappedLabels = selectedLoginAccounts
        .filter(([, cfg]) => (cfg.glAccount?.trim() ?? '').length > 0)
        .map(([label]) => label);
    const hasResolvedLoginMapping =
        selectedPipelineLabel !== null &&
        selectedLoginMappedLabels.includes(selectedPipelineLabel);
    // The GL account for the pipeline-selected label (used by transfer form and
    // post-scrape refresh).
    const selectedPipelineGlAccount =
        selectedPipelineLabel !== null
            ? (selectedLoginAccounts
                  .find(([l]) => l === selectedPipelineLabel)?.[1]
                  ?.glAccount?.trim() ?? '')
            : '';

    const selectedLoginConflictCount = selectedLoginAccounts.reduce(
        (count, [, config]) => {
            const glAccount = config.glAccount?.trim() ?? '';
            return glAccount.length > 0 &&
                conflictingGlAccountSet.has(glAccount)
                ? count + 1
                : count;
        },
        0,
    );

    const requiredSecretDomainSet = new Set(
        requiredSecretsForExtension.map((entry) =>
            secretDomainKey(entry.domain),
        ),
    );
    const trimmedSecretDomain = secretDomain.trim();
    const currentSecretEntry = accountSecrets.find(
        (entry) => entry.domain === trimmedSecretDomain,
    );
    const currentDomainExists = currentSecretEntry !== undefined;
    const extraSecretCount = hasRequiredSecretsSync
        ? accountSecrets.reduce((count, entry) => {
              const key = secretDomainKey(entry.domain);
              return requiredSecretDomainSet.has(key) ? count : count + 1;
          }, 0)
        : 0;

    // ─── Effects ────────────────────────────────────────────────────────────────

    useEffect(() => {
        secretDomainRef.current = secretDomain;
    }, [secretDomain]);

    // Reset all own state when the ledger path changes.
    useEffect(() => {
        setScrapeStatus(null);
        setScrapeDebugSocket(null);
        setSelectedPipelineLabel(null);
        setSelectedLoginExtensionDraft('');
        setNewLoginName('');
        setNewLoginExtension('');
        setAccountSecrets([]);
        setRequiredSecretsForExtension([]);
        setHasRequiredSecretsSync(false);
        setSecretDomain('');
        setSecretUsername('');
        setSecretPassword('');
        setIsSecretsPanelExpanded(false);
        setSecretsStatus(null);
        setIsLoadingAccountSecrets(false);
        setIsSavingAccountSecret(false);
        setBusySecretKey(null);
        setDocuments([]);
        setSelectedDocumentNames([]);
        setUnpostedEntries([]);
        setPipelineStatus(null);
        setIsLoadingDocuments(false);
        setIsRunningExtraction(false);
        setIsLoadingAccountJournal(false);
        setIsLoadingUnposted(false);
        setPostDrafts({});
        setBusyPostEntryId(null);
        setUnpostEntryId('');
        setUnpostPostingIndex('');
        setIsUnpostingEntry(false);
        setTransferDraft({
            account1: '',
            entryId1: '',
            account2: '',
            entryId2: '',
        });
        setIsPostingTransfer(false);
        setScrapeLogEntries([]);
    }, [ledgerPath]);

    // Reload scrape log when selected login or scrapeLogVersion changes.
    useEffect(() => {
        if (activeScrapeLoginName === null) {
            setScrapeLogEntries([]);
            return;
        }
        setScrapeLogEntries(readScrapeLog(activeScrapeLoginName));
    }, [activeScrapeLoginName, scrapeLogVersion]);

    // List scrape extensions when ledger changes.
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
                if (cancelled) return;
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

    // Check for legacy ledger migration when ledger changes.
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
                if (cancelled) return;
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
                if (!cancelled) setLegacyMigrationPreview(null);
            })
            .finally(() => {
                if (!cancelled) setIsCheckingLegacyMigration(false);
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    // Fetch the active debug session socket when ledger changes.
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeDebugSocket(null);
            return;
        }

        let cancelled = false;
        void getScrapeDebugSessionSocket()
            .then((socket) => {
                if (!cancelled) setScrapeDebugSocket(socket);
            })
            .catch(() => {
                if (!cancelled) setScrapeDebugSocket(null);
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    // Auto-populate extension draft when selected login changes.
    useEffect(() => {
        const extension = selectedLoginConfig?.extension?.trim() ?? '';
        setSelectedLoginExtensionDraft(extension);
    }, [selectedLoginConfig]);

    // Auto-select the pipeline label when it can be unambiguously determined.
    useEffect(() => {
        if (selectedLoginMappedLabels.length === 1) {
            setSelectedPipelineLabel(selectedLoginMappedLabels[0] ?? null);
        } else {
            // Clear if the previously selected label is no longer valid.
            setSelectedPipelineLabel((current) =>
                current !== null && selectedLoginMappedLabels.includes(current)
                    ? current
                    : null,
            );
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [selectedLoginName, selectedLoginMappedLabels.join(',')]);

    // Load account secrets when login or secrets panel state changes.
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
        if (!isSecretsPanelExpanded) {
            setIsLoadingAccountSecrets(false);
            return;
        }

        const loginName = activeSecretsLoginName;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingAccountSecrets(true);
            void listLoginSecrets(loginName)
                .then((entries) => {
                    if (!cancelled) setAccountSecrets(entries);
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
                    if (!cancelled) setIsLoadingAccountSecrets(false);
                });
        }, 250);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [activeSecretsLoginName, isSecretsPanelExpanded, ledgerPath]);

    // Sync login secrets for the selected extension.
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
        if (!isSecretsPanelExpanded) {
            setIsLoadingAccountSecrets(false);
            return;
        }

        const loginName = activeSecretsLoginName;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingAccountSecrets(true);
            void syncLoginSecretsForExtension(ledgerPath, loginName, extension)
                .then((result) => {
                    if (cancelled) return;
                    setRequiredSecretsForExtension(result.required);
                    setHasRequiredSecretsSync(true);

                    const currentDomain = secretDomainRef.current.trim();
                    const requiredDomainSet = new Set(
                        result.required.map((entry) =>
                            secretDomainKey(entry.domain),
                        ),
                    );

                    if (
                        currentDomain.length > 0 &&
                        !requiredDomainSet.has(currentDomain)
                    ) {
                        setSecretDomain('');
                        setSecretUsername('');
                        setSecretPassword('');
                    } else if (
                        currentDomain.length === 0 &&
                        result.required.length > 0
                    ) {
                        const first = result.required[0];
                        if (first !== undefined) {
                            setSecretDomain(first.domain);
                        }
                    }

                    const requiredCount = result.required.length;
                    const missingCount =
                        result.missingUsername.length +
                        result.missingPassword.length;
                    const extraCount = result.extras.length;
                    if (requiredCount === 0) {
                        setSecretsStatus(
                            'No declared secrets for this extension.',
                        );
                    } else {
                        const extraSuffix =
                            extraCount > 0
                                ? ` ${extraCount} extra domain${extraCount === 1 ? '' : 's'} found.`
                                : '';
                        const missingSuffix =
                            missingCount > 0
                                ? ` ${missingCount} credential${missingCount === 1 ? '' : 's'} missing.`
                                : '';
                        setSecretsStatus(
                            `${requiredCount} required domain${requiredCount === 1 ? '' : 's'}.${missingSuffix}${extraSuffix}`,
                        );
                    }

                    return listLoginSecrets(loginName)
                        .then((entries) => {
                            if (!cancelled) setAccountSecrets(entries);
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
                    if (!cancelled) setIsLoadingAccountSecrets(false);
                });
        }, 200);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [
        activeSecretsLoginName,
        isSecretsPanelExpanded,
        ledgerPath,
        scrapeExtension,
    ]);

    // Load extension name from login config when the active scrape login changes.
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
                    if (cancelled) return;
                    const normalizedConfig = normalizeLoginConfig(config);
                    setScrapeExtension(
                        normalizedConfig.extension?.trim() ?? '',
                    );
                })
                .catch(() => {
                    if (!cancelled) setScrapeExtension('');
                });
        }, 100);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [activeScrapeLoginName, ledgerPath]);

    // Load documents/journal/unposted for the currently selected scrape account mapping.
    useEffect(() => {
        if (ledgerPath === null) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setUnpostedEntries([]);
            setPostDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
            return;
        }

        const loginName = selectedLoginName.trim();
        const label = selectedPipelineLabel;
        if (loginName.length === 0 || label === null) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setUnpostedEntries([]);
            setPostDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
            return;
        }

        const glAccount =
            selectedLoginAccounts
                .find(([l]) => l === label)?.[1]
                ?.glAccount?.trim() ?? '';
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingDocuments(true);
            setIsLoadingAccountJournal(true);
            setIsLoadingUnposted(true);
            void Promise.all([
                listLoginAccountDocuments(ledgerPath, loginName, label),
                getLoginAccountJournal(ledgerPath, loginName, label),
                getLoginAccountUnposted(ledgerPath, loginName, label),
            ])
                .then(
                    ([fetchedDocuments, _fetchedJournal, fetchedUnposted]) => {
                        if (cancelled) return;
                        setDocuments(fetchedDocuments);
                        setSelectedDocumentNames((current) =>
                            current.filter((name) =>
                                fetchedDocuments.some(
                                    (doc) => doc.filename === name,
                                ),
                            ),
                        );
                        setUnpostedEntries(fetchedUnposted);
                        setPostDrafts((current) => {
                            const next: Record<string, PostDraft> = {};
                            for (const entry of fetchedUnposted) {
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
                                    : glAccount,
                        }));
                    },
                )
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setDocuments([]);
                        setSelectedDocumentNames([]);
                        setUnpostedEntries([]);
                        setPostDrafts({});
                        setPipelineStatus(
                            `Failed to load login pipeline data: ${String(error)}`,
                        );
                    }
                })
                .finally(() => {
                    if (!cancelled) {
                        setIsLoadingDocuments(false);
                        setIsLoadingAccountJournal(false);
                        setIsLoadingUnposted(false);
                    }
                });
        }, 250);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [
        ledgerPath,
        selectedLoginName,
        selectedPipelineLabel,
        selectedLoginAccounts,
    ]);

    // ─── Handlers ───────────────────────────────────────────────────────────────

    const reloadScrapeExtensions = useCallback(
        async (path: string, preferredExtension: string) => {
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
        },
        [],
    );

    async function handleLoadScrapeExtension(sourceType: 'zip' | 'directory') {
        if (!ledger) return;

        const source: string | null = await openDialog({
            directory: sourceType === 'directory',
            multiple: false,
            title:
                sourceType === 'directory'
                    ? 'Load extension from directory'
                    : 'Load extension from zip',
            ...(sourceType === 'zip'
                ? { filters: [{ name: 'ZIP archive', extensions: ['zip'] }] }
                : {}),
        });
        if (source === null) return;
        if (source.length === 0) {
            setScrapeStatus('Extension load canceled.');
            setExtensionLoadStatus('Extension load canceled.');
            return;
        }

        setIsImportingScrapeExtension(true);
        setScrapeStatus('Loading extension...');
        setExtensionLoadStatus('Loading extension...');
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
                    setExtensionLoadStatus('Extension load canceled.');
                    return;
                }

                loadedExtensionName = await loadScrapeExtension(
                    ledger.path,
                    source,
                    true,
                );
            }

            await reloadScrapeExtensions(ledger.path, loadedExtensionName);
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
            setExtensionLoadStatus(
                `Loaded extension '${loadedExtensionName}'.`,
            );
        } catch (error) {
            setScrapeStatus(`Failed to load extension: ${String(error)}`);
            setExtensionLoadStatus(
                `Failed to load extension: ${String(error)}`,
            );
        } finally {
            setIsImportingScrapeExtension(false);
        }
    }

    async function handleLoadUnpackedExtension() {
        if (!ledger) return;

        const source: string | null = await openDialog({
            directory: true,
            multiple: false,
            title: 'Load unpacked extension directory',
        });
        if (source === null) return;
        if (source.length === 0) {
            setScrapeStatus('Extension load canceled.');
            setExtensionLoadStatus('Extension load canceled.');
            return;
        }

        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Select a login first.');
            setExtensionLoadStatus('Select a login first.');
            return;
        }

        try {
            await setLoginExtension(ledger.path, loginName, source);
            setScrapeExtension(source);
            setScrapeStatus(`Set unpacked extension: ${source}`);
            setExtensionLoadStatus(`Set unpacked extension: ${source}`);
        } catch (error) {
            setScrapeStatus(
                `Failed to set unpacked extension: ${String(error)}`,
            );
            setExtensionLoadStatus(
                `Failed to set unpacked extension: ${String(error)}`,
            );
        }
    }

    async function handleStartScrapeDebug() {
        if (!ledger) return;
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Select a login first.');
            return;
        }
        setIsStartingScrapeDebug(true);
        setScrapeStatus('Starting debug session...');
        try {
            const socket = await startScrapeDebugSessionForLogin(
                ledger.path,
                loginName,
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
        if (scrapeDebugSocket === null) return;
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

    async function confirmSaveOrDiscardSecretValue(
        context: string,
    ): Promise<boolean> {
        if (secretPassword.length === 0) return true;

        const shouldSave = await onSecretPrompt({
            title: 'Unsaved password',
            message: `You have an unsaved password ${context}. Save it first?`,
            confirmLabel: 'Save',
            cancelLabel: 'Discard',
        });
        if (!shouldSave) {
            setSecretPassword('');
            setSecretsStatus('Discarded unsaved password.');
            return true;
        }

        const saved = await handleSaveDomainCredentials();
        if (saved) return true;
        const shouldDiscardAfterFailedSave = await onSecretPrompt({
            title: 'Save failed',
            message: `Could not save the password ${context}. Discard it and continue?`,
            confirmLabel: 'Discard',
            cancelLabel: 'Keep editing',
        });

        if (shouldDiscardAfterFailedSave) {
            setSecretPassword('');
            setSecretsStatus('Discarded unsaved password.');
            return true;
        }
        return false;
    }

    async function handleRefreshAccountSecrets() {
        if (activeSecretsLoginName === null) {
            setSecretsStatus('Select a login first.');
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

    async function handleSaveDomainCredentials(): Promise<boolean> {
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSecretsStatus('Select a login first.');
            return false;
        }
        const domain = secretDomain.trim();
        if (domain.length === 0) {
            setSecretsStatus('Domain is required.');
            return false;
        }
        const username = secretUsername.trim();
        const password = secretPassword;

        if (username.length === 0 && password.length === 0) {
            setSecretsStatus('Username or password is required.');
            return false;
        }

        setIsSavingAccountSecret(true);
        try {
            if (username.length > 0 && password.length > 0) {
                await setLoginCredentials(
                    loginName,
                    domain,
                    username,
                    password,
                );
            } else if (username.length > 0) {
                await setLoginUsername(loginName, domain, username);
            } else {
                await setLoginPassword(loginName, domain, password);
            }
            await refreshLoginSecrets(loginName);
            setSecretPassword('');
            const isNew = currentDomainExists;
            setSecretsStatus(
                isNew
                    ? `Credentials updated for ${domain}.`
                    : `Credentials saved for ${domain}.`,
            );
            return true;
        } catch (error) {
            setSecretsStatus(`Failed to save credentials: ${String(error)}`);
            return false;
        } finally {
            setIsSavingAccountSecret(false);
        }
    }

    async function handleRemoveDomainSecret(domain: string) {
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSecretsStatus('Select a login first.');
            return;
        }
        setBusySecretKey(domain);
        try {
            await removeLoginDomain(loginName, domain);
            await refreshLoginSecrets(loginName);
            if (secretDomain === domain) {
                setSecretUsername('');
                setSecretPassword('');
            }
            setSecretsStatus(`Removed credentials for ${domain}.`);
        } catch (error) {
            setSecretsStatus(`Failed to remove ${domain}: ${String(error)}`);
        } finally {
            setBusySecretKey(null);
        }
    }

    async function handleEditDomainPreset(domain: string) {
        const canContinue = await confirmSaveOrDiscardSecretValue(
            'before selecting another domain',
        );
        if (!canContinue) return;
        setSecretDomain(domain);
        setSecretPassword('');
        setSecretsStatus(`Edit credentials for ${domain}.`);

        const loginName = activeSecretsLoginName;
        if (loginName !== null) {
            try {
                const username = await getLoginUsername(loginName, domain);
                setSecretUsername(username);
            } catch {
                setSecretUsername('');
            }
        }
    }

    async function handleMigrateLoginSecrets() {
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSecretsStatus('Select a login first.');
            return;
        }
        setIsSavingAccountSecret(true);
        try {
            const migrated = await migrateLoginSecrets(loginName);
            await refreshLoginSecrets(loginName);
            if (migrated.length === 0) {
                setSecretsStatus('No legacy credentials to migrate.');
            } else {
                setSecretsStatus(
                    `Migrated ${migrated.length} domain${migrated.length === 1 ? '' : 's'}: ${migrated.join(', ')}.`,
                );
            }
        } catch (error) {
            setSecretsStatus(`Migration failed: ${String(error)}`);
        } finally {
            setIsSavingAccountSecret(false);
        }
    }

    async function handleCreateLoginConfig() {
        if (!ledger) return;
        const loginName = newLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Login name is required.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await createLogin(ledger.path, loginName, newLoginExtension.trim());
            setNewLoginName('');
            setNewLoginExtension('');
            onSelectedLoginNameChange(loginName);
            onLoginManagementTabChange('select');
            onLoginConfigStatusChange(`Created login '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to create login: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleDeleteSelectedLoginConfig() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login to delete.');
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
            onLoginConfigStatusChange('Delete login canceled.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await deleteLogin(ledger.path, loginName);
            onSelectedLoginNameChange('');
            onLoginConfigStatusChange(`Deleted login '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to delete login: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleSaveSelectedLoginExtension() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await setLoginExtension(
                ledger.path,
                loginName,
                selectedLoginExtensionDraft.trim(),
            );
            onLoginConfigStatusChange(`Saved extension for '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to save extension: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleSetLoginAccountMapping() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }
        const label = (editingMappingLabel ?? '').trim();
        if (label.length === 0) {
            onLoginConfigStatusChange('Label is required.');
            return;
        }
        const glAccount = editingMappingGlAccountDraft.trim();

        onIsSavingLoginConfigChange(true);
        try {
            await setLoginAccount(
                ledger.path,
                loginName,
                label,
                glAccount.length === 0 ? null : glAccount,
            );
            onLoginConfigStatusChange(
                glAccount.length === 0
                    ? `Set '${loginName}/${label}' as ignored (no GL account).`
                    : `Mapped '${loginName}/${label}' to '${glAccount}'.`,
            );
            onEditingMappingLabelChange(null);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to set mapping: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleRemoveLoginAccountMapping(label: string) {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
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
        if (!shouldRemove) return;

        onIsSavingLoginConfigChange(true);
        try {
            await deleteLoginAccount(ledger.path, loginName, label);
            onLoginConfigStatusChange(`Removed '${loginName}/${label}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to remove mapping: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleRepairSelectedLoginLabels() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            const outcome = await repairLoginAccountLabels(
                ledger.path,
                loginName,
            );
            const migratedCount = outcome.migrated.length;
            const skippedCount = outcome.skipped.length;
            const warningSummary =
                outcome.warnings.length === 0
                    ? ''
                    : ` Warnings: ${outcome.warnings.join(' ')}`;
            onLoginConfigStatusChange(
                `Repaired ${migratedCount} label${migratedCount === 1 ? '' : 's'} for '${loginName}'. Skipped ${skippedCount}.${warningSummary}`,
            );
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to repair labels: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    function handleSubmitSecretForm(event: SyntheticEvent<HTMLFormElement>) {
        event.preventDefault();
        void handleSaveDomainCredentials();
    }

    function parseOptionalIndex(raw: string): {
        value: number | null;
        error: string | null;
    } {
        const trimmed = raw.trim();
        if (trimmed.length === 0) return { value: null, error: null };
        const parsed = Number.parseInt(trimmed, 10);
        if (!Number.isFinite(parsed) || Number.isNaN(parsed) || parsed < 0) {
            return {
                value: null,
                error: 'Posting index must be a non-negative integer.',
            };
        }
        return { value: parsed, error: null };
    }

    async function refreshAccountPipelineData(
        loginName: string,
        label: string,
        glAccount = '',
    ) {
        if (!ledger) return;

        setIsLoadingDocuments(true);
        setIsLoadingAccountJournal(true);
        setIsLoadingUnposted(true);
        try {
            const [fetchedDocuments, _fetchedJournal, fetchedUnposted] =
                await Promise.all([
                    listLoginAccountDocuments(ledger.path, loginName, label),
                    getLoginAccountJournal(ledger.path, loginName, label),
                    getLoginAccountUnposted(ledger.path, loginName, label),
                ]);
            setDocuments(fetchedDocuments);
            setSelectedDocumentNames((current) =>
                current.filter((name) =>
                    fetchedDocuments.some((doc) => doc.filename === name),
                ),
            );
            setUnpostedEntries(fetchedUnposted);
            setPostDrafts((current) => {
                const next: Record<string, PostDraft> = {};
                for (const entry of fetchedUnposted) {
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
                        : glAccount,
            }));
        } finally {
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
        }
    }

    async function handleRefreshAccountPipelineData() {
        const loginName = selectedLoginName.trim();
        const label = selectedPipelineLabel;
        if (loginName.length === 0 || label === null) {
            setPipelineStatus('Select a login and label first.');
            return;
        }
        try {
            await refreshAccountPipelineData(
                loginName,
                label,
                selectedPipelineGlAccount,
            );
            setPipelineStatus(
                `Loaded documents and journals for ${loginName}/${label}.`,
            );
        } catch (error) {
            setPipelineStatus(
                `Failed to refresh account pipeline data: ${String(error)}`,
            );
        }
    }

    function handleToggleDocumentSelection(filename: string, checked: boolean) {
        setSelectedDocumentNames((current) => {
            if (checked) {
                if (current.includes(filename)) return current;
                return [...current, filename];
            }
            return current.filter((name) => name !== filename);
        });
        setPipelineStatus(null);
    }

    async function handleRunExtraction() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        const label = selectedPipelineLabel;
        if (loginName.length === 0 || label === null) {
            setPipelineStatus('Select a login and label first.');
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
                loginName,
                label,
                documentNames,
            );
            await refreshAccountPipelineData(
                loginName,
                label,
                selectedPipelineGlAccount,
            );
            setPipelineStatus(
                `Extraction complete. Added ${newCount} new transaction(s).`,
            );
        } catch (error) {
            setPipelineStatus(`Extraction failed: ${String(error)}`);
        } finally {
            setIsRunningExtraction(false);
        }
    }

    function handleSetPostDraft(entryId: string, patch: Partial<PostDraft>) {
        setPostDrafts((current) => ({
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

    async function handlePostAccountEntry(entryId: string) {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        const label = selectedPipelineLabel;
        if (loginName.length === 0 || label === null) {
            setPipelineStatus('Select a login and label first.');
            return;
        }
        const draft = postDrafts[entryId] ?? {
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

        setBusyPostEntryId(entryId);
        try {
            const glId = await postLoginAccountEntry(
                ledger.path,
                loginName,
                label,
                entryId,
                counterpartAccount,
                postingIndex.value,
            );
            await refreshAccountPipelineData(
                loginName,
                label,
                selectedPipelineGlAccount,
            );
            setUnpostEntryId(entryId);
            setPipelineStatus(`Posted ${entryId} to ${glId}.`);
            onLedgerRefresh();
        } catch (error) {
            setPipelineStatus(`Post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handleUnpostAccountEntry() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        const label = selectedPipelineLabel;
        if (loginName.length === 0 || label === null) {
            setPipelineStatus('Select a login and label first.');
            return;
        }
        const entryId = unpostEntryId.trim();
        if (entryId.length === 0) {
            setPipelineStatus('Entry ID is required for unpost.');
            return;
        }

        const postingIndex = parseOptionalIndex(unpostPostingIndex);
        if (postingIndex.error !== null) {
            setPipelineStatus(postingIndex.error);
            return;
        }

        setIsUnpostingEntry(true);
        try {
            await unpostLoginAccountEntry(
                ledger.path,
                loginName,
                label,
                entryId,
                postingIndex.value,
            );
            await refreshAccountPipelineData(
                loginName,
                label,
                selectedPipelineGlAccount,
            );
            setPipelineStatus(`Unposted ${entryId}.`);
            onLedgerRefresh();
        } catch (error) {
            setPipelineStatus(`Unpost failed: ${String(error)}`);
        } finally {
            setIsUnpostingEntry(false);
        }
    }

    async function handlePostTransferPair() {
        if (!ledger) return;
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

        setIsPostingTransfer(true);
        try {
            const glId = await postTransfer(
                ledger.path,
                account1,
                entryId1,
                account2,
                entryId2,
            );
            if (
                selectedLoginName.trim().length > 0 &&
                selectedPipelineLabel !== null
            ) {
                await refreshAccountPipelineData(
                    selectedLoginName.trim(),
                    selectedPipelineLabel,
                    selectedPipelineGlAccount,
                );
            }
            setPipelineStatus(
                `Transfer posting complete: ${entryId1} ↔ ${entryId2} (${glId}).`,
            );
            onLedgerRefresh();
        } catch (error) {
            setPipelineStatus(`Transfer post failed: ${String(error)}`);
        } finally {
            setIsPostingTransfer(false);
        }
    }

    async function handleMigrateLegacyLedger() {
        if (!ledger) return;
        setIsMigratingLegacyLedger(true);
        setScrapeStatus('Migrating legacy accounts layout...');
        try {
            const outcome = await migrateLedger(ledger.path, false);
            setLegacyMigrationPreview(null);
            setScrapeStatus(
                `Migration complete. Migrated ${outcome.migrated.length} account(s).`,
            );
            onLedgerRefresh();
        } catch (error) {
            setScrapeStatus(`Migration failed: ${String(error)}`);
        } finally {
            setIsMigratingLegacyLedger(false);
        }
    }

    async function handleRunScrape() {
        if (!ledger) return;
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Login is required.');
            return;
        }

        setIsRunningScrape(true);
        setScrapeStatus(`Running scrape for ${loginName}...`);
        const timestamp = new Date().toISOString();
        try {
            await runScrapeForLogin(ledger.path, loginName);
            localStorage.setItem(`lastScrape:${loginName}`, timestamp);
            appendScrapeLog({
                loginName,
                timestamp,
                success: true,
                source: 'manual',
            });
            setScrapeStatus(`Scrape completed for ${loginName}.`);
            await onScrapeComplete(loginName);
            try {
                // Refresh pipeline for all mapped labels of this login.
                if (selectedPipelineLabel !== null) {
                    await refreshAccountPipelineData(
                        loginName,
                        selectedPipelineLabel,
                        selectedPipelineGlAccount,
                    );
                }
            } catch {
                // Surface scrape success first; pipeline reload errors are non-fatal here.
            }
        } catch (error) {
            appendScrapeLog({
                loginName,
                timestamp,
                success: false,
                error: String(error),
                source: 'manual',
            });
            setScrapeStatus(`Scrape failed: ${String(error)}`);
        } finally {
            setIsRunningScrape(false);
            setScrapeLogEntries(readScrapeLog(loginName));
        }
    }

    // ─── JSX ────────────────────────────────────────────────────────────────────

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h2>Run scrape</h2>
                        <p>
                            Select a login in the Login Management section
                            below, then run the same scraper pipeline as the CLI
                            command.
                        </p>
                    </div>
                    <div className="header-actions">
                        <details className="add-extension-disclosure">
                            <summary
                                className="ghost-button"
                                style={
                                    isImportingScrapeExtension
                                        ? {
                                              pointerEvents: 'none',
                                              opacity: 0.5,
                                          }
                                        : undefined
                                }
                            >
                                {isImportingScrapeExtension
                                    ? 'Loading...'
                                    : 'Add extension...'}
                            </summary>
                            <div className="add-extension-menu">
                                <button
                                    className="ghost-button"
                                    type="button"
                                    disabled={isImportingScrapeExtension}
                                    onClick={() => {
                                        void handleLoadScrapeExtension('zip');
                                    }}
                                >
                                    Load .zip...
                                </button>
                                <button
                                    className="ghost-button"
                                    type="button"
                                    disabled={isImportingScrapeExtension}
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
                                    disabled={isImportingScrapeExtension}
                                    onClick={() => {
                                        void handleLoadUnpackedExtension();
                                    }}
                                >
                                    Load unpacked...
                                </button>
                            </div>
                        </details>
                    </div>
                </div>
                {extensionLoadStatus === null ? null : (
                    <p className="status">{extensionLoadStatus}</p>
                )}
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
                                    Legacy `accounts/` data is present. Migrate
                                    to login-scoped storage before continuing.
                                </p>
                            </div>
                            <div className="header-actions">
                                <button
                                    className="ghost-button"
                                    type="button"
                                    disabled={isMigratingLegacyLedger}
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
                            {legacyMigrationPreview.migrated.length} account(s)
                            ready to migrate.{' '}
                            {legacyMigrationPreview.skipped.length} account(s)
                            will be skipped.
                        </p>
                        {legacyMigrationPreview.warnings.length > 0 ? (
                            <p className="status">
                                Warnings:{' '}
                                {legacyMigrationPreview.warnings.length}. Run
                                CLI `refreshmint migrate --dry-run` for details.
                            </p>
                        ) : null}
                    </section>
                )}
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Login mappings</h3>
                            <p>
                                Configure login names, extension defaults, and
                                label to GL account mappings.
                            </p>
                        </div>
                        <div className="header-actions">
                            <button
                                className="ghost-button"
                                type="button"
                                onClick={() => {
                                    onLoginConfigChanged();
                                }}
                                disabled={
                                    isLoadingLoginConfigs || isSavingLoginConfig
                                }
                            >
                                {isLoadingLoginConfigs
                                    ? 'Refreshing...'
                                    : 'Refresh logins'}
                            </button>
                        </div>
                    </div>
                    <fieldset>
                        <legend>
                            <div className="tabs pipeline-subtabs">
                                <button
                                    type="button"
                                    className={
                                        loginManagementTab === 'select'
                                            ? 'tab active'
                                            : 'tab'
                                    }
                                    onClick={() => {
                                        onLoginManagementTabChange('select');
                                    }}
                                    disabled={loginNames.length === 0}
                                >
                                    Existing login
                                </button>
                                <button
                                    type="button"
                                    className={
                                        loginManagementTab === 'create'
                                            ? 'tab active'
                                            : 'tab'
                                    }
                                    onClick={() => {
                                        onLoginManagementTabChange('create');
                                    }}
                                >
                                    Create new login
                                </button>
                            </div>
                        </legend>
                        {loginManagementTab === 'create' ? (
                            <div className="login-create-body">
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
                                                onLoginConfigStatusChange(null);
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
                                                onLoginConfigStatusChange(null);
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
                            </div>
                        ) : (
                            <>
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Existing login</span>
                                        <select
                                            value={selectedLoginName}
                                            onChange={(event) => {
                                                onSelectedLoginNameChange(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
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
                                        <select
                                            value={selectedLoginExtensionDraft}
                                            onChange={(event) => {
                                                setSelectedLoginExtensionDraft(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 ||
                                                isSavingLoginConfig ||
                                                isLoadingScrapeExtensions
                                            }
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
                                            {selectedLoginExtensionDraft.length >
                                                0 &&
                                            !scrapeExtensions.includes(
                                                selectedLoginExtensionDraft,
                                            ) ? (
                                                <option
                                                    value={
                                                        selectedLoginExtensionDraft
                                                    }
                                                >
                                                    {selectedLoginExtensionDraft.includes(
                                                        '/',
                                                    ) ||
                                                    selectedLoginExtensionDraft.includes(
                                                        '\\',
                                                    )
                                                        ? `(unpacked) ${selectedLoginExtensionDraft.split('/').pop() ?? selectedLoginExtensionDraft}`
                                                        : selectedLoginExtensionDraft}
                                                </option>
                                            ) : null}
                                        </select>
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
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
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
                                            void handleRepairSelectedLoginLabels();
                                        }}
                                        disabled={
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
                                        }
                                    >
                                        {isSavingLoginConfig
                                            ? 'Saving...'
                                            : 'Repair labels'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleDeleteSelectedLoginConfig();
                                        }}
                                        disabled={
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
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
                                        No labels configured yet. Labels are
                                        discovered automatically on the first
                                        scrape run. Use + to add a mapping
                                        manually.
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
                                                    ([label, config]) => {
                                                        const glAccount =
                                                            config.glAccount?.trim() ??
                                                            '';
                                                        const hasConflict =
                                                            glAccount.length >
                                                                0 &&
                                                            conflictingGlAccountSet.has(
                                                                glAccount,
                                                            );
                                                        return (
                                                            <tr key={label}>
                                                                <td>
                                                                    <span className="mono">
                                                                        {label}
                                                                    </span>
                                                                </td>
                                                                <td>
                                                                    {config.glAccount ??
                                                                        '(ignored)'}
                                                                    {hasConflict ? (
                                                                        <span className="secret-chip">
                                                                            conflict
                                                                        </span>
                                                                    ) : null}
                                                                </td>
                                                                <td>
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        onClick={() => {
                                                                            onEditingMappingLabelChange(
                                                                                label,
                                                                            );
                                                                            onEditingMappingGlAccountDraftChange(
                                                                                config.glAccount ??
                                                                                    '',
                                                                            );
                                                                            onLoginConfigStatusChange(
                                                                                null,
                                                                            );
                                                                        }}
                                                                        disabled={
                                                                            isSavingLoginConfig
                                                                        }
                                                                    >
                                                                        Edit
                                                                    </button>
                                                                    {glAccount.length >
                                                                    0 ? (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            onClick={() => {
                                                                                void onIgnoreLoginAccountMapping(
                                                                                    selectedLoginName,
                                                                                    label,
                                                                                    glAccount,
                                                                                );
                                                                            }}
                                                                            disabled={
                                                                                isSavingLoginConfig
                                                                            }
                                                                        >
                                                                            Ignore
                                                                        </button>
                                                                    ) : null}
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
                                                        );
                                                    },
                                                )}
                                            </tbody>
                                        </table>
                                    </div>
                                )}
                                {selectedLoginConflictCount > 0 ? (
                                    <p className="status">
                                        {selectedLoginConflictCount} mapping
                                        conflict
                                        {selectedLoginConflictCount === 1
                                            ? ''
                                            : 's'}{' '}
                                        for this login. Resolve by editing or
                                        ignoring a conflicting mapping.
                                    </p>
                                ) : null}
                                {selectedLoginConfig !== null &&
                                editingMappingLabel === null ? (
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                onEditingMappingLabelChange('');
                                                onEditingMappingGlAccountDraftChange(
                                                    '',
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            + Add mapping
                                        </button>
                                    </div>
                                ) : null}
                                {editingMappingLabel !== null ? (
                                    <>
                                        <div className="txn-grid">
                                            <label className="field">
                                                <span>Label</span>
                                                <input
                                                    type="text"
                                                    value={editingMappingLabel}
                                                    placeholder="checking"
                                                    readOnly={
                                                        editingMappingLabel.length >
                                                        0
                                                    }
                                                    onChange={(event) => {
                                                        onEditingMappingLabelChange(
                                                            event.target.value,
                                                        );
                                                        onLoginConfigStatusChange(
                                                            null,
                                                        );
                                                    }}
                                                    disabled={
                                                        isSavingLoginConfig
                                                    }
                                                />
                                            </label>
                                            <label className="field">
                                                <span>GL account</span>
                                                <input
                                                    type="text"
                                                    value={
                                                        editingMappingGlAccountDraft
                                                    }
                                                    placeholder="Assets:Bank:Checking (blank = ignored)"
                                                    list="scrape-account-options"
                                                    onChange={(event) => {
                                                        onEditingMappingGlAccountDraftChange(
                                                            event.target.value,
                                                        );
                                                        onLoginConfigStatusChange(
                                                            null,
                                                        );
                                                    }}
                                                    disabled={
                                                        isSavingLoginConfig
                                                    }
                                                />
                                            </label>
                                        </div>
                                        <datalist id="scrape-account-options">
                                            {scrapeAccountOptions.map(
                                                (name) => (
                                                    <option
                                                        key={name}
                                                        value={name}
                                                    />
                                                ),
                                            )}
                                        </datalist>
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
                                                    : 'Save mapping'}
                                            </button>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    onEditingMappingLabelChange(
                                                        null,
                                                    );
                                                    onLoginConfigStatusChange(
                                                        null,
                                                    );
                                                }}
                                                disabled={isSavingLoginConfig}
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                    </>
                                ) : null}
                            </>
                        )}
                        {loginConfigStatus === null ? null : (
                            <p className="status">{loginConfigStatus}</p>
                        )}
                    </fieldset>
                </section>
                {hasActiveScrapeLogin ? (
                    <p className="hint mono">Login: {activeScrapeLoginName}</p>
                ) : (
                    <p className="hint">
                        Select a login in the Login Management section above to
                        run scrape or start a debug session.
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
                        {isRunningScrape ? 'Running scrape...' : 'Run scrape'}
                    </button>
                    <button
                        type="button"
                        className="secondary-button"
                        onClick={onScrapeAll}
                        disabled={
                            autoScrapeActive !== null ||
                            isRunningScrape ||
                            loginNames.length === 0
                        }
                    >
                        Scrape and Extract All
                    </button>
                </div>
                {scrapeStatus === null ? null : (
                    <p
                        className={
                            scrapeStatus.toLowerCase().includes('failed') ||
                            scrapeStatus.toLowerCase().includes('error')
                                ? 'status status-error'
                                : 'status'
                        }
                    >
                        {scrapeStatus}
                    </p>
                )}
                {scrapeLogEntries.length > 0 && (
                    <details className="scrape-log-disclosure">
                        <summary className="disclosure-summary">
                            Scrape log ({scrapeLogEntries.length})
                        </summary>
                        <table className="scrape-log-table">
                            <thead>
                                <tr>
                                    <th>Time</th>
                                    <th>Source</th>
                                    <th>Status</th>
                                    <th>Error</th>
                                </tr>
                            </thead>
                            <tbody>
                                {scrapeLogEntries.map((entry, i) => (
                                    <tr
                                        key={i}
                                        className={
                                            entry.success ? '' : 'status-error'
                                        }
                                    >
                                        <td>
                                            {new Date(
                                                entry.timestamp,
                                            ).toLocaleString()}
                                        </td>
                                        <td>{entry.source}</td>
                                        <td>
                                            {entry.success ? 'OK' : 'Failed'}
                                        </td>
                                        <td>{entry.error ?? ''}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </details>
                )}
                <details className="dev-tools-disclosure">
                    <summary className="disclosure-summary">
                        Developer tools
                        {scrapeDebugSocket !== null ? ' (session active)' : ''}
                    </summary>
                    <div className="dev-tools-body">
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
                    </div>
                </details>
                <section className="secrets-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Login secrets</h3>
                            <p>
                                Manage per-login keychain secrets for the active
                                login selection.
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
                                    !isSecretsPanelExpanded ||
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
                    <details
                        className="login-create-disclosure"
                        open={isSecretsPanelExpanded}
                        onToggle={(event) => {
                            setIsSecretsPanelExpanded(event.currentTarget.open);
                        }}
                    >
                        <summary className="disclosure-summary">
                            {isSecretsPanelExpanded
                                ? 'Hide secrets'
                                : 'Show secrets'}
                        </summary>
                        <div className="login-create-body">
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
                                        <span>Username</span>
                                        <input
                                            type="text"
                                            autoComplete="username"
                                            value={secretUsername}
                                            placeholder="username"
                                            onChange={(event) => {
                                                setSecretUsername(
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
                                        <span>Password</span>
                                        <input
                                            type="password"
                                            autoComplete="new-password"
                                            value={secretPassword}
                                            placeholder={
                                                currentSecretEntry?.hasPassword ===
                                                true
                                                    ? '●●●●●●●●'
                                                    : ''
                                            }
                                            onChange={(event) => {
                                                setSecretPassword(
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
                                        type="submit"
                                        className="ghost-button"
                                        disabled={
                                            !hasActiveSecretsLogin ||
                                            trimmedSecretDomain.length === 0 ||
                                            isSavingAccountSecret ||
                                            busySecretKey !== null
                                        }
                                    >
                                        {isSavingAccountSecret
                                            ? 'Saving...'
                                            : currentDomainExists
                                              ? 'Update credentials'
                                              : 'Save credentials'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleMigrateLoginSecrets();
                                        }}
                                        disabled={
                                            !hasActiveSecretsLogin ||
                                            isSavingAccountSecret ||
                                            busySecretKey !== null
                                        }
                                    >
                                        Migrate legacy
                                    </button>
                                </div>
                                <p className="hint">
                                    Enter domain, username, and password.
                                    Username is stored without biometric;
                                    password requires Touch ID / Face ID on
                                    macOS.
                                </p>
                            </form>
                            {isLoadingAccountSecrets ? (
                                <p className="status">
                                    Loading login secrets...
                                </p>
                            ) : accountSecrets.length === 0 ? (
                                <p className="hint">
                                    {hasActiveSecretsLogin
                                        ? 'No credentials stored for this login.'
                                        : 'Select a login to manage secrets.'}
                                </p>
                            ) : (
                                <div className="table-wrap">
                                    <table className="ledger-table">
                                        <thead>
                                            <tr>
                                                <th>Domain</th>
                                                <th>Username</th>
                                                <th>Password</th>
                                                <th>Actions</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {accountSecrets.map((entry) => {
                                                const key = secretDomainKey(
                                                    entry.domain,
                                                );
                                                const isBusy =
                                                    busySecretKey === key;
                                                const isExtra =
                                                    hasRequiredSecretsSync &&
                                                    !requiredSecretDomainSet.has(
                                                        key,
                                                    );
                                                return (
                                                    <tr key={key}>
                                                        <td>
                                                            <span>
                                                                {entry.domain}
                                                            </span>
                                                            {isExtra ? (
                                                                <span className="secret-chip">
                                                                    extra
                                                                </span>
                                                            ) : null}
                                                        </td>
                                                        <td>
                                                            {entry.hasUsername
                                                                ? '(set)'
                                                                : '—'}
                                                        </td>
                                                        <td>
                                                            {entry.hasPassword
                                                                ? '●●●●●●●●'
                                                                : '—'}
                                                        </td>
                                                        <td>
                                                            <div className="pipeline-row-actions">
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    onClick={() => {
                                                                        void handleEditDomainPreset(
                                                                            entry.domain,
                                                                        );
                                                                    }}
                                                                    disabled={
                                                                        isBusy ||
                                                                        isSavingAccountSecret
                                                                    }
                                                                >
                                                                    Edit
                                                                </button>
                                                                <button
                                                                    type="button"
                                                                    className="ghost-button"
                                                                    onClick={() => {
                                                                        void handleRemoveDomainSecret(
                                                                            entry.domain,
                                                                        );
                                                                    }}
                                                                    disabled={
                                                                        isBusy ||
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
                                            })}
                                        </tbody>
                                    </table>
                                </div>
                            )}
                            {hasRequiredSecretsSync && extraSecretCount > 0 ? (
                                <p className="hint">
                                    {extraSecretCount} domain
                                    {extraSecretCount === 1 ? '' : 's'} stored
                                    for this login are not declared by the
                                    selected extension.
                                </p>
                            ) : null}
                            {secretsStatus === null ? null : (
                                <p className="status">{secretsStatus}</p>
                            )}
                        </div>
                    </details>
                </section>
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Extraction pipeline</h3>
                            <p>
                                Select documents, run extraction, and review
                                account-level journal and posting state.
                            </p>
                        </div>
                        <div className="header-actions">
                            {selectedLoginMappedLabels.length > 1 ? (
                                <label className="field">
                                    <span>Label</span>
                                    <select
                                        value={selectedPipelineLabel ?? ''}
                                        onChange={(event) => {
                                            setSelectedPipelineLabel(
                                                event.target.value.length > 0
                                                    ? event.target.value
                                                    : null,
                                            );
                                        }}
                                    >
                                        <option value="">Select label</option>
                                        {selectedLoginMappedLabels.map(
                                            (label) => (
                                                <option
                                                    key={label}
                                                    value={label}
                                                >
                                                    {label}
                                                </option>
                                            ),
                                        )}
                                    </select>
                                </label>
                            ) : null}
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
                                    isLoadingUnposted ||
                                    isRunningExtraction ||
                                    busyPostEntryId !== null ||
                                    isUnpostingEntry ||
                                    isPostingTransfer
                                }
                            >
                                {isLoadingDocuments ||
                                isLoadingAccountJournal ||
                                isLoadingUnposted
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
                                    documents.map((doc) => doc.filename),
                                );
                                setPipelineStatus(null);
                            }}
                            disabled={
                                isLoadingDocuments || documents.length === 0
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
                                selectedDocumentNames.length === 0
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
                                scrapeExtension.trim().length === 0
                            }
                        >
                            {isRunningExtraction
                                ? 'Running extraction...'
                                : `Run extraction (${selectedDocumentNames.length > 0 ? selectedDocumentNames.length : documents.length})`}
                        </button>
                    </div>
                    {isLoadingDocuments ? (
                        <p className="status">Loading documents...</p>
                    ) : documents.length === 0 ? (
                        <p className="hint">
                            {!hasActiveScrapeLogin
                                ? 'Select a login to view documents.'
                                : !hasResolvedLoginMapping
                                  ? 'Configure a GL account mapping for this login to view documents.'
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
                                    {documents.map((document) => (
                                        <tr key={document.filename}>
                                            <td>
                                                <input
                                                    type="checkbox"
                                                    checked={selectedDocumentNames.includes(
                                                        document.filename,
                                                    )}
                                                    onChange={(event) => {
                                                        handleToggleDocumentSelection(
                                                            document.filename,
                                                            event.target
                                                                .checked,
                                                        );
                                                    }}
                                                />
                                            </td>
                                            <td>
                                                <span className="mono">
                                                    {document.filename}
                                                </span>
                                            </td>
                                            <td className="mono">
                                                {document.info
                                                    ?.coverageEndDate ?? '-'}
                                            </td>
                                            <td className="mono">
                                                {document.info
                                                    ?.scrapeSessionId ?? '-'}
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    )}
                    <div className="txn-form-header">
                        <div>
                            <h3>Posting queue</h3>
                            <p>
                                Assign counterpart accounts for unposted
                                entries.
                            </p>
                        </div>
                    </div>
                    {isLoadingUnposted ? (
                        <p className="status">Loading unposted entries...</p>
                    ) : unpostedEntries.length === 0 ? (
                        <p className="hint">
                            {!hasActiveScrapeLogin
                                ? 'Select a login to view unposted entries.'
                                : !hasResolvedLoginMapping
                                  ? 'Configure a GL account mapping for this login to view unposted entries.'
                                  : 'No unposted entries for this login mapping.'}
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
                                    {unpostedEntries.map((entry) => {
                                        const draft = postDrafts[entry.id] ?? {
                                            counterpartAccount: '',
                                            postingIndex: '',
                                        };
                                        const isBusy =
                                            busyPostEntryId === entry.id;
                                        return (
                                            <tr key={entry.id}>
                                                <td className="mono">
                                                    {entry.date}
                                                </td>
                                                <td className="mono">
                                                    {entry.id}
                                                </td>
                                                <td>{entry.description}</td>
                                                <td>
                                                    <input
                                                        type="text"
                                                        value={
                                                            draft.counterpartAccount
                                                        }
                                                        placeholder="Expenses:Food"
                                                        onChange={(event) => {
                                                            handleSetPostDraft(
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
                                                        onChange={(event) => {
                                                            handleSetPostDraft(
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
                                                                void handlePostAccountEntry(
                                                                    entry.id,
                                                                );
                                                            }}
                                                            disabled={
                                                                isBusy ||
                                                                isUnpostingEntry ||
                                                                isPostingTransfer
                                                            }
                                                        >
                                                            {isBusy
                                                                ? 'Posting...'
                                                                : 'Post'}
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
                                                                            selectedPipelineGlAccount,
                                                                        entryId1:
                                                                            entry.id,
                                                                    }),
                                                                );
                                                                setPipelineStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        >
                                                            Use as A
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
                                                                            selectedPipelineGlAccount,
                                                                        entryId2:
                                                                            entry.id,
                                                                    }),
                                                                );
                                                                setPipelineStatus(
                                                                    null,
                                                                );
                                                            }}
                                                        >
                                                            Use as B
                                                        </button>
                                                    </div>
                                                </td>
                                            </tr>
                                        );
                                    })}
                                </tbody>
                            </table>
                        </div>
                    )}
                    <div className="txn-grid">
                        <label className="field">
                            <span>Unpost entry ID</span>
                            <input
                                type="text"
                                value={unpostEntryId}
                                placeholder="entry id"
                                onChange={(event) => {
                                    setUnpostEntryId(event.target.value);
                                    setPipelineStatus(null);
                                }}
                            />
                        </label>
                        <label className="field">
                            <span>Unpost posting index (optional)</span>
                            <input
                                type="text"
                                value={unpostPostingIndex}
                                placeholder="0"
                                onChange={(event) => {
                                    setUnpostPostingIndex(event.target.value);
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
                                void handleUnpostAccountEntry();
                            }}
                            disabled={isUnpostingEntry}
                        >
                            {isUnpostingEntry ? 'Unposting...' : 'Unpost entry'}
                        </button>
                    </div>
                    <div className="txn-form-header">
                        <div>
                            <h3>Transfer posting</h3>
                            <p>
                                Match two entries across accounts as a transfer.
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
                                    setTransferDraft((current) => ({
                                        ...current,
                                        account1: event.target.value,
                                    }));
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
                                    setTransferDraft((current) => ({
                                        ...current,
                                        entryId1: event.target.value,
                                    }));
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
                                    setTransferDraft((current) => ({
                                        ...current,
                                        account2: event.target.value,
                                    }));
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
                                    setTransferDraft((current) => ({
                                        ...current,
                                        entryId2: event.target.value,
                                    }));
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
                                void handlePostTransferPair();
                            }}
                            disabled={isPostingTransfer}
                        >
                            {isPostingTransfer
                                ? 'Posting transfer...'
                                : 'Post transfer'}
                        </button>
                    </div>
                </section>
                {scrapeExtensions.length === 0 && !isLoadingScrapeExtensions ? (
                    <p className="hint">
                        No runnable extensions found in extensions/*/driver.mjs.
                    </p>
                ) : null}
                {pipelineStatus === null ? null : (
                    <p className="status">{pipelineStatus}</p>
                )}
            </section>
        </div>
    );
}
