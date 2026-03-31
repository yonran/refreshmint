import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

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
    getLoginConfig,
    type LoginConfig,
    listLogins,
    listLoginAccountDocuments,
    runLoginAccountExtraction,
    getLoginAccountUnposted,
    suggestCategories,
    postLoginAccountEntry,
    postLoginAccountTransfer,
    type CategoryResult,
    openLedger,
    runScrapeForLogin,
    type AccountRow,
    type AmountStyleHint,
    type AmountTotal,
    type LedgerView,
    setLoginAccount,
} from './tauri-commands.ts';
import { PipelineTab } from './tabs/PipelineTab.tsx';
import { ReportsTab } from './tabs/ReportsTab.tsx';
import { ScrapeTab } from './tabs/ScrapeTab.tsx';
import { TransactionsTab } from './tabs/TransactionsTab.tsx';
import {
    createEmptyPipelineTabSession,
    createEmptyTransactionsTabSession,
    type SecretPromptState,
    type LoginAccountMapping,
    type LoginAccountRef,
    type PipelineTabSession,
    type RecategorizeTab,
    type TransactionsTabSession,
    normalizeLoginConfig,
} from './types.ts';

type AppTab = ActiveTab | `recategorize:${number}`;

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
    const [autoScrapeEnabled, setAutoScrapeEnabled] = useState(
        () => localStorage.getItem('pref:autoScrapeEnabled') !== 'false',
    );
    const [autoScrapeIntervalHours, setAutoScrapeIntervalHours] = useState(() =>
        Number(localStorage.getItem('pref:autoScrapeIntervalHours') ?? '24'),
    );
    const [autoScrapeQueue, setAutoScrapeQueue] = useState<string[]>([]);
    const [autoScrapeActive, setAutoScrapeActive] = useState<string | null>(
        null,
    );
    const [autoEtlStatus, setAutoEtlStatus] = useState<string | null>(null);
    const [autoEtlErrors, setAutoEtlErrors] = useState<string | null>(null);
    const [promptRequest, setPromptRequest] = useState<{
        message: string;
    } | null>(null);
    const [scrapeLogVersion, setScrapeLogVersion] = useState(0);
    const [loginAccounts, setLoginAccounts] = useState<LoginAccountRef[]>([]);

    function handleSelectAccount(accountName: string) {
        setTransactionsTabSession((current) => ({
            ...current,
            transactionsSearch: `acct:${accountName}`,
        }));
        setActiveTab('transactions');
    }
    const [recentLedgers, setRecentLedgersState] = useState<string[]>([]);
    const [loginNames, setLoginNames] = useState<string[]>([]);
    const [loginConfigsByName, setLoginConfigsByName] = useState<
        Record<string, LoginConfig>
    >({});
    const [loginManagementTab, setLoginManagementTab] = useState<
        'select' | 'create'
    >('select');
    const [selectedLoginName, setSelectedLoginName] = useState('');
    const [editingMappingLabel, setEditingMappingLabel] = useState<
        string | null
    >(null);
    const [editingMappingGlAccountDraft, setEditingMappingGlAccountDraft] =
        useState('');
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
    const [recategorizeTabs, setRecategorizeTabs] = useState<RecategorizeTab[]>(
        [],
    );
    const [pendingTransactionSearch, setPendingTransactionSearch] = useState<
        string | null
    >(null);
    const [transactionsTabSession, setTransactionsTabSession] =
        useState<TransactionsTabSession>(createEmptyTransactionsTabSession);
    const [pipelineTabSession, setPipelineTabSession] =
        useState<PipelineTabSession>(createEmptyPipelineTabSession);
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
    const autoEtlForLoginRef = useRef<
        ((loginName: string) => Promise<void>) | null
    >(null);
    const handleLedgerRefreshRef = useRef<(() => void) | null>(null);
    const loginNamesRef = useRef<string[]>([]);
    const autoScrapeActiveRef = useRef<string | null>(autoScrapeActive);
    const promptInputRef = useRef<HTMLInputElement | null>(null);
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
            setLoginNames([]);
            setLoginConfigsByName({});
            setLoginManagementTab('select');
            setSelectedLoginName('');
            setEditingMappingLabel(null);
            setEditingMappingGlAccountDraft('');
            setLoginConfigStatus(null);
            setIsLoadingLoginConfigs(false);
            setIsSavingLoginConfig(false);
            setHasLoadedLoginConfigs(false);
            setLoginConfigsReloadToken(0);
            setLoginAccountMappings({});
            setRecategorizeTabs([]);
            setPendingTransactionSearch(null);
            setTransactionsTabSession(createEmptyTransactionsTabSession());
            setPipelineTabSession(createEmptyPipelineTabSession());
        }
    }, [ledgerPath]);

    useEffect(() => {
        if (loginNames.length === 0) {
            setLoginManagementTab('create');
        }
    }, [loginNames]);

    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === ',') {
                e.preventDefault();
                setActiveTab('preferences');
            }
        };
        window.addEventListener('keydown', handler);
        return () => {
            window.removeEventListener('keydown', handler);
        };
    }, []);

    useEffect(() => {
        if (
            activeRecategorizeTabId !== null &&
            !recategorizeTabs.some((tab) => tab.id === activeRecategorizeTabId)
        ) {
            setActiveTab('transactions');
        }
    }, [activeRecategorizeTabId, recategorizeTabs]);

    // Keep loginNamesRef current so Effect 1 can read logins without depending on loginNames.
    useEffect(() => {
        loginNamesRef.current = loginNames;
    }, [loginNames]);

    // Effect 1: build auto-scrape queue of stale logins on a timer.
    // Uses a timer rather than reacting to loginNames so that config edits
    // (which reload login configs but don't add/remove logins) do not
    // spuriously trigger scrapes.
    useEffect(() => {
        if (!autoScrapeEnabled || !ledger) return;

        function check() {
            const names = loginNamesRef.current;
            if (names.length === 0) return;
            const intervalMs = autoScrapeIntervalHours * 60 * 60 * 1000;
            const now = Date.now();
            const stale = names.filter((loginName) => {
                const last = localStorage.getItem(`lastScrape:${loginName}`);
                if (last === null) return true;
                return now - new Date(last).getTime() > intervalMs;
            });
            if (stale.length === 0) return;
            setAutoScrapeQueue((current) => {
                const toAdd = stale.filter(
                    (n) =>
                        !current.includes(n) &&
                        n !== autoScrapeActiveRef.current,
                );
                return toAdd.length === 0 ? current : [...current, ...toAdd];
            });
        }

        // Immediate check when ledger opens or autoscrape settings change.
        check();
        // Periodic re-check every 5 minutes to catch logins that become stale
        // while the app is open.
        const id = window.setInterval(check, 5 * 60 * 1000);
        return () => {
            window.clearInterval(id);
        };
    }, [ledger, autoScrapeEnabled, autoScrapeIntervalHours]);

    // Keep autoScrapeActiveRef current so the interval in Effect 1 (which
    // closes over a stale value) can see the currently active login.
    useEffect(() => {
        autoScrapeActiveRef.current = autoScrapeActive;
    }, [autoScrapeActive]);

    // Listen for prompt requests from the Rust scrape driver and surface them
    // as a blocking modal so the user can supply MFA codes etc.
    useEffect(() => {
        const unlisten = listen<{ message: string }>(
            'refreshmint://prompt-requested',
            (event) => {
                setPromptRequest({ message: event.payload.message });
            },
        );
        return () => {
            unlisten
                .then((fn) => {
                    fn();
                })
                .catch(() => {});
        };
    }, []);

    // Keep autoEtlForLoginRef current so Effect 2's async chain always sees
    // the latest loginAccounts and loginConfigsByName without adding them to
    // Effect 2's dependency array.
    useEffect(() => {
        autoEtlForLoginRef.current = async (loginName: string) => {
            if (!ledger) return;
            const ledgerPath = ledger.path;
            const accounts = loginAccounts.filter(
                (a) => a.loginName === loginName,
            );
            const cfg = normalizeLoginConfig(
                loginConfigsByName[loginName] ?? null,
            );

            setAutoEtlErrors(null);

            // Phase 1: Extract documents → account journal entries
            const extractErrors: string[] = [];
            for (const { label } of accounts) {
                if ((cfg.extension?.trim() ?? '') === '') continue;
                setAutoEtlStatus(`ETL extract: ${loginName}/${label}…`);
                try {
                    const docs = await listLoginAccountDocuments(
                        ledgerPath,
                        loginName,
                        label,
                    );
                    if (docs.length > 0) {
                        await runLoginAccountExtraction(
                            ledgerPath,
                            loginName,
                            label,
                            docs.map((d) => d.filename),
                        );
                    }
                } catch (err) {
                    console.error(
                        `Auto-ETL extract failed ${loginName}/${label}:`,
                        err,
                    );
                    extractErrors.push(`${loginName}/${label}: ${String(err)}`);
                }
            }

            // Phase 2: Post unposted entries → GL
            let posted = false;
            const postErrors: string[] = [];
            for (const { label } of accounts) {
                const glAccount = cfg.accounts[label]?.glAccount?.trim() ?? '';
                // No early-continue for missing glAccount: accounts without a
                // glAccount (e.g. target-yon) can still auto-post via transfer
                // matching. Default posting to Expenses:Unknown is skipped when
                // glAccount is absent (see else-if below).
                setAutoEtlStatus(`ETL post: ${loginName}/${label}…`);
                try {
                    const [unposted, suggestions] = await Promise.all([
                        getLoginAccountUnposted(ledgerPath, loginName, label),
                        suggestCategories(ledgerPath, loginName, label),
                    ]);
                    for (const entry of unposted) {
                        try {
                            const suggestion: CategoryResult | undefined =
                                suggestions[entry.id];
                            if (suggestion?.transferMatch) {
                                const parts =
                                    suggestion.transferMatch.accountLocator.split(
                                        '/',
                                    );
                                const otherLogin = parts[1] ?? '';
                                const otherLabel = parts[3] ?? '';
                                if (otherLogin && otherLabel) {
                                    await postLoginAccountTransfer(
                                        ledgerPath,
                                        loginName,
                                        label,
                                        entry.id,
                                        otherLogin,
                                        otherLabel,
                                        suggestion.transferMatch.entryId,
                                    );
                                    posted = true;
                                }
                            } else if (glAccount) {
                                await postLoginAccountEntry(
                                    ledgerPath,
                                    loginName,
                                    label,
                                    entry.id,
                                    'Expenses:Unknown',
                                    null,
                                );
                                posted = true;
                            }
                            // else: no glAccount and no transfer match → leave unposted
                        } catch (err) {
                            console.error(
                                `Auto-ETL post failed ${loginName}/${label}/${entry.id}:`,
                                err,
                            );
                            postErrors.push(
                                `${loginName}/${label}/${entry.id}: ${String(err)}`,
                            );
                        }
                    }
                } catch (err) {
                    console.error(
                        `Auto-ETL post phase failed ${loginName}/${label}:`,
                        err,
                    );
                    postErrors.push(`${loginName}/${label}: ${String(err)}`);
                }
            }

            if (posted) {
                handleLedgerRefreshRef.current?.();
            }

            const allErrors = [...extractErrors, ...postErrors];
            if (allErrors.length > 0) {
                setAutoEtlErrors(`Auto-ETL errors: ${allErrors.join('; ')}`);
            }
        };
    }, [ledger, loginAccounts, loginConfigsByName]);

    // Effect 2: drain queue one login at a time
    useEffect(() => {
        if (
            autoScrapeActive !== null ||
            autoScrapeQueue.length === 0 ||
            !ledger
        )
            return;
        const [loginName, ...rest] = autoScrapeQueue;
        if (loginName === undefined) return;
        setAutoScrapeActive(loginName);
        setAutoScrapeQueue(rest);
        const timestamp = new Date().toISOString();
        void runScrapeForLogin(ledger.path, loginName, 'auto')
            .then(async () => {
                localStorage.setItem(`lastScrape:${loginName}`, timestamp);
                await autoEtlForLoginRef.current?.(loginName);
            })
            .catch((error: unknown) => {
                const msg = String(error);
                setAutoEtlErrors(`Scrape error (${loginName}): ${msg}`);
            })
            .finally(() => {
                setAutoScrapeActive(null);
                setAutoEtlStatus(null);
                setScrapeLogVersion((v) => v + 1);
            });
    }, [ledger, autoScrapeQueue, autoScrapeActive]);

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
    handleLedgerRefreshRef.current = handleLedgerRefresh;

    function handleLoadConflictMapping(
        loginName: string,
        label: string,
        glAccount: string,
    ) {
        setActiveTab('scrape');
        setSelectedLoginName(loginName);
        setEditingMappingLabel(label);
        setEditingMappingGlAccountDraft(glAccount);
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
            setEditingMappingLabel(label);
            setEditingMappingGlAccountDraft('');
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

    function handleScrapeAll() {
        setAutoScrapeQueue(
            loginNamesRef.current.filter(
                (n) => n !== autoScrapeActiveRef.current,
            ),
        );
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

                    {autoScrapeActive !== null && (
                        <div className="auto-scrape-banner">
                            <span>
                                {autoEtlStatus ?? (
                                    <>
                                        Auto-scraping {autoScrapeActive}
                                        {autoScrapeQueue.length > 0
                                            ? ` (${autoScrapeQueue.length} remaining)`
                                            : ''}
                                        …
                                    </>
                                )}
                            </span>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setAutoScrapeQueue([]);
                                }}
                            >
                                Skip
                            </button>
                        </div>
                    )}
                    {autoEtlErrors !== null && (
                        <div className="auto-scrape-banner auto-scrape-banner--error">
                            <span>{autoEtlErrors}</span>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setAutoEtlErrors(null);
                                }}
                            >
                                Dismiss
                            </button>
                        </div>
                    )}

                    {activeTab === 'accounts' ? (
                        <div className="table-wrap">
                            <AccountsTable
                                accounts={ledger.accounts}
                                onSelectAccount={handleSelectAccount}
                            />
                        </div>
                    ) : activeTab === 'transactions' ||
                      activeRecategorizeTab !== null ? (
                        <TransactionsTab
                            ledger={ledger}
                            isActive={true}
                            hideObviousAmounts={hideObviousAmounts}
                            onLedgerRefresh={handleLedgerRefresh}
                            onRecategorizeTabsChange={(updater) => {
                                setRecategorizeTabs(updater);
                            }}
                            activeRecategorizeTab={activeRecategorizeTab}
                            onOpenNewRecategorizeTab={(id) => {
                                setActiveTab(`recategorize:${id}`);
                            }}
                            onNavigateToTransactions={() => {
                                setActiveTab('transactions');
                            }}
                            recategorizeTabIdRef={recategorizeTabIdRef}
                            pendingSearch={pendingTransactionSearch}
                            onPendingSearchConsumed={() => {
                                setPendingTransactionSearch(null);
                            }}
                            session={transactionsTabSession}
                            onSessionChange={setTransactionsTabSession}
                        />
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
                                setPendingTransactionSearch(id);
                                setActiveTab('transactions');
                            }}
                            session={pipelineTabSession}
                            onSessionChange={setPipelineTabSession}
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
                            <section className="preferences-section">
                                <h3>Auto-scrape</h3>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={autoScrapeEnabled}
                                        onChange={(e) => {
                                            const v = e.target.checked;
                                            setAutoScrapeEnabled(v);
                                            localStorage.setItem(
                                                'pref:autoScrapeEnabled',
                                                String(v),
                                            );
                                        }}
                                    />
                                    <span>Auto-scrape accounts when stale</span>
                                </label>
                                <label className="checkbox-field">
                                    <span>Scrape interval (hours):</span>
                                    <input
                                        type="number"
                                        min="1"
                                        value={autoScrapeIntervalHours}
                                        onChange={(e) => {
                                            const v = Number(e.target.value);
                                            setAutoScrapeIntervalHours(v);
                                            localStorage.setItem(
                                                'pref:autoScrapeIntervalHours',
                                                String(v),
                                            );
                                        }}
                                    />
                                </label>
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
                            editingMappingLabel={editingMappingLabel}
                            onEditingMappingLabelChange={setEditingMappingLabel}
                            editingMappingGlAccountDraft={
                                editingMappingGlAccountDraft
                            }
                            onEditingMappingGlAccountDraftChange={
                                setEditingMappingGlAccountDraft
                            }
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
                            scrapeLogVersion={scrapeLogVersion}
                            onScrapeComplete={async (loginName) => {
                                await autoEtlForLoginRef.current?.(loginName);
                            }}
                            onScrapeAll={handleScrapeAll}
                            autoScrapeActive={autoScrapeActive}
                        />
                    )}
                </section>
            )}
            {promptRequest === null ? null : (
                <div className="secret-prompt-overlay">
                    <div
                        className="secret-prompt"
                        role="dialog"
                        aria-modal="true"
                    >
                        <h3>Scraper prompt</h3>
                        <p>{promptRequest.message}</p>
                        <input
                            ref={promptInputRef}
                            type="text"
                            autoFocus
                            onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                    const val =
                                        promptInputRef.current?.value ?? '';
                                    setPromptRequest(null);
                                    void invoke('submit_prompt_answer', {
                                        answer: val,
                                    });
                                }
                            }}
                        />
                        <div className="txn-actions">
                            <button
                                type="button"
                                className="primary-button"
                                onClick={() => {
                                    const val =
                                        promptInputRef.current?.value ?? '';
                                    setPromptRequest(null);
                                    void invoke('submit_prompt_answer', {
                                        answer: val,
                                    });
                                }}
                            >
                                Submit
                            </button>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setPromptRequest(null);
                                    void invoke('submit_prompt_answer', {
                                        answer: null,
                                    });
                                }}
                            >
                                Cancel
                            </button>
                        </div>
                    </div>
                </div>
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

export default App;
