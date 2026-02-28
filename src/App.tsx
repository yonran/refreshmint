import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
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
    type AccountSecretEntry,
    type AccountJournalEntry,
    type AmountStyleHint,
    type AmountTotal,
    addLoginSecret,
    addTransaction,
    addTransactionText,
    type CategoryResult,
    createLogin,
    deleteLoginAccount,
    deleteLogin,
    getLoginAccountJournal,
    getLoginConfig,
    getLoginAccountUnposted,
    getLockStatusSnapshot,
    getUnpostedEntriesForTransfer,
    getScrapeDebugSessionSocket,
    type LoginAccountConfig,
    type LoginConfig,
    type LockStatus,
    type LockStatusSnapshot,
    listLoginAccountDocuments,
    listLoginSecrets,
    readAttachmentDataUrl,
    readLoginAccountDocumentRows,
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
    postLoginAccountEntry,
    postLoginAccountTransfer,
    postTransfer,
    reenterLoginSecret,
    removeLoginSecret,
    runLoginAccountExtraction,
    runScrapeForLogin,
    setLoginAccount,
    setLoginExtension,
    startScrapeDebugSessionForLogin,
    startLockMetadataWatch,
    stopLockMetadataWatch,
    stopScrapeDebugSession,
    suggestCategories,
    suggestGlCategories,
    recategorizeGlTransaction,
    mergeGlTransfer,
    type GlCategoryResult,
    syncGlTransaction,
    type SecretEntry,
    syncLoginSecretsForExtension,
    type TransactionRow,
    type UnpostedTransferResult,
    unpostLoginAccountEntry,
    validateTransaction,
    validateTransactionText,
    queryTransactions,
} from './tauri-commands.ts';
import { getCurrentToken, getSearchSuggestions } from './search-utils.ts';
import { ReportsTab } from './ReportsTab.tsx';

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

type PostDraft = {
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

type LoginAccountRef = {
    loginName: string;
    label: string;
};

type PipelineBulkAccountStat = {
    loginName: string;
    label: string;
    extract: {
        eligible: boolean;
        documentCount: number;
        skipReason: 'missing-extension' | 'no-documents' | null;
        inspectError: string | null;
        locked: boolean;
    };
    post: {
        eligible: boolean;
        unpostedCount: number;
        skipReason: 'missing-gl-account' | 'no-unposted' | null;
        inspectError: string | null;
        locked: boolean;
    };
};

type PipelineBulkSummary = {
    eligibleAccounts: number;
    totalDocuments: number;
    totalUnpostedEntries: number;
    skippedMissingExtension: number;
    skippedNoDocuments: number;
    skippedMissingGlAccount: number;
    skippedNoUnposted: number;
    inspectFailures: number;
    lockedAccounts: number;
};

type PipelineBulkStats = {
    accounts: PipelineBulkAccountStat[];
    gl: LockStatus;
    extract: PipelineBulkSummary;
    post: PipelineBulkSummary;
};

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeLoginAccountConfig(value: unknown): LoginAccountConfig {
    if (!isRecord(value)) {
        return {};
    }
    const glAccount = value['glAccount'];
    if (typeof glAccount === 'string' || glAccount === null) {
        return { glAccount };
    }
    return {};
}

function normalizeLoginConfig(
    value: LoginConfig | null | undefined,
): LoginConfig {
    if (!isRecord(value)) {
        return { accounts: {} };
    }
    const extension = value['extension'];
    const accounts: Record<string, LoginAccountConfig> = {};
    const rawAccounts = value['accounts'];
    if (isRecord(rawAccounts)) {
        for (const [label, accountConfig] of Object.entries(rawAccounts)) {
            accounts[label] = normalizeLoginAccountConfig(accountConfig);
        }
    }
    const normalized: LoginConfig = { accounts };
    if (typeof extension === 'string') {
        normalized.extension = extension;
    }
    return normalized;
}

function suggestGlAccountName(label: string): string {
    const lc = label.toLowerCase();
    const name = label.charAt(0).toUpperCase() + label.slice(1);
    if (/credit|card|visa|mastercard|amex|discover/.test(lc)) {
        return `Liabilities:CreditCard:${name}`;
    }
    if (/savings/.test(lc)) {
        return `Assets:Savings:${name}`;
    }
    return `Assets:Checking:${name}`;
}

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

function createEmptyPipelineBulkSummary(): PipelineBulkSummary {
    return {
        eligibleAccounts: 0,
        totalDocuments: 0,
        totalUnpostedEntries: 0,
        skippedMissingExtension: 0,
        skippedNoDocuments: 0,
        skippedMissingGlAccount: 0,
        skippedNoUnposted: 0,
        inspectFailures: 0,
        lockedAccounts: 0,
    };
}

function App() {
    const [createStatus, setCreateStatus] = useState<string | null>(null);
    const [isCreating, setIsCreating] = useState(false);
    const [openStatus, setOpenStatus] = useState<string | null>(null);
    const [isOpening, setIsOpening] = useState(false);
    const [ledger, setLedger] = useState<LedgerView | null>(null);
    const [activeTab, setActiveTab] = useState<ActiveTab>('accounts');
    const [selectedAccount, setSelectedAccount] = useState<string | null>(null);
    const [unpostedOnly, setUnpostedOnly] = useState(false);
    const [selectedLoginAccount, setSelectedLoginAccount] =
        useState<LoginAccountRef | null>(null);
    const [loginAccounts, setLoginAccounts] = useState<LoginAccountRef[]>([]);

    function handleSelectAccount(accountName: string) {
        setSelectedAccount((current) =>
            current === accountName ? null : accountName,
        );
        setScrapeAccount(accountName);
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
    const [loginManagementTab, setLoginManagementTab] = useState<
        'select' | 'create'
    >('select');
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
    const [hasLoadedLoginConfigs, setHasLoadedLoginConfigs] = useState(false);
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
    const [isSecretsPanelExpanded, setIsSecretsPanelExpanded] = useState(false);
    const [secretsStatus, setSecretsStatus] = useState<string | null>(null);
    const [isRunningScrape, setIsRunningScrape] = useState(false);
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isImportingScrapeExtension, setIsImportingScrapeExtension] =
        useState(false);
    const [extensionLoadStatus, setExtensionLoadStatus] = useState<
        string | null
    >(null);
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
    const [unpostedEntries, setUnpostedEntries] = useState<
        AccountJournalEntry[]
    >([]);
    const [pipelineStatus, setPipelineStatus] = useState<string | null>(null);
    const [pipelineSubTab, setPipelineSubTab] = useState<
        'evidence' | 'evidence-rows' | 'account-rows' | 'gl-rows'
    >('evidence');
    const [evidenceRowsDocument, setEvidenceRowsDocument] = useState('');
    const [documentRows, setDocumentRows] = useState<string[][]>([]);
    const [isLoadingDocumentRows, setIsLoadingDocumentRows] = useState(false);
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
    const [pipelineSelectedEntryIds, setPipelineSelectedEntryIds] = useState<
        Set<string>
    >(new Set());
    const [pipelineCategorySuggestions, setPipelineCategorySuggestions] =
        useState<Record<string, CategoryResult>>({});
    const [pipelineGlAccountDraft, setPipelineGlAccountDraft] = useState('');
    const [isSavingPipelineGlAccount, setIsSavingPipelineGlAccount] =
        useState(false);
    const [isPipelinePosting, setIsPipelinePosting] = useState(false);
    const [isPipelineExtractingAllLedger, setIsPipelineExtractingAllLedger] =
        useState(false);
    const [isPipelinePostingAllLedger, setIsPipelinePostingAllLedger] =
        useState(false);
    const [lockStatusSnapshot, setLockStatusSnapshot] =
        useState<LockStatusSnapshot | null>(null);
    const [pipelineBulkStats, setPipelineBulkStats] =
        useState<PipelineBulkStats | null>(null);
    const [isLoadingPipelineBulkStats, setIsLoadingPipelineBulkStats] =
        useState(false);
    const [transferModalEntryId, setTransferModalEntryId] = useState<
        string | null
    >(null);
    const [transferModalResults, setTransferModalResults] = useState<
        UnpostedTransferResult[]
    >([]);
    const [isLoadingTransferModal, setIsLoadingTransferModal] = useState(false);
    const [transactionsSearch, setTransactionsSearch] = useState('');
    const [transferModalSearch, setTransferModalSearch] = useState('');
    const [queryResults, setQueryResults] = useState<TransactionRow[] | null>(
        null,
    );
    const [queryError, setQueryError] = useState<string | null>(null);
    const [isNewTxnExpandedOverride, setIsNewTxnExpandedOverride] = useState<
        boolean | null
    >(null);
    const [acSuggestions, setAcSuggestions] = useState<string[]>([]);
    const [acActiveIndex, setAcActiveIndex] = useState(-1);
    const [glCategorySuggestions, setGlCategorySuggestions] = useState<
        Record<string, GlCategoryResult>
    >({});
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
    const suggestRequestId = useRef(0);
    const searchInputRef = useRef<HTMLInputElement>(null);
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
            : Object.entries(
                  normalizeLoginConfig(selectedLoginConfig).accounts,
              ).sort(([a], [b]) => a.localeCompare(b));
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
    const selectedScrapeAccountHasConflict =
        selectedScrapeAccount.length > 0 &&
        conflictingGlAccountSet.has(selectedScrapeAccount);
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
    const hasExtension =
        selectedLoginAccount !== null &&
        (loginConfigsByName[
            selectedLoginAccount.loginName
        ]?.extension?.trim() ?? '') !== '';
    const requestLoginConfigReload = useCallback(() => {
        setLoginConfigsReloadToken((current) => current + 1);
    }, []);
    const refreshPipelineBulkStats = useCallback(async () => {
        if (ledgerPath === null) {
            setPipelineBulkStats(null);
            setLockStatusSnapshot(null);
            return null;
        }

        setIsLoadingPipelineBulkStats(true);
        try {
            const uniqueLoginNames = Array.from(
                new Set(loginAccounts.map((account) => account.loginName)),
            );
            const snapshot = await getLockStatusSnapshot(
                ledgerPath,
                uniqueLoginNames,
            );
            const accountStats = await Promise.all(
                loginAccounts.map(async ({ loginName, label }) => {
                    const normalizedConfig = normalizeLoginConfig(
                        loginConfigsByName[loginName] ?? null,
                    );
                    const extension = normalizedConfig.extension?.trim() ?? '';
                    const glAccount =
                        normalizedConfig.accounts[label]?.glAccount?.trim() ??
                        '';
                    const locked = snapshot.logins[loginName]?.locked ?? false;

                    let documentCount = 0;
                    let extractSkipReason:
                        | 'missing-extension'
                        | 'no-documents'
                        | null = null;
                    let extractInspectError: string | null = null;
                    if (extension.length === 0) {
                        extractSkipReason = 'missing-extension';
                    } else {
                        try {
                            const docs = await listLoginAccountDocuments(
                                ledgerPath,
                                loginName,
                                label,
                            );
                            documentCount = docs.length;
                            if (documentCount === 0) {
                                extractSkipReason = 'no-documents';
                            }
                        } catch (error) {
                            extractInspectError = String(error);
                        }
                    }

                    let unpostedCount = 0;
                    let postSkipReason:
                        | 'missing-gl-account'
                        | 'no-unposted'
                        | null = null;
                    let postInspectError: string | null = null;
                    if (glAccount.length === 0) {
                        postSkipReason = 'missing-gl-account';
                    } else {
                        try {
                            const unposted = await getLoginAccountUnposted(
                                ledgerPath,
                                loginName,
                                label,
                            );
                            unpostedCount = unposted.length;
                            if (unpostedCount === 0) {
                                postSkipReason = 'no-unposted';
                            }
                        } catch (error) {
                            postInspectError = String(error);
                        }
                    }

                    return {
                        loginName,
                        label,
                        extract: {
                            eligible:
                                extractSkipReason === null &&
                                extractInspectError === null &&
                                documentCount > 0,
                            documentCount,
                            skipReason: extractSkipReason,
                            inspectError: extractInspectError,
                            locked,
                        },
                        post: {
                            eligible:
                                postSkipReason === null &&
                                postInspectError === null &&
                                unpostedCount > 0,
                            unpostedCount,
                            skipReason: postSkipReason,
                            inspectError: postInspectError,
                            locked,
                        },
                    } satisfies PipelineBulkAccountStat;
                }),
            );

            const extract = createEmptyPipelineBulkSummary();
            const post = createEmptyPipelineBulkSummary();
            for (const account of accountStats) {
                if (account.extract.eligible) {
                    extract.eligibleAccounts += 1;
                    extract.totalDocuments += account.extract.documentCount;
                }
                if (account.extract.skipReason === 'missing-extension') {
                    extract.skippedMissingExtension += 1;
                }
                if (account.extract.skipReason === 'no-documents') {
                    extract.skippedNoDocuments += 1;
                }
                if (account.extract.inspectError !== null) {
                    extract.inspectFailures += 1;
                }
                if (account.extract.locked) {
                    extract.lockedAccounts += 1;
                }

                if (account.post.eligible) {
                    post.eligibleAccounts += 1;
                    post.totalUnpostedEntries += account.post.unpostedCount;
                }
                if (account.post.skipReason === 'missing-gl-account') {
                    post.skippedMissingGlAccount += 1;
                }
                if (account.post.skipReason === 'no-unposted') {
                    post.skippedNoUnposted += 1;
                }
                if (account.post.inspectError !== null) {
                    post.inspectFailures += 1;
                }
                if (account.post.locked) {
                    post.lockedAccounts += 1;
                }
            }

            const stats = {
                accounts: accountStats,
                gl: snapshot.gl,
                extract,
                post,
            } satisfies PipelineBulkStats;
            setLockStatusSnapshot(snapshot);
            setPipelineBulkStats(stats);
            return stats;
        } finally {
            setIsLoadingPipelineBulkStats(false);
        }
    }, [ledgerPath, loginAccounts, loginConfigsByName]);
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

    const pipelineGlRows = useMemo(() => {
        if (!ledger || accountJournalEntries.length === 0) return [];
        const postedIds = new Set(
            accountJournalEntries
                .filter((e) => e.posted !== null)
                .map((e) => {
                    const parts = (e.posted ?? '').split(':');
                    return parts[parts.length - 1] ?? '';
                })
                .filter((id) => id.length > 0),
        );
        return ledger.transactions.filter((txn) => postedIds.has(txn.id));
    }, [ledger, accountJournalEntries]);

    const pipelineGlAccount = useMemo<string | null>(() => {
        if (!selectedLoginAccount) return null;
        return (
            loginConfigsByName[selectedLoginAccount.loginName]?.accounts[
                selectedLoginAccount.label
            ]?.glAccount ?? null
        );
    }, [selectedLoginAccount, loginConfigsByName]);
    const glLockStatus = lockStatusSnapshot?.gl ?? {
        locked: false,
        metadata: null,
    };
    const selectedLoginLockStatus =
        selectedLoginAccount === null
            ? null
            : (lockStatusSnapshot?.logins[selectedLoginAccount.loginName] ??
              null);
    const selectedLoginLocked = selectedLoginLockStatus?.locked ?? false;

    const visibleTransferResults = useMemo(() => {
        const q = transferModalSearch.trim().toLowerCase();
        if (!q) return transferModalResults;
        return transferModalResults.filter(
            (r) =>
                r.loginName.toLowerCase().includes(q) ||
                r.label.toLowerCase().includes(q) ||
                r.entry.description.toLowerCase().includes(q) ||
                r.entry.date.includes(q) ||
                (r.entry.amount ?? '').includes(q),
        );
    }, [transferModalResults, transferModalSearch]);

    const evidencedRowNumbers = useMemo(() => {
        if (!evidenceRowsDocument) return new Set<number>();
        const rowNumbers = new Set<number>();
        for (const entry of accountJournalEntries) {
            for (const evidence of entry.evidence) {
                const prefix = evidenceRowsDocument + ':';
                if (evidence.startsWith(prefix)) {
                    const rest = evidence.slice(prefix.length);
                    const rowNum = parseInt(rest.split(':')[0] ?? '', 10);
                    if (!isNaN(rowNum)) {
                        rowNumbers.add(rowNum);
                    }
                }
            }
        }
        return rowNumbers;
    }, [evidenceRowsDocument, accountJournalEntries]);

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
        if (ledgerPath !== null) {
            setTransactionDraft(createTransactionDraft());
            setRawDraft('');
            setAddStatus(null);
            setDraftStatus(null);
            setScrapeStatus(null);
            setScrapeDebugSocket(null);
            setScrapeAccount('');
            setLoginNames([]);
            setLoginConfigsByName({});
            setLoginManagementTab('select');
            setSelectedLoginName('');
            setSelectedLoginExtensionDraft('');
            setNewLoginName('');
            setNewLoginExtension('');
            setLoginLabelDraft('');
            setLoginGlAccountDraft('');
            setLoginConfigStatus(null);
            setIsLoadingLoginConfigs(false);
            setIsSavingLoginConfig(false);
            setHasLoadedLoginConfigs(false);
            setLoginConfigsReloadToken(0);
            setLoginAccountMappings({});
            setAccountSecrets([]);
            setRequiredSecretsForExtension([]);
            setHasRequiredSecretsSync(false);
            setSecretDomain('');
            setSecretName('');
            setSecretValue('');
            setIsSecretsPanelExpanded(false);
            setSecretsStatus(null);
            setIsLoadingAccountSecrets(false);
            setIsSavingAccountSecret(false);
            setBusySecretKey(null);
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnpostedEntries([]);
            setPipelineStatus(null);
            setPipelineSubTab('evidence');
            setEvidenceRowsDocument('');
            setDocumentRows([]);
            setIsLoadingDocumentRows(false);
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
            setQueryResults(null);
            setQueryError(null);
            setIsNewTxnExpandedOverride(null);
            setTransactionsSearch('');
            setAcSuggestions([]);
            setAcActiveIndex(-1);
        }
    }, [ledgerPath]);

    // Auto-select the first login account whenever the account list changes.
    useEffect(() => {
        if (loginAccounts.length === 0) return;
        setSelectedLoginAccount((current) => {
            if (
                current !== null &&
                loginAccounts.some(
                    (a) =>
                        a.loginName === current.loginName &&
                        a.label === current.label,
                )
            ) {
                return current;
            }
            return loginAccounts[0] ?? null;
        });
    }, [loginAccounts]);

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
    }, [activeSecretsLoginName, isSecretsPanelExpanded, ledgerPath]);

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
    }, [
        activeSecretsLoginName,
        isSecretsPanelExpanded,
        ledgerPath,
        scrapeExtension,
    ]);

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
                    const normalizedConfig = normalizeLoginConfig(config);
                    setScrapeExtension(
                        normalizedConfig.extension?.trim() ?? '',
                    );
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
            setUnpostedEntries([]);
            setPostDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
            return;
        }

        if (selectedLoginMapping === null) {
            setDocuments([]);
            setSelectedDocumentNames([]);
            setAccountJournalEntries([]);
            setUnpostedEntries([]);
            setPostDrafts({});
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
            return;
        }

        const mapping = selectedLoginMapping;
        let cancelled = false;
        const timer = window.setTimeout(() => {
            setIsLoadingDocuments(true);
            setIsLoadingAccountJournal(true);
            setIsLoadingUnposted(true);
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
                getLoginAccountUnposted(
                    ledgerPath,
                    mapping.loginName,
                    mapping.label,
                ),
            ])
                .then(([fetchedDocuments, fetchedJournal, fetchedUnposted]) => {
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
                                : selectedScrapeAccount,
                    }));
                })
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setDocuments([]);
                        setSelectedDocumentNames([]);
                        setAccountJournalEntries([]);
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
        setEvidenceRowsDocument('');
        setDocumentRows([]);
        setPipelineSelectedEntryIds(new Set());
        setPipelineCategorySuggestions({});
        setPipelineGlAccountDraft(
            selectedLoginAccount
                ? suggestGlAccountName(selectedLoginAccount.label)
                : '',
        );
        setTransferModalEntryId(null);
    }, [selectedLoginAccount]);

    useEffect(() => {
        if (ledgerPath === null) {
            setLockStatusSnapshot(null);
            return;
        }

        let cancelled = false;
        let unlisten: (() => void) | null = null;

        void startLockMetadataWatch(ledgerPath)
            .then(() =>
                listen('refreshmint://lock-status-changed', () => {
                    if (!cancelled) {
                        void refreshPipelineBulkStats();
                    }
                }),
            )
            .then((listener) => {
                if (!cancelled) {
                    unlisten = listener;
                } else {
                    listener();
                }
            })
            .catch((error: unknown) => {
                console.error('lock metadata watcher failed:', error);
            });

        return () => {
            cancelled = true;
            if (unlisten !== null) {
                unlisten();
            }
            void stopLockMetadataWatch();
        };
    }, [ledgerPath, refreshPipelineBulkStats]);

    useEffect(() => {
        if (
            activeTab !== 'pipeline' ||
            ledgerPath === null ||
            !hasLoadedLoginConfigs
        ) {
            return;
        }
        void refreshPipelineBulkStats().catch((error: unknown) => {
            console.error('pipeline bulk stats failed:', error);
        });
    }, [
        activeTab,
        hasLoadedLoginConfigs,
        ledgerPath,
        loginAccounts,
        loginConfigsByName,
        refreshPipelineBulkStats,
    ]);

    useEffect(() => {
        if (ledgerPath === null || selectedLoginAccount === null) {
            return;
        }

        const { loginName, label } = selectedLoginAccount;
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
                .then(([fetchedDocuments, fetchedJournal, fetchedUnposted]) => {
                    if (cancelled) {
                        return;
                    }
                    setDocuments(fetchedDocuments);
                    setAccountJournalEntries(fetchedJournal);
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
                    // Non-blocking category suggestions (fail-open)
                    const reqId = ++suggestRequestId.current;
                    suggestCategories(ledgerPath, loginName, label)
                        .then((result) => {
                            if (
                                !cancelled &&
                                reqId === suggestRequestId.current
                            ) {
                                setPipelineCategorySuggestions(result);
                            }
                        })
                        .catch((err: unknown) => {
                            console.error('suggestCategories failed:', err);
                        });
                })
                .catch((error: unknown) => {
                    if (!cancelled) {
                        setPipelineStatus(
                            `Failed to load login account journal: ${String(error)}`,
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
        }, 200);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [ledgerPath, selectedLoginAccount]);

    useEffect(() => {
        if (!ledger) {
            return;
        }
        void setLastActiveTab(activeTab);
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
            setExtensionLoadStatus('Extension load canceled.');
            return;
        }

        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            const msg = selectedLoginMappingError ?? 'Select a login first.';
            setScrapeStatus(msg);
            setExtensionLoadStatus(msg);
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
        if (!ledger) {
            return;
        }
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus(selectedLoginMappingError ?? 'Login is required.');
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
            setLoginManagementTab('select');
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
            await deleteLoginAccount(ledger.path, loginName, label);
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
            setUnpostedEntries([]);
            setPostDrafts({});
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
        setIsLoadingUnposted(true);
        try {
            const [fetchedDocuments, fetchedJournal, fetchedUnposted] =
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
                    getLoginAccountUnposted(
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
                        : account,
            }));
        } finally {
            setIsLoadingDocuments(false);
            setIsLoadingAccountJournal(false);
            setIsLoadingUnposted(false);
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

    async function handleLoadDocumentRows(documentName: string) {
        if (!ledger || selectedLoginAccount === null) return;
        setEvidenceRowsDocument(documentName);
        if (!documentName) {
            setDocumentRows([]);
            return;
        }
        setIsLoadingDocumentRows(true);
        try {
            const rows = await readLoginAccountDocumentRows(
                ledger.path,
                selectedLoginAccount.loginName,
                selectedLoginAccount.label,
                documentName,
            );
            setDocumentRows(rows);
        } catch {
            setDocumentRows([]);
        } finally {
            setIsLoadingDocumentRows(false);
        }
    }

    async function handlePipelineExtraction(documentName: string) {
        if (!ledger || selectedLoginAccount === null || !documentName) return;
        const { loginName, label } = selectedLoginAccount;
        setIsRunningExtraction(true);
        setPipelineStatus(`Running extraction for ${documentName}...`);
        try {
            const newCount = await runLoginAccountExtraction(
                ledger.path,
                loginName,
                label,
                [documentName],
            );
            const [journal, unposted] = await Promise.all([
                getLoginAccountJournal(ledger.path, loginName, label),
                getLoginAccountUnposted(ledger.path, loginName, label),
            ]);
            setAccountJournalEntries(journal);
            setUnpostedEntries(unposted);
            setPostDrafts((current) => {
                const next: Record<string, PostDraft> = {};
                for (const entry of unposted) {
                    next[entry.id] = current[entry.id] ?? {
                        counterpartAccount: '',
                        postingIndex: '',
                    };
                }
                return next;
            });
            setPipelineStatus(
                `Extraction complete. ${newCount} new entr${newCount === 1 ? 'y' : 'ies'} added.`,
            );
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Extraction failed: ${String(error)}`);
        } finally {
            setIsRunningExtraction(false);
        }
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
                mapping.loginName,
                mapping.label,
                entryId,
                counterpartAccount,
                postingIndex.value,
            );
            await refreshAccountPipelineData(account);
            setUnpostEntryId(entryId);
            setPipelineStatus(`Posted ${entryId} to ${glId}.`);
        } catch (error) {
            setPipelineStatus(`Post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handleUnpostAccountEntry() {
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
                mapping.loginName,
                mapping.label,
                entryId,
                postingIndex.value,
            );
            await refreshAccountPipelineData(account);
            setPipelineStatus(`Unposted ${entryId}.`);
        } catch (error) {
            setPipelineStatus(`Unpost failed: ${String(error)}`);
        } finally {
            setIsUnpostingEntry(false);
        }
    }

    async function handlePostTransferPair() {
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

        setIsPostingTransfer(true);
        try {
            const glId = await postTransfer(
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
                `Transfer posting complete: ${entryId1} ↔ ${entryId2} (${glId}).`,
            );
        } catch (error) {
            setPipelineStatus(`Transfer post failed: ${String(error)}`);
        } finally {
            setIsPostingTransfer(false);
        }
    }

    /** Reload journal + unposted data for the current pipeline login/label. */
    async function refreshPipelineLoginAccountData() {
        if (!ledger || !selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        const [fetchedJournal, fetchedUnposted] = await Promise.all([
            getLoginAccountJournal(ledger.path, loginName, label),
            getLoginAccountUnposted(ledger.path, loginName, label),
        ]);
        setAccountJournalEntries(fetchedJournal);
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
        // Reload full ledger so Transactions tab and GL Rows stay in sync.
        openLedger(ledger.path)
            .then((reloaded) => {
                setLedger(reloaded);
            })
            .catch((err: unknown) => {
                console.error('ledger reload failed:', err);
            });
        // Non-blocking re-run of suggestCategories to refresh mismatch flags
        const reqId = ++suggestRequestId.current;
        suggestCategories(ledger.path, loginName, label)
            .then((result) => {
                if (reqId === suggestRequestId.current) {
                    setPipelineCategorySuggestions(result);
                }
            })
            .catch((err: unknown) => {
                console.error('suggestCategories failed:', err);
            });
    }

    async function handleRecategorizeGlTransaction(
        txnId: string,
        newAccount: string,
    ) {
        if (!ledger) return;
        try {
            await recategorizeGlTransaction(ledger.path, txnId, newAccount);
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

    async function handleSavePipelineGlAccount() {
        if (!ledger || !selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        const glAccount = pipelineGlAccountDraft.trim();
        setIsSavingPipelineGlAccount(true);
        try {
            await setLoginAccount(
                ledger.path,
                loginName,
                label,
                glAccount.length === 0 ? null : glAccount,
            );
            requestLoginConfigReload();
            setPipelineGlAccountDraft('');
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Failed to save GL account: ${String(error)}`);
        } finally {
            setIsSavingPipelineGlAccount(false);
        }
    }

    async function doPipelinePostForAccount(
        loginName: string,
        label: string,
        entryId: string,
        suggestions: Record<string, CategoryResult>,
    ): Promise<string> {
        if (!ledger) {
            throw new Error('No ledger selected.');
        }
        const suggestion = suggestions[entryId];

        if (suggestion?.transferMatch) {
            const locator = suggestion.transferMatch.accountLocator;
            const parts = locator.split('/');
            const otherLoginName = parts[1] ?? '';
            const otherLabel = parts[3] ?? '';
            if (!otherLoginName || !otherLabel) {
                throw new Error(
                    `Transfer: could not parse locator: ${locator}`,
                );
            }
            const glId = await postLoginAccountTransfer(
                ledger.path,
                loginName,
                label,
                entryId,
                otherLoginName,
                otherLabel,
                suggestion.transferMatch.entryId,
            );
            return `Transfer posted: ${entryId} ↔ ${suggestion.transferMatch.entryId} (${glId})`;
        }

        const glId = await postLoginAccountEntry(
            ledger.path,
            loginName,
            label,
            entryId,
            'Expenses:Unknown',
            null,
        );
        return `Posted ${entryId} to ${glId}`;
    }

    /**
     * Core posting logic that throws on error. Used by both the single-entry
     * handler (which catches) and the bulk handlers.
     */
    async function doPipelinePost(entryId: string): Promise<string> {
        if (!selectedLoginAccount) {
            throw new Error('No account selected.');
        }
        return doPipelinePostForAccount(
            selectedLoginAccount.loginName,
            selectedLoginAccount.label,
            entryId,
            pipelineCategorySuggestions,
        );
    }

    async function handlePipelinePostEntry(entryId: string) {
        setBusyPostEntryId(entryId);
        try {
            const message = await doPipelinePost(entryId);
            await refreshPipelineLoginAccountData();
            setPipelineStatus(message);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handlePipelinePostAll() {
        if (!ledger || !selectedLoginAccount) return;
        setIsPipelinePosting(true);
        try {
            for (const entry of unpostedEntries) {
                setBusyPostEntryId(entry.id);
                try {
                    const message = await doPipelinePost(entry.id);
                    await refreshPipelineLoginAccountData();
                    setPipelineStatus(message);
                } catch (error) {
                    setPipelineStatus(`Post failed: ${String(error)}`);
                    break; // stop-on-first-error
                } finally {
                    setBusyPostEntryId(null);
                }
            }
        } finally {
            setIsPipelinePosting(false);
            void refreshPipelineBulkStats();
        }
    }

    async function handlePipelinePostSelected() {
        if (!ledger || !selectedLoginAccount) return;
        setIsPipelinePosting(true);
        try {
            for (const entry of unpostedEntries) {
                if (!pipelineSelectedEntryIds.has(entry.id)) continue;
                setBusyPostEntryId(entry.id);
                try {
                    const message = await doPipelinePost(entry.id);
                    await refreshPipelineLoginAccountData();
                    setPipelineStatus(message);
                } catch (error) {
                    setPipelineStatus(`Post failed: ${String(error)}`);
                    break; // stop-on-first-error
                } finally {
                    setBusyPostEntryId(null);
                }
            }
        } finally {
            setIsPipelinePosting(false);
            setPipelineSelectedEntryIds(new Set());
            void refreshPipelineBulkStats();
        }
    }

    async function handlePipelineExtractAllLedger() {
        if (!ledger) return;
        setIsPipelineExtractingAllLedger(true);
        try {
            const stats =
                pipelineBulkStats ?? (await refreshPipelineBulkStats());
            if (!stats) return;
            const candidates = stats.accounts.filter(
                (account) =>
                    account.extract.eligible && !account.extract.locked,
            );
            if (candidates.length === 0) {
                setPipelineStatus(
                    'No unlocked accounts are ready for extraction.',
                );
                return;
            }

            let succeeded = 0;
            let failed = 0;
            let locked = 0;
            let totalNew = 0;
            let refreshSelected = false;
            const selectedKey =
                selectedLoginAccount === null
                    ? null
                    : `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`;

            for (const [index, account] of candidates.entries()) {
                const accountKey = `${account.loginName}/${account.label}`;
                setPipelineStatus(
                    `Extract All: ${accountKey} (${index + 1}/${candidates.length})`,
                );
                try {
                    const docs = await listLoginAccountDocuments(
                        ledger.path,
                        account.loginName,
                        account.label,
                    );
                    if (docs.length === 0) {
                        continue;
                    }
                    const newCount = await runLoginAccountExtraction(
                        ledger.path,
                        account.loginName,
                        account.label,
                        docs.map((doc) => doc.filename),
                    );
                    totalNew += newCount;
                    succeeded += 1;
                    if (selectedKey === accountKey) {
                        refreshSelected = true;
                    }
                } catch (error) {
                    if (String(error).includes('currently in use')) {
                        locked += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            if (
                refreshSelected &&
                selectedLoginAccount !== null &&
                selectedKey ===
                    `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
            ) {
                await refreshPipelineLoginAccountData();
            }
            await refreshPipelineBulkStats();
            setPipelineStatus(
                `Extract All complete. ${succeeded} account(s) extracted, ${failed} failed, ${locked} locked, ${totalNew} new entr${totalNew === 1 ? 'y' : 'ies'} added.`,
            );
        } finally {
            setIsPipelineExtractingAllLedger(false);
        }
    }

    async function handlePipelinePostAllLedger() {
        if (!ledger) return;
        setIsPipelinePostingAllLedger(true);
        try {
            const stats =
                pipelineBulkStats ?? (await refreshPipelineBulkStats());
            if (!stats) return;
            if (stats.gl.locked) {
                setPipelineStatus('General journal is currently locked.');
                return;
            }
            const candidates = stats.accounts.filter(
                (account) => account.post.eligible && !account.post.locked,
            );
            if (candidates.length === 0) {
                setPipelineStatus(
                    'No unlocked accounts are ready for posting.',
                );
                return;
            }

            let postedCount = 0;
            let failed = 0;
            let locked = 0;
            let refreshSelected = false;
            let reloadLedgerAfter = false;
            const selectedKey =
                selectedLoginAccount === null
                    ? null
                    : `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`;

            for (const [index, account] of candidates.entries()) {
                const accountKey = `${account.loginName}/${account.label}`;
                setPipelineStatus(
                    `Post All: ${accountKey} (${index + 1}/${candidates.length})`,
                );
                try {
                    const [unposted, suggestions] = await Promise.all([
                        getLoginAccountUnposted(
                            ledger.path,
                            account.loginName,
                            account.label,
                        ),
                        suggestCategories(
                            ledger.path,
                            account.loginName,
                            account.label,
                        ),
                    ]);
                    for (const entry of unposted) {
                        try {
                            await doPipelinePostForAccount(
                                account.loginName,
                                account.label,
                                entry.id,
                                suggestions,
                            );
                            postedCount += 1;
                            reloadLedgerAfter = true;
                        } catch (error) {
                            if (String(error).includes('currently in use')) {
                                locked += 1;
                            } else {
                                failed += 1;
                            }
                        }
                    }
                    if (selectedKey === accountKey) {
                        refreshSelected = true;
                    }
                } catch (error) {
                    if (String(error).includes('currently in use')) {
                        locked += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            if (reloadLedgerAfter) {
                const reloaded = await openLedger(ledger.path);
                setLedger(reloaded);
            }
            if (
                refreshSelected &&
                selectedLoginAccount !== null &&
                selectedKey ===
                    `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
            ) {
                await refreshPipelineLoginAccountData();
            }
            await refreshPipelineBulkStats();
            setPipelineStatus(
                `Post All complete. ${postedCount} entr${postedCount === 1 ? 'y' : 'ies'} posted, ${failed} failed, ${locked} locked.`,
            );
        } finally {
            setIsPipelinePostingAllLedger(false);
        }
    }

    async function handlePipelineSyncEntry(entryId: string) {
        if (!ledger || !selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(entryId);
        try {
            const glId = await syncGlTransaction(
                ledger.path,
                loginName,
                label,
                entryId,
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(`Synced ${entryId} → ${glId}`);
            void refreshPipelineBulkStats();
        } catch (error) {
            setPipelineStatus(`Sync failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
        }
    }

    async function handleOpenTransferModal(entryId: string) {
        if (!ledger || !selectedLoginAccount) return;
        const { loginName, label } = selectedLoginAccount;
        setTransferModalEntryId(entryId);
        setTransferModalSearch('');
        setIsLoadingTransferModal(true);
        try {
            const results = await getUnpostedEntriesForTransfer(
                ledger.path,
                loginName,
                label,
                entryId,
            );
            setTransferModalResults(results);
        } catch (error) {
            setPipelineStatus(
                `Failed to load transfer candidates: ${String(error)}`,
            );
            setTransferModalEntryId(null);
        } finally {
            setIsLoadingTransferModal(false);
        }
    }

    async function handleLinkTransferFromModal(other: UnpostedTransferResult) {
        if (!ledger || !selectedLoginAccount || transferModalEntryId === null)
            return;
        const { loginName, label } = selectedLoginAccount;
        setBusyPostEntryId(transferModalEntryId);
        setTransferModalEntryId(null);
        try {
            const glId = await postLoginAccountTransfer(
                ledger.path,
                loginName,
                label,
                transferModalEntryId,
                other.loginName,
                other.label,
                other.entry.id,
            );
            await refreshPipelineLoginAccountData();
            setPipelineStatus(
                `Transfer posted: ${transferModalEntryId} ↔ ${other.entry.id} (${glId})`,
            );
        } catch (error) {
            setPipelineStatus(`Transfer post failed: ${String(error)}`);
        } finally {
            setBusyPostEntryId(null);
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

        setIsRunningScrape(true);
        setScrapeStatus(`Running scrape for ${loginName}...`);
        try {
            await runScrapeForLogin(ledger.path, loginName);
            setScrapeStatus(`Scrape completed for ${loginName}.`);
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
                            <div className="table-wrap">
                                <TransactionsTable
                                    transactions={filteredTransactions}
                                    ledgerPath={ledgerPath}
                                    glCategorySuggestions={
                                        glCategorySuggestions
                                    }
                                    onRecategorize={(txnId, newAccount) => {
                                        void handleRecategorizeGlTransaction(
                                            txnId,
                                            newAccount,
                                        );
                                    }}
                                    onMergeTransfer={(txnId1, txnId2) => {
                                        void handleMergeGlTransfer(
                                            txnId1,
                                            txnId2,
                                        );
                                    }}
                                />
                            </div>
                        </div>
                    ) : activeTab === 'pipeline' ? (
                        <div className="transactions-panel">
                            <section className="txn-form">
                                <div className="txn-form-header">
                                    <div>
                                        <h2>Pipeline</h2>
                                        <p>
                                            Inspect the ETL stages for a bank
                                            account: evidence documents, raw
                                            rows, extracted account entries, and
                                            posted GL transactions.
                                        </p>
                                    </div>
                                </div>
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Bank account</span>
                                        <select
                                            value={
                                                selectedLoginAccount !== null
                                                    ? `${selectedLoginAccount.loginName}/${selectedLoginAccount.label}`
                                                    : ''
                                            }
                                            disabled={
                                                isPipelineExtractingAllLedger ||
                                                isPipelinePostingAllLedger
                                            }
                                            onChange={(event) => {
                                                const value =
                                                    event.target.value;
                                                if (!value) {
                                                    setSelectedLoginAccount(
                                                        null,
                                                    );
                                                    return;
                                                }
                                                const [loginName, label] =
                                                    value.split('/', 2);
                                                if (
                                                    loginName !== undefined &&
                                                    label !== undefined &&
                                                    loginName.length > 0 &&
                                                    label.length > 0
                                                ) {
                                                    setSelectedLoginAccount({
                                                        loginName,
                                                        label,
                                                    });
                                                }
                                            }}
                                        >
                                            <option value="">
                                                Select a source...
                                            </option>
                                            {loginAccounts.map((account) => {
                                                const key = `${account.loginName}/${account.label}`;
                                                return (
                                                    <option
                                                        key={key}
                                                        value={key}
                                                    >
                                                        {key}
                                                    </option>
                                                );
                                            })}
                                        </select>
                                    </label>
                                </div>
                                <div className="pipeline-panel">
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="primary-button"
                                            disabled={
                                                isLoadingPipelineBulkStats ||
                                                isPipelineExtractingAllLedger ||
                                                (pipelineBulkStats?.extract
                                                    .eligibleAccounts ?? 0) ===
                                                    0
                                            }
                                            onClick={() => {
                                                void handlePipelineExtractAllLedger();
                                            }}
                                        >
                                            {isPipelineExtractingAllLedger
                                                ? 'Extracting...'
                                                : `Extract All (${pipelineBulkStats?.extract.totalDocuments ?? 0})`}
                                        </button>
                                        <button
                                            type="button"
                                            className="primary-button"
                                            disabled={
                                                isLoadingPipelineBulkStats ||
                                                isPipelinePostingAllLedger ||
                                                glLockStatus.locked ||
                                                (pipelineBulkStats?.post
                                                    .eligibleAccounts ?? 0) ===
                                                    0
                                            }
                                            onClick={() => {
                                                void handlePipelinePostAllLedger();
                                            }}
                                        >
                                            {isPipelinePostingAllLedger
                                                ? 'Posting...'
                                                : `Post All (${pipelineBulkStats?.post.totalUnpostedEntries ?? 0})`}
                                        </button>
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            disabled={
                                                isLoadingPipelineBulkStats
                                            }
                                            onClick={() => {
                                                void refreshPipelineBulkStats();
                                            }}
                                        >
                                            {isLoadingPipelineBulkStats
                                                ? 'Refreshing stats...'
                                                : 'Refresh Stats'}
                                        </button>
                                    </div>
                                    {pipelineBulkStats !== null && (
                                        <div className="hint">
                                            <div>
                                                {`Extract All: ${pipelineBulkStats.extract.totalDocuments} document(s) across ${pipelineBulkStats.extract.eligibleAccounts} eligible account(s).`}
                                            </div>
                                            <div>
                                                {`${pipelineBulkStats.extract.skippedMissingExtension} missing extension, ${pipelineBulkStats.extract.skippedNoDocuments} no documents, ${pipelineBulkStats.extract.inspectFailures} inspect failures, ${pipelineBulkStats.extract.lockedAccounts} locked.`}
                                            </div>
                                            <div>
                                                {`Post All: ${pipelineBulkStats.post.totalUnpostedEntries} entr${pipelineBulkStats.post.totalUnpostedEntries === 1 ? 'y' : 'ies'} across ${pipelineBulkStats.post.eligibleAccounts} eligible account(s).`}
                                            </div>
                                            <div>
                                                {`${pipelineBulkStats.post.skippedMissingGlAccount} missing GL mapping, ${pipelineBulkStats.post.skippedNoUnposted} no unposted, ${pipelineBulkStats.post.inspectFailures} inspect failures, ${pipelineBulkStats.post.lockedAccounts} locked, GL ${pipelineBulkStats.gl.locked ? 'locked' : 'unlocked'}.`}
                                            </div>
                                        </div>
                                    )}
                                    {selectedLoginLockStatus?.locked ===
                                        true && (
                                        <p className="status">
                                            {selectedLoginLockStatus.metadata ===
                                            null
                                                ? 'Selected login is currently in use by another operation.'
                                                : `Selected login locked by ${selectedLoginLockStatus.metadata.owner}/${selectedLoginLockStatus.metadata.purpose}.`}
                                        </p>
                                    )}
                                    {glLockStatus.locked && (
                                        <p className="status">
                                            {glLockStatus.metadata === null
                                                ? 'General journal is currently in use by another operation.'
                                                : `General journal locked by ${glLockStatus.metadata.owner}/${glLockStatus.metadata.purpose}.`}
                                        </p>
                                    )}
                                </div>
                                <div className="tabs pipeline-subtabs">
                                    <button
                                        className={
                                            pipelineSubTab === 'evidence'
                                                ? 'tab active'
                                                : 'tab'
                                        }
                                        onClick={() => {
                                            setPipelineSubTab('evidence');
                                        }}
                                        type="button"
                                    >
                                        Evidence
                                    </button>
                                    <button
                                        className={
                                            pipelineSubTab === 'evidence-rows'
                                                ? 'tab active'
                                                : 'tab'
                                        }
                                        onClick={() => {
                                            setPipelineSubTab('evidence-rows');
                                        }}
                                        type="button"
                                    >
                                        Evidence Rows
                                    </button>
                                    <button
                                        className={
                                            pipelineSubTab === 'account-rows'
                                                ? 'tab active'
                                                : 'tab'
                                        }
                                        onClick={() => {
                                            setPipelineSubTab('account-rows');
                                        }}
                                        type="button"
                                    >
                                        Account Rows
                                    </button>
                                    <button
                                        className={
                                            pipelineSubTab === 'gl-rows'
                                                ? 'tab active'
                                                : 'tab'
                                        }
                                        onClick={() => {
                                            setPipelineSubTab('gl-rows');
                                        }}
                                        type="button"
                                    >
                                        GL Rows
                                    </button>
                                </div>
                                {selectedLoginAccount === null ? (
                                    <p className="hint">
                                        Select a bank account to inspect its
                                        pipeline stages.
                                    </p>
                                ) : pipelineSubTab === 'evidence' ? (
                                    isLoadingDocuments ? (
                                        <p className="status">
                                            Loading documents...
                                        </p>
                                    ) : documents.length === 0 ? (
                                        <p className="hint">
                                            No documents found for this account.
                                        </p>
                                    ) : (
                                        <div className="table-wrap">
                                            <table className="ledger-table">
                                                <thead>
                                                    <tr>
                                                        <th>Document</th>
                                                        <th>Type</th>
                                                        <th>Coverage End</th>
                                                        <th>Scraped At</th>
                                                        <th>Scrape Session</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {documents.map((doc) => (
                                                        <tr key={doc.filename}>
                                                            <td className="mono">
                                                                {doc.filename}
                                                            </td>
                                                            <td className="mono">
                                                                {doc.info
                                                                    ?.mimeType ??
                                                                    '-'}
                                                            </td>
                                                            <td className="mono">
                                                                {doc.info
                                                                    ?.coverageEndDate ??
                                                                    '-'}
                                                            </td>
                                                            <td className="mono">
                                                                {doc.info
                                                                    ?.scrapedAt ??
                                                                    '-'}
                                                            </td>
                                                            <td className="mono">
                                                                {doc.info
                                                                    ?.scrapeSessionId ??
                                                                    '-'}
                                                            </td>
                                                        </tr>
                                                    ))}
                                                </tbody>
                                            </table>
                                        </div>
                                    )
                                ) : pipelineSubTab === 'evidence-rows' ? (
                                    <>
                                        <div className="txn-grid">
                                            <label className="field">
                                                <span>Document</span>
                                                <select
                                                    value={evidenceRowsDocument}
                                                    onChange={(e) => {
                                                        void handleLoadDocumentRows(
                                                            e.target.value,
                                                        );
                                                    }}
                                                >
                                                    <option value="">
                                                        Select a document...
                                                    </option>
                                                    {documents
                                                        .filter((d) =>
                                                            d.filename
                                                                .toLowerCase()
                                                                .endsWith(
                                                                    '.csv',
                                                                ),
                                                        )
                                                        .map((d) => (
                                                            <option
                                                                key={d.filename}
                                                                value={
                                                                    d.filename
                                                                }
                                                            >
                                                                {d.filename}
                                                            </option>
                                                        ))}
                                                </select>
                                            </label>
                                            <div className="field">
                                                <span />
                                                <button
                                                    type="button"
                                                    className="primary-button"
                                                    onClick={() => {
                                                        void handlePipelineExtraction(
                                                            evidenceRowsDocument,
                                                        );
                                                    }}
                                                    disabled={
                                                        !evidenceRowsDocument ||
                                                        isRunningExtraction ||
                                                        !hasExtension ||
                                                        selectedLoginLocked ||
                                                        isPipelineExtractingAllLedger
                                                    }
                                                >
                                                    {isRunningExtraction
                                                        ? 'Extracting...'
                                                        : evidencedRowNumbers.size >
                                                            0
                                                          ? 'Re-extract document → account rows'
                                                          : 'Extract document → account rows'}
                                                </button>
                                                {!hasExtension && (
                                                    <span className="hint">
                                                        No extension bound — set
                                                        one in Scraping.
                                                    </span>
                                                )}
                                            </div>
                                        </div>
                                        {evidenceRowsDocument === '' ? (
                                            <p className="hint">
                                                Select a CSV document to view
                                                its raw rows.
                                            </p>
                                        ) : isLoadingDocumentRows ? (
                                            <p className="status">
                                                Loading rows...
                                            </p>
                                        ) : documentRows.length === 0 ? (
                                            <p className="hint">
                                                No rows found.
                                            </p>
                                        ) : (
                                            <div className="table-wrap">
                                                <table className="ledger-table">
                                                    <tbody>
                                                        {documentRows.map(
                                                            (row, rowIndex) => {
                                                                const rowNum =
                                                                    rowIndex +
                                                                    1;
                                                                const isHeader =
                                                                    rowIndex ===
                                                                    0;
                                                                const isEvidenced =
                                                                    evidencedRowNumbers.has(
                                                                        rowNum,
                                                                    );
                                                                return (
                                                                    <tr
                                                                        key={
                                                                            rowIndex
                                                                        }
                                                                        className={
                                                                            isHeader
                                                                                ? 'evidence-header-row'
                                                                                : isEvidenced
                                                                                  ? 'evidence-highlighted-row'
                                                                                  : undefined
                                                                        }
                                                                    >
                                                                        <td className="mono">
                                                                            {
                                                                                rowNum
                                                                            }
                                                                        </td>
                                                                        {row.map(
                                                                            (
                                                                                cell,
                                                                                colIndex,
                                                                            ) => (
                                                                                <td
                                                                                    key={
                                                                                        colIndex
                                                                                    }
                                                                                    className="mono"
                                                                                >
                                                                                    {
                                                                                        cell
                                                                                    }
                                                                                </td>
                                                                            ),
                                                                        )}
                                                                    </tr>
                                                                );
                                                            },
                                                        )}
                                                    </tbody>
                                                </table>
                                            </div>
                                        )}
                                    </>
                                ) : pipelineSubTab === 'account-rows' ? (
                                    isLoadingAccountJournal ? (
                                        <p className="status">
                                            Loading account rows...
                                        </p>
                                    ) : (
                                        <div className="account-rows-panel">
                                            <div className="pipeline-panel">
                                                <div className="pipeline-actions pipeline-gl-account-row">
                                                    <span className="pipeline-gl-account-label">
                                                        GL Account:
                                                    </span>
                                                    {pipelineGlAccount !==
                                                        null &&
                                                    pipelineGlAccount !== '' ? (
                                                        <span className="mono">
                                                            {pipelineGlAccount}
                                                        </span>
                                                    ) : (
                                                        <>
                                                            <input
                                                                type="text"
                                                                placeholder={suggestGlAccountName(
                                                                    selectedLoginAccount.label,
                                                                )}
                                                                value={
                                                                    pipelineGlAccountDraft
                                                                }
                                                                onChange={(
                                                                    e,
                                                                ) => {
                                                                    setPipelineGlAccountDraft(
                                                                        e.target
                                                                            .value,
                                                                    );
                                                                }}
                                                            />
                                                            <button
                                                                type="button"
                                                                className="primary-button"
                                                                disabled={
                                                                    isSavingPipelineGlAccount ||
                                                                    selectedLoginLocked ||
                                                                    pipelineGlAccountDraft.trim()
                                                                        .length ===
                                                                        0
                                                                }
                                                                onClick={() => {
                                                                    void handleSavePipelineGlAccount();
                                                                }}
                                                            >
                                                                {isSavingPipelineGlAccount
                                                                    ? 'Saving...'
                                                                    : 'Save GL Account'}
                                                            </button>
                                                        </>
                                                    )}
                                                </div>
                                                <div className="pipeline-actions">
                                                    <button
                                                        type="button"
                                                        className="primary-button"
                                                        disabled={
                                                            isPipelinePosting ||
                                                            isPipelinePostingAllLedger ||
                                                            glLockStatus.locked ||
                                                            selectedLoginLocked ||
                                                            unpostedEntries.length ===
                                                                0
                                                        }
                                                        onClick={() => {
                                                            void handlePipelinePostAll();
                                                        }}
                                                    >
                                                        {isPipelinePosting
                                                            ? 'Posting...'
                                                            : `Post All (${unpostedEntries.length.toString()})`}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        className="ghost-button"
                                                        disabled={
                                                            isPipelinePosting ||
                                                            isPipelinePostingAllLedger ||
                                                            glLockStatus.locked ||
                                                            selectedLoginLocked ||
                                                            pipelineSelectedEntryIds.size ===
                                                                0
                                                        }
                                                        onClick={() => {
                                                            void handlePipelinePostSelected();
                                                        }}
                                                    >
                                                        {`Post Selected (${pipelineSelectedEntryIds.size.toString()})`}
                                                    </button>
                                                </div>
                                            </div>
                                            <div className="table-wrap">
                                                <table className="ledger-table">
                                                    <thead>
                                                        <tr>
                                                            <th></th>
                                                            <th>Date</th>
                                                            <th>Description</th>
                                                            <th>Amount</th>
                                                            <th>Status</th>
                                                            <th>Actions</th>
                                                        </tr>
                                                    </thead>
                                                    <tbody>
                                                        {accountJournalEntries.length ===
                                                        0 ? (
                                                            <tr>
                                                                <td
                                                                    colSpan={6}
                                                                    className="table-empty"
                                                                >
                                                                    No entries
                                                                    found.
                                                                </td>
                                                            </tr>
                                                        ) : (
                                                            accountJournalEntries.map(
                                                                (entry) => {
                                                                    const suggestion =
                                                                        pipelineCategorySuggestions[
                                                                            entry
                                                                                .id
                                                                        ];
                                                                    const amountChanged =
                                                                        suggestion?.amountChanged ??
                                                                        false;
                                                                    const statusChanged =
                                                                        suggestion?.statusChanged ??
                                                                        false;
                                                                    const needsSync =
                                                                        entry.posted !==
                                                                            null &&
                                                                        (amountChanged ||
                                                                            statusChanged);
                                                                    const isUnposted =
                                                                        entry.posted ===
                                                                        null;
                                                                    const transferMatch =
                                                                        suggestion?.transferMatch ??
                                                                        null;
                                                                    const isBusy =
                                                                        busyPostEntryId ===
                                                                        entry.id;
                                                                    const isSelected =
                                                                        pipelineSelectedEntryIds.has(
                                                                            entry.id,
                                                                        );
                                                                    return (
                                                                        <tr
                                                                            key={
                                                                                entry.id
                                                                            }
                                                                            className={
                                                                                needsSync
                                                                                    ? 'row-needs-sync'
                                                                                    : undefined
                                                                            }
                                                                        >
                                                                            <td>
                                                                                {isUnposted && (
                                                                                    <input
                                                                                        type="checkbox"
                                                                                        checked={
                                                                                            isSelected
                                                                                        }
                                                                                        onChange={(
                                                                                            e,
                                                                                        ) => {
                                                                                            setPipelineSelectedEntryIds(
                                                                                                (
                                                                                                    current,
                                                                                                ) => {
                                                                                                    const next =
                                                                                                        new Set(
                                                                                                            current,
                                                                                                        );
                                                                                                    if (
                                                                                                        e
                                                                                                            .target
                                                                                                            .checked
                                                                                                    )
                                                                                                        next.add(
                                                                                                            entry.id,
                                                                                                        );
                                                                                                    else
                                                                                                        next.delete(
                                                                                                            entry.id,
                                                                                                        );
                                                                                                    return next;
                                                                                                },
                                                                                            );
                                                                                        }}
                                                                                    />
                                                                                )}
                                                                            </td>
                                                                            <td className="mono">
                                                                                {
                                                                                    entry.date
                                                                                }
                                                                            </td>
                                                                            <td>
                                                                                {
                                                                                    entry.description
                                                                                }
                                                                            </td>
                                                                            <td className="mono">
                                                                                {entry.amount ??
                                                                                    '-'}
                                                                            </td>
                                                                            <td>
                                                                                {isUnposted ? (
                                                                                    <span className="status-chip">
                                                                                        unposted
                                                                                    </span>
                                                                                ) : needsSync ? (
                                                                                    <span className="status-chip status-chip-warning">
                                                                                        ⚠
                                                                                        needs
                                                                                        sync
                                                                                    </span>
                                                                                ) : (
                                                                                    <span className="status-chip status-chip-ok">
                                                                                        posted
                                                                                    </span>
                                                                                )}
                                                                            </td>
                                                                            <td>
                                                                                <div className="pipeline-row-actions">
                                                                                    {isUnposted ? (
                                                                                        <>
                                                                                            <button
                                                                                                type="button"
                                                                                                className="primary-button"
                                                                                                disabled={
                                                                                                    isBusy ||
                                                                                                    isPipelinePosting ||
                                                                                                    isPipelinePostingAllLedger ||
                                                                                                    glLockStatus.locked ||
                                                                                                    selectedLoginLocked
                                                                                                }
                                                                                                onClick={() => {
                                                                                                    void handlePipelinePostEntry(
                                                                                                        entry.id,
                                                                                                    );
                                                                                                }}
                                                                                            >
                                                                                                {isBusy
                                                                                                    ? 'Posting...'
                                                                                                    : 'Post'}
                                                                                            </button>
                                                                                            {(entry.isTransfer ||
                                                                                                transferMatch !==
                                                                                                    null) && (
                                                                                                <button
                                                                                                    type="button"
                                                                                                    className="ghost-button"
                                                                                                    disabled={
                                                                                                        isBusy ||
                                                                                                        isPipelinePosting ||
                                                                                                        isPipelinePostingAllLedger ||
                                                                                                        glLockStatus.locked ||
                                                                                                        selectedLoginLocked
                                                                                                    }
                                                                                                    onClick={() => {
                                                                                                        void handleOpenTransferModal(
                                                                                                            entry.id,
                                                                                                        );
                                                                                                    }}
                                                                                                >
                                                                                                    Link
                                                                                                    Transfer
                                                                                                </button>
                                                                                            )}
                                                                                        </>
                                                                                    ) : (
                                                                                        <>
                                                                                            {needsSync && (
                                                                                                <button
                                                                                                    type="button"
                                                                                                    className="ghost-button"
                                                                                                    disabled={
                                                                                                        isBusy ||
                                                                                                        isPipelinePosting ||
                                                                                                        isPipelinePostingAllLedger ||
                                                                                                        glLockStatus.locked ||
                                                                                                        selectedLoginLocked
                                                                                                    }
                                                                                                    onClick={() => {
                                                                                                        void handlePipelineSyncEntry(
                                                                                                            entry.id,
                                                                                                        );
                                                                                                    }}
                                                                                                >
                                                                                                    {isBusy
                                                                                                        ? 'Syncing...'
                                                                                                        : 'Sync'}
                                                                                                </button>
                                                                                            )}
                                                                                            <button
                                                                                                type="button"
                                                                                                className="ghost-button"
                                                                                                onClick={() => {
                                                                                                    const parts =
                                                                                                        (
                                                                                                            entry.posted ??
                                                                                                            ''
                                                                                                        ).split(
                                                                                                            ':',
                                                                                                        );
                                                                                                    const glTxnId =
                                                                                                        parts[
                                                                                                            parts.length -
                                                                                                                1
                                                                                                        ] ??
                                                                                                        '';
                                                                                                    if (
                                                                                                        glTxnId
                                                                                                    ) {
                                                                                                        setTransactionsSearch(
                                                                                                            glTxnId,
                                                                                                        );
                                                                                                        setActiveTab(
                                                                                                            'transactions',
                                                                                                        );
                                                                                                    }
                                                                                                }}
                                                                                            >
                                                                                                View
                                                                                            </button>
                                                                                        </>
                                                                                    )}
                                                                                </div>
                                                                            </td>
                                                                        </tr>
                                                                    );
                                                                },
                                                            )
                                                        )}
                                                    </tbody>
                                                </table>
                                            </div>
                                            {transferModalEntryId !== null && (
                                                <div
                                                    className="modal-overlay"
                                                    onClick={() => {
                                                        setTransferModalEntryId(
                                                            null,
                                                        );
                                                    }}
                                                >
                                                    <div
                                                        className="modal-dialog"
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                        }}
                                                    >
                                                        <div className="modal-header">
                                                            <h3>
                                                                Link Transfer
                                                            </h3>
                                                            <button
                                                                type="button"
                                                                className="ghost-button"
                                                                onClick={() => {
                                                                    setTransferModalEntryId(
                                                                        null,
                                                                    );
                                                                }}
                                                            >
                                                                Close
                                                            </button>
                                                        </div>
                                                        <input
                                                            type="search"
                                                            placeholder="Search candidates…"
                                                            value={
                                                                transferModalSearch
                                                            }
                                                            onChange={(e) => {
                                                                setTransferModalSearch(
                                                                    e.target
                                                                        .value,
                                                                );
                                                            }}
                                                        />
                                                        {isLoadingTransferModal ? (
                                                            <p className="status">
                                                                Loading
                                                                entries...
                                                            </p>
                                                        ) : (
                                                            <div className="table-wrap">
                                                                <table className="ledger-table">
                                                                    <thead>
                                                                        <tr>
                                                                            <th>
                                                                                Date
                                                                            </th>
                                                                            <th>
                                                                                Login/Label
                                                                            </th>
                                                                            <th>
                                                                                Description
                                                                            </th>
                                                                            <th>
                                                                                Amount
                                                                            </th>
                                                                            <th></th>
                                                                        </tr>
                                                                    </thead>
                                                                    <tbody>
                                                                        {visibleTransferResults.length ===
                                                                        0 ? (
                                                                            <tr>
                                                                                <td
                                                                                    colSpan={
                                                                                        5
                                                                                    }
                                                                                    className="table-empty"
                                                                                >
                                                                                    No
                                                                                    unposted
                                                                                    entries
                                                                                    found
                                                                                    in
                                                                                    other
                                                                                    accounts.
                                                                                </td>
                                                                            </tr>
                                                                        ) : (
                                                                            visibleTransferResults.map(
                                                                                (
                                                                                    r,
                                                                                ) => (
                                                                                    <tr
                                                                                        key={
                                                                                            r
                                                                                                .entry
                                                                                                .id
                                                                                        }
                                                                                    >
                                                                                        <td className="mono">
                                                                                            {
                                                                                                r
                                                                                                    .entry
                                                                                                    .date
                                                                                            }
                                                                                        </td>
                                                                                        <td>
                                                                                            {
                                                                                                r.loginName
                                                                                            }

                                                                                            /
                                                                                            {
                                                                                                r.label
                                                                                            }
                                                                                        </td>
                                                                                        <td>
                                                                                            {
                                                                                                r
                                                                                                    .entry
                                                                                                    .description
                                                                                            }
                                                                                        </td>
                                                                                        <td className="mono">
                                                                                            {r
                                                                                                .entry
                                                                                                .amount ??
                                                                                                '-'}
                                                                                        </td>
                                                                                        <td>
                                                                                            <button
                                                                                                type="button"
                                                                                                className="primary-button"
                                                                                                onClick={() => {
                                                                                                    void handleLinkTransferFromModal(
                                                                                                        r,
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
                                                        )}
                                                    </div>
                                                </div>
                                            )}
                                        </div>
                                    )
                                ) : (
                                    <div className="table-wrap">
                                        <TransactionsTable
                                            transactions={pipelineGlRows}
                                            ledgerPath={ledgerPath}
                                        />
                                    </div>
                                )}
                                {pipelineStatus === null ? null : (
                                    <p className="status">{pipelineStatus}</p>
                                )}
                            </section>
                        </div>
                    ) : activeTab === 'reports' ? (
                        <ReportsTab
                            ledger={ledger.path}
                            accounts={ledger.accounts}
                        />
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
                                        <details className="add-extension-disclosure">
                                            <summary
                                                className="ghost-button"
                                                style={
                                                    isImportingScrapeExtension
                                                        ? {
                                                              pointerEvents:
                                                                  'none',
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
                                        </details>
                                    </div>
                                </div>
                                {extensionLoadStatus === null ? null : (
                                    <p className="status">
                                        {extensionLoadStatus}
                                    </p>
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
                                    <div className="tabs pipeline-subtabs">
                                        <button
                                            type="button"
                                            className={
                                                loginManagementTab === 'select'
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                setLoginManagementTab('select');
                                            }}
                                            disabled={loginNames.length === 0}
                                        >
                                            Selected login
                                        </button>
                                        <button
                                            type="button"
                                            className={
                                                loginManagementTab === 'create'
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                setLoginManagementTab('create');
                                            }}
                                        >
                                            Create login
                                        </button>
                                    </div>
                                    {loginManagementTab === 'create' ? (
                                        <div className="login-create-body">
                                            <div className="txn-grid">
                                                <label className="field">
                                                    <span>
                                                        Create login name
                                                    </span>
                                                    <input
                                                        type="text"
                                                        value={newLoginName}
                                                        placeholder="chase-personal"
                                                        onChange={(event) => {
                                                            setNewLoginName(
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
                                                        disabled={
                                                            isSavingLoginConfig
                                                        }
                                                    />
                                                </label>
                                                <label className="field">
                                                    <span>
                                                        Initial extension
                                                    </span>
                                                    <input
                                                        type="text"
                                                        value={
                                                            newLoginExtension
                                                        }
                                                        placeholder="optional"
                                                        onChange={(event) => {
                                                            setNewLoginExtension(
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
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
                                                        void handleCreateLoginConfig();
                                                    }}
                                                    disabled={
                                                        isSavingLoginConfig
                                                    }
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
                                                    <span>Selected login</span>
                                                    <select
                                                        value={
                                                            selectedLoginName
                                                        }
                                                        onChange={(event) => {
                                                            setSelectedLoginName(
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
                                                        disabled={
                                                            isSavingLoginConfig
                                                        }
                                                    >
                                                        <option value="">
                                                            {isLoadingLoginConfigs
                                                                ? 'Loading logins...'
                                                                : 'Select login'}
                                                        </option>
                                                        {loginNames.map(
                                                            (loginName) => (
                                                                <option
                                                                    key={
                                                                        loginName
                                                                    }
                                                                    value={
                                                                        loginName
                                                                    }
                                                                >
                                                                    {loginName}
                                                                </option>
                                                            ),
                                                        )}
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
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
                                                        disabled={
                                                            selectedLoginName.length ===
                                                                0 ||
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
                                                        void handleSaveSelectedLoginExtension();
                                                    }}
                                                    disabled={
                                                        selectedLoginName.length ===
                                                            0 ||
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
                                                        void handleDeleteSelectedLoginConfig();
                                                    }}
                                                    disabled={
                                                        selectedLoginName.length ===
                                                            0 ||
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
                                                    Select a login to manage its
                                                    account labels.
                                                </p>
                                            ) : selectedLoginAccounts.length ===
                                              0 ? (
                                                <p className="hint">
                                                    No labels configured for
                                                    this login.
                                                </p>
                                            ) : (
                                                <div className="table-wrap">
                                                    <table className="ledger-table">
                                                        <thead>
                                                            <tr>
                                                                <th>Label</th>
                                                                <th>
                                                                    GL Account
                                                                </th>
                                                                <th>Actions</th>
                                                            </tr>
                                                        </thead>
                                                        <tbody>
                                                            {selectedLoginAccounts.map(
                                                                ([
                                                                    label,
                                                                    config,
                                                                ]) => {
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
                                                                        <tr
                                                                            key={
                                                                                label
                                                                            }
                                                                        >
                                                                            <td>
                                                                                <span className="mono">
                                                                                    {
                                                                                        label
                                                                                    }
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
                                                                                        setLoginLabelDraft(
                                                                                            label,
                                                                                        );
                                                                                        setLoginGlAccountDraft(
                                                                                            config.glAccount ??
                                                                                                '',
                                                                                        );
                                                                                        setLoginConfigStatus(
                                                                                            `Loaded '${selectedLoginName}/${label}' for editing.`,
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
                                                                                            void handleIgnoreLoginAccountMapping(
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
                                                    {selectedLoginConflictCount}{' '}
                                                    mapping conflict
                                                    {selectedLoginConflictCount ===
                                                    1
                                                        ? ''
                                                        : 's'}{' '}
                                                    for this login. Resolve by
                                                    editing or ignoring a
                                                    conflicting mapping.
                                                </p>
                                            ) : null}
                                            <div className="txn-grid">
                                                <label className="field">
                                                    <span>Label</span>
                                                    <input
                                                        type="text"
                                                        value={loginLabelDraft}
                                                        placeholder="checking"
                                                        onChange={(event) => {
                                                            setLoginLabelDraft(
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
                                                        disabled={
                                                            selectedLoginName.length ===
                                                                0 ||
                                                            isSavingLoginConfig
                                                        }
                                                    />
                                                </label>
                                                <label className="field">
                                                    <span>GL account</span>
                                                    <input
                                                        type="text"
                                                        value={
                                                            loginGlAccountDraft
                                                        }
                                                        placeholder="Assets:Bank:Checking (blank = ignored)"
                                                        list="scrape-account-options"
                                                        onChange={(event) => {
                                                            setLoginGlAccountDraft(
                                                                event.target
                                                                    .value,
                                                            );
                                                            setLoginConfigStatus(
                                                                null,
                                                            );
                                                        }}
                                                        disabled={
                                                            selectedLoginName.length ===
                                                                0 ||
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
                                                            0 ||
                                                        isSavingLoginConfig
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
                                                        setLoginConfigStatus(
                                                            null,
                                                        );
                                                    }}
                                                    disabled={
                                                        selectedLoginName.length ===
                                                            0 ||
                                                        isSavingLoginConfig
                                                    }
                                                >
                                                    Use selected account
                                                </button>
                                            </div>
                                        </>
                                    )}
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
                                <details className="dev-tools-disclosure">
                                    <summary className="disclosure-summary">
                                        Developer tools
                                        {scrapeDebugSocket !== null
                                            ? ' (session active)'
                                            : ''}
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
                                                    scrapeDebugSocket !==
                                                        null ||
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
                                                    scrapeDebugSocket ===
                                                        null ||
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
                                                disabled={
                                                    scrapeDebugSocket === null
                                                }
                                            >
                                                Copy socket
                                            </button>
                                        </div>
                                        {scrapeDebugSocket === null ? null : (
                                            <p className="hint mono">
                                                Debug socket:{' '}
                                                {scrapeDebugSocket}
                                            </p>
                                        )}
                                    </div>
                                </details>
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
                                            setIsSecretsPanelExpanded(
                                                event.currentTarget.open,
                                            );
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
                                                onSubmit={
                                                    handleSubmitSecretForm
                                                }
                                            >
                                                <div className="txn-grid">
                                                    <label className="field">
                                                        <span>Domain</span>
                                                        <input
                                                            type="text"
                                                            value={secretDomain}
                                                            placeholder="example.com"
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                setSecretDomain(
                                                                    event.target
                                                                        .value,
                                                                );
                                                                setSecretsStatus(
                                                                    null,
                                                                );
                                                            }}
                                                            disabled={
                                                                !hasActiveSecretsLogin ||
                                                                isSavingAccountSecret ||
                                                                busySecretKey !==
                                                                    null
                                                            }
                                                        />
                                                    </label>
                                                    <label className="field">
                                                        <span>Name</span>
                                                        <input
                                                            type="text"
                                                            value={secretName}
                                                            placeholder="password"
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                setSecretName(
                                                                    event.target
                                                                        .value,
                                                                );
                                                                setSecretsStatus(
                                                                    null,
                                                                );
                                                            }}
                                                            disabled={
                                                                !hasActiveSecretsLogin ||
                                                                isSavingAccountSecret ||
                                                                busySecretKey !==
                                                                    null
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
                                                            onChange={(
                                                                event,
                                                            ) => {
                                                                setSecretValue(
                                                                    event.target
                                                                        .value,
                                                                );
                                                                setSecretsStatus(
                                                                    null,
                                                                );
                                                            }}
                                                            disabled={
                                                                !hasActiveSecretsLogin ||
                                                                isSavingAccountSecret ||
                                                                busySecretKey !==
                                                                    null
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
                                                            busySecretKey !==
                                                                null
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
                                                            busySecretKey !==
                                                                null
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
                                                    domain/name. Press Enter or
                                                    use Set or Change value to
                                                    save the value.
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
                                                                        <tr
                                                                            key={
                                                                                key
                                                                            }
                                                                        >
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
                                                    stored for this login are
                                                    not declared by the selected
                                                    extension.
                                                </p>
                                            ) : null}
                                            {secretsStatus === null ? null : (
                                                <p className="status">
                                                    {secretsStatus}
                                                </p>
                                            )}
                                        </div>
                                    </details>
                                </section>
                                <section className="pipeline-panel">
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Extraction pipeline</h3>
                                            <p>
                                                Select documents, run
                                                extraction, and review
                                                account-level journal and
                                                posting state.
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
                                            <h3>Posting queue</h3>
                                            <p>
                                                Assign counterpart accounts for
                                                unposted entries.
                                            </p>
                                        </div>
                                    </div>
                                    {isLoadingUnposted ? (
                                        <p className="status">
                                            Loading unposted entries...
                                        </p>
                                    ) : unpostedEntries.length === 0 ? (
                                        <p className="hint">
                                            {selectedScrapeAccount.length === 0
                                                ? 'Choose a GL account to view unposted entries.'
                                                : !hasResolvedLoginMapping
                                                  ? 'Resolve login mapping first to view unposted entries.'
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
                                                    {unpostedEntries.map(
                                                        (entry) => {
                                                            const draft =
                                                                postDrafts[
                                                                    entry.id
                                                                ] ?? {
                                                                    counterpartAccount:
                                                                        '',
                                                                    postingIndex:
                                                                        '',
                                                                };
                                                            const isBusy =
                                                                busyPostEntryId ===
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
                                                                            onChange={(
                                                                                event,
                                                                            ) => {
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
                                            <span>Unpost entry ID</span>
                                            <input
                                                type="text"
                                                value={unpostEntryId}
                                                placeholder="entry id"
                                                onChange={(event) => {
                                                    setUnpostEntryId(
                                                        event.target.value,
                                                    );
                                                    setPipelineStatus(null);
                                                }}
                                            />
                                        </label>
                                        <label className="field">
                                            <span>
                                                Unpost posting index (optional)
                                            </span>
                                            <input
                                                type="text"
                                                value={unpostPostingIndex}
                                                placeholder="0"
                                                onChange={(event) => {
                                                    setUnpostPostingIndex(
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
                                                void handleUnpostAccountEntry();
                                            }}
                                            disabled={isUnpostingEntry}
                                        >
                                            {isUnpostingEntry
                                                ? 'Unposting...'
                                                : 'Unpost entry'}
                                        </button>
                                    </div>
                                    <div className="txn-form-header">
                                        <div>
                                            <h3>Transfer posting</h3>
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

function TransactionsTable({
    transactions,
    ledgerPath,
    glCategorySuggestions = {},
    onRecategorize,
    onMergeTransfer,
}: {
    transactions: TransactionRow[];
    ledgerPath: string | null;
    glCategorySuggestions?: Record<string, GlCategoryResult>;
    onRecategorize?: (txnId: string, newAccount: string) => void;
    onMergeTransfer?: (txnId1: string, txnId2: string) => void;
}) {
    const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
    const [lightboxFilename, setLightboxFilename] = useState<string | null>(
        null,
    );
    const [lightboxLoading, setLightboxLoading] = useState(false);
    const [lightboxError, setLightboxError] = useState<string | null>(null);

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
        onRecategorize !== undefined || onMergeTransfer !== undefined;
    const colCount = hasActions ? 6 : 5;

    return (
        <>
            <table className="ledger-table">
                <thead>
                    <tr>
                        <th>Date</th>
                        <th>Description</th>
                        <th>Postings</th>
                        <th>Amount</th>
                        <th>Attachments</th>
                        {hasActions && <th>Categorize</th>}
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
                            const glSuggestion = glCategorySuggestions[txn.id];
                            const transferMatch =
                                glSuggestion?.transferMatch ?? null;
                            const suggested = glSuggestion?.suggested ?? null;
                            return (
                                <tr
                                    key={txn.id}
                                    className={
                                        isUncategorized
                                            ? 'row-uncategorized'
                                            : undefined
                                    }
                                >
                                    <td className="mono">{txn.date}</td>
                                    <td>{txn.description}</td>
                                    <td>
                                        <PostingsList postings={txn.postings} />
                                    </td>
                                    <td className="amount">
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
                                                            >
                                                                {evidenceRef}
                                                            </span>
                                                        ),
                                                )}
                                            </div>
                                        )}
                                    </td>
                                    {hasActions && (
                                        <td>
                                            {transferMatch !== null &&
                                            onMergeTransfer !== undefined ? (
                                                <div className="categorize-chip">
                                                    <span className="text-muted">
                                                        Transfer ↔{' '}
                                                        {transferMatch.date}{' '}
                                                        {
                                                            transferMatch.description
                                                        }
                                                    </span>
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
                                                        Merge
                                                    </button>
                                                </div>
                                            ) : suggested !== null &&
                                              onRecategorize !== undefined ? (
                                                <div className="categorize-chip">
                                                    <span className="text-muted">
                                                        {suggested}
                                                    </span>
                                                    <button
                                                        type="button"
                                                        className="ghost-button"
                                                        onClick={() => {
                                                            onRecategorize(
                                                                txn.id,
                                                                suggested,
                                                            );
                                                        }}
                                                    >
                                                        Accept
                                                    </button>
                                                </div>
                                            ) : null}
                                        </td>
                                    )}
                                </tr>
                            );
                        })
                    )}
                </tbody>
            </table>
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
