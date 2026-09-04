import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import {
    confirm as confirmDialog,
    open as openDialog,
} from '@tauri-apps/plugin-dialog';
import {
    type DomainSecretEntry,
    type LedgerView,
    type LoginConfig,
    type MigrationOutcome,
    createLogin,
    deleteLogin,
    deleteLoginAccount,
    getLoginConfig,
    getLoginUsername,
    getScrapeDebugSessionSocket,
    listLoginSecrets,
    listScrapeExtensions,
    loadScrapeExtension,
    migrateLedger,
    migrateLoginSecrets,
    removeLoginDomain,
    repairLoginAccountLabels,
    setLoginAccount,
    setLoginCredentials,
    setLoginExtension,
    setLoginPassword,
    setLoginUsername,
    startScrapeDebugSessionForLogin,
    stopScrapeDebugSession,
    syncLoginSecretsForExtension,
} from '../tauri-commands.ts';
import { type SecretPromptState, normalizeLoginConfig } from '../types.ts';
import { AccountInput } from '../components/AccountInput.tsx';

interface SettingsTabProps {
    ledger: LedgerView | null;
    // Selected login is lifted to App and shared with the Scrape tab so both
    // tabs act on the same login.
    selectedLoginName: string;
    onSelectedLoginNameChange: (name: string) => void;
    // Login config data + mapping-edit state, lifted to App (the global
    // conflicts panel drives the same edit form).
    loginNames: string[];
    loginConfigsByName: Record<string, LoginConfig>;
    isLoadingLoginConfigs: boolean;
    conflictingGlAccountSet: Set<string>;
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
    onLoginConfigChanged: () => void;
    onIgnoreLoginAccountMapping: (
        loginName: string,
        label: string,
        glAccount: string,
    ) => Promise<void>;
    onLedgerRefresh: () => void;
    onSecretPrompt: (p: SecretPromptState) => Promise<boolean>;
    headlessScrape: boolean;
}

function secretDomainKey(domain: string): string {
    return domain;
}

export function SettingsTab({
    ledger,
    selectedLoginName,
    onSelectedLoginNameChange,
    loginNames,
    loginConfigsByName,
    isLoadingLoginConfigs,
    conflictingGlAccountSet,
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
    onIgnoreLoginAccountMapping,
    onLedgerRefresh,
    onSecretPrompt,
    headlessScrape,
}: SettingsTabProps) {
    const [selectedLoginExtensionDraft, setSelectedLoginExtensionDraft] =
        useState('');
    const [newLoginName, setNewLoginName] = useState('');
    const [newLoginExtension, setNewLoginExtension] = useState('');
    // scrapeExtension is the selected login's extension name; the secrets-sync
    // effect uses it. scrapeExtensions is the available list (its own copy —
    // the Scrape tab keeps a separate copy for the login-mapping dropdown).
    const [scrapeExtension, setScrapeExtension] = useState('');
    const [scrapeExtensions, setScrapeExtensions] = useState<string[]>([]);
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isImportingScrapeExtension, setIsImportingScrapeExtension] =
        useState(false);
    const [extensionLoadStatus, setExtensionLoadStatus] = useState<
        string | null
    >(null);
    const [settingsStatus, setSettingsStatus] = useState<string | null>(null);
    const [scrapeDebugSocket, setScrapeDebugSocket] = useState<string | null>(
        null,
    );
    const [isStartingScrapeDebug, setIsStartingScrapeDebug] = useState(false);
    const [isStoppingScrapeDebug, setIsStoppingScrapeDebug] = useState(false);
    const [legacyMigrationPreview, setLegacyMigrationPreview] =
        useState<MigrationOutcome | null>(null);
    const [isCheckingLegacyMigration, setIsCheckingLegacyMigration] =
        useState(false);
    const [isMigratingLegacyLedger, setIsMigratingLegacyLedger] =
        useState(false);
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

    const secretDomainRef = useRef('');
    const ledgerPath = ledger?.path ?? null;

    // ─── Computed values ────────────────────────────────────────────────────────

    const activeSecretsLoginName = selectedLoginName.trim() || null;
    const hasActiveSecretsLogin = activeSecretsLoginName !== null;

    const scrapeAccountOptions = ledger
        ? ledger.accounts
              .map((account) => account.name.trim())
              .filter(
                  (name, index, names) =>
                      name.length > 0 && names.indexOf(name) === index,
              )
        : [];
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

    // Auto-populate the login-extension draft when the selected login changes.
    useEffect(() => {
        const extension = selectedLoginConfig?.extension?.trim() ?? '';
        setSelectedLoginExtensionDraft(extension);
    }, [selectedLoginConfig]);

    // Reset own state when the ledger path changes.
    useEffect(() => {
        setSettingsStatus(null);
        setExtensionLoadStatus(null);
        setSelectedLoginExtensionDraft('');
        setNewLoginName('');
        setNewLoginExtension('');
        setScrapeDebugSocket(null);
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
    }, [ledgerPath]);

    // List scrape extensions when ledger changes (own copy).
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtensions([]);
            setScrapeExtension('');
            setIsLoadingScrapeExtensions(false);
            return;
        }

        let cancelled = false;
        setIsLoadingScrapeExtensions(true);
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
                    setSettingsStatus(
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

    // Load extension name from login config when the active login changes.
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtension('');
            return;
        }

        const loginName = activeSecretsLoginName;
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
    }, [activeSecretsLoginName, ledgerPath]);

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
            setExtensionLoadStatus('Extension load canceled.');
            return;
        }

        setIsImportingScrapeExtension(true);
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
            const loginName = activeSecretsLoginName;
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
            setExtensionLoadStatus(
                `Loaded extension '${loadedExtensionName}'.`,
            );
        } catch (error) {
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
            setSettingsStatus('Extension load canceled.');
            return;
        }

        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSettingsStatus('Select a login first.');
            return;
        }

        try {
            await setLoginExtension(ledger.path, loginName, source);
            setScrapeExtension(source);
            setSettingsStatus(`Set unpacked extension: ${source}`);
        } catch (error) {
            setSettingsStatus(
                `Failed to set unpacked extension: ${String(error)}`,
            );
        }
    }

    async function handleStartScrapeDebug() {
        if (!ledger) return;
        const loginName = activeSecretsLoginName;
        if (loginName === null) {
            setSettingsStatus('Select a login first.');
            return;
        }
        setIsStartingScrapeDebug(true);
        setSettingsStatus('Starting debug session...');
        try {
            const socket = await startScrapeDebugSessionForLogin(
                ledger.path,
                loginName,
                headlessScrape,
            );
            setScrapeDebugSocket(socket);
            setSettingsStatus(`Debug session started. Socket: ${socket}`);
        } catch (error) {
            setSettingsStatus(
                `Failed to start debug session: ${String(error)}`,
            );
        } finally {
            setIsStartingScrapeDebug(false);
        }
    }

    async function handleStopScrapeDebug() {
        setIsStoppingScrapeDebug(true);
        try {
            await stopScrapeDebugSession();
            setScrapeDebugSocket(null);
            setSettingsStatus('Debug session stopped.');
        } catch (error) {
            setSettingsStatus(`Failed to stop debug session: ${String(error)}`);
        } finally {
            setIsStoppingScrapeDebug(false);
        }
    }

    async function handleCopyDebugSocket() {
        if (scrapeDebugSocket === null) return;
        try {
            await navigator.clipboard.writeText(scrapeDebugSocket);
            setSettingsStatus('Debug socket copied to clipboard.');
        } catch (error) {
            setSettingsStatus(`Failed to copy socket: ${String(error)}`);
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

    function handleSubmitSecretForm(event: SyntheticEvent<HTMLFormElement>) {
        event.preventDefault();
        void handleSaveDomainCredentials();
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

    async function handleMigrateLegacyLedger() {
        if (!ledger) return;
        setIsMigratingLegacyLedger(true);
        setSettingsStatus('Migrating legacy accounts layout...');
        try {
            const outcome = await migrateLedger(ledger.path, false);
            setLegacyMigrationPreview(null);
            setSettingsStatus(
                `Migration complete. Migrated ${outcome.migrated.length} account(s).`,
            );
            onLedgerRefresh();
        } catch (error) {
            setSettingsStatus(`Migration failed: ${String(error)}`);
        } finally {
            setIsMigratingLegacyLedger(false);
        }
    }

    // ─── JSX ────────────────────────────────────────────────────────────────────

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h2>Logins</h2>
                        <p>
                            Manage scraper extensions, per-login secrets, legacy
                            migrations, and developer tools. (App-wide behavior
                            toggles live under Preferences.)
                        </p>
                    </div>
                </div>
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Connections</h3>
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
                                                                        <span className="chip secret-chip">
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
                                                <AccountInput
                                                    value={
                                                        editingMappingGlAccountDraft
                                                    }
                                                    onChange={(next) => {
                                                        onEditingMappingGlAccountDraftChange(
                                                            next,
                                                        );
                                                        onLoginConfigStatusChange(
                                                            null,
                                                        );
                                                    }}
                                                    accounts={
                                                        scrapeAccountOptions
                                                    }
                                                    placeholder="Assets:Bank:Checking (blank = ignored)"
                                                    disabled={
                                                        isSavingLoginConfig
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
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Extensions</h3>
                            <p>
                                Load scraper extensions from a .zip or a source
                                directory.
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
                                        disabled={isImportingScrapeExtension}
                                        onClick={() => {
                                            void handleLoadScrapeExtension(
                                                'directory',
                                            );
                                        }}
                                    >
                                        Load directory...
                                    </button>
                                </div>
                            </details>
                        </div>
                    </div>
                    {extensionLoadStatus === null ? null : (
                        <p className="status">{extensionLoadStatus}</p>
                    )}
                    {scrapeExtensions.length === 0 &&
                    !isLoadingScrapeExtensions ? (
                        <p className="hint">
                            No runnable extensions found in
                            extensions/*/driver.mjs.
                        </p>
                    ) : null}
                </section>
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Migrations</h3>
                            <p>
                                Migrate a legacy `accounts/` layout to
                                login-scoped storage.
                            </p>
                        </div>
                    </div>
                    {isCheckingLegacyMigration ? (
                        <p className="status">
                            Checking for legacy account layout...
                        </p>
                    ) : null}
                    {legacyMigrationPreview === null ? (
                        <p className="hint">No legacy migration needed.</p>
                    ) : (
                        <>
                            <div className="txn-form-header">
                                <div>
                                    <h4>Migration available</h4>
                                    <p>
                                        Legacy `accounts/` data is present.
                                        Migrate to login-scoped storage before
                                        continuing.
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
                                {legacyMigrationPreview.migrated.length}{' '}
                                account(s) ready to migrate.{' '}
                                {legacyMigrationPreview.skipped.length}{' '}
                                account(s) will be skipped.
                            </p>
                            {legacyMigrationPreview.warnings.length > 0 ? (
                                <p className="status">
                                    Warnings:{' '}
                                    {legacyMigrationPreview.warnings.length}.
                                    Run CLI `refreshmint migrate --dry-run` for
                                    details.
                                </p>
                            ) : null}
                        </>
                    )}
                </section>
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
                                                                <span className="chip secret-chip">
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
                                    !hasActiveSecretsLogin ||
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
                            <button
                                type="button"
                                className="ghost-button"
                                disabled={isImportingScrapeExtension}
                                onClick={() => {
                                    void handleLoadUnpackedExtension();
                                }}
                            >
                                Load unpacked...
                            </button>
                        </div>
                        {scrapeDebugSocket === null ? null : (
                            <p className="hint mono">
                                Debug socket: {scrapeDebugSocket}
                            </p>
                        )}
                    </div>
                </details>
                {settingsStatus === null ? null : (
                    <p
                        className={
                            settingsStatus.toLowerCase().includes('failed') ||
                            settingsStatus.toLowerCase().includes('error')
                                ? 'status status-error'
                                : 'status'
                        }
                    >
                        {settingsStatus}
                    </p>
                )}
            </section>
        </div>
    );
}
