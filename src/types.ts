import type {
    LoginConfig,
    LoginAccountConfig,
    LockStatus,
    TransactionRow,
    HledgerReportResult,
} from './tauri-commands.ts';
import {
    createDefaultReportConfig,
    type ReportConfig,
} from './report-utils.ts';

export type TransactionDraft = {
    date: string;
    description: string;
    comment: string;
    postings: DraftPosting[];
};

export type DraftPosting = {
    account: string;
    amount: string;
    comment: string;
};

export type TransactionEntryMode = 'form' | 'raw';
export type SplitDraftRow = { account: string; amount: string };
export type PipelineSubTab = 'evidence' | 'evidence-rows' | 'account-rows';

export type SecretPromptState = {
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
};

export type LoginAccountRef = {
    loginName: string;
    label: string;
};

export type TransactionsTabSession = {
    unpostedOnly: boolean;
    bookkeepingFilter:
        | 'all'
        | 'reconciled'
        | 'linked'
        | 'settled'
        | 'softClosed'
        | 'generated';
    transactionDraft: TransactionDraft | null;
    rawDraft: string;
    entryMode: TransactionEntryMode;
    transactionsSearch: string;
    selectedTransactionIds: string[];
    transactionsTableScrollTop: number;
    isNewTxnExpandedOverride: boolean | null;
    glTransferModalTxnId: string | null;
    glTransferModalSearch: string;
};

export type ScrapeTabSession = {
    /**
     * The login whose scrape this tab started and is still running, or null.
     * Held in the App-owned session so switching tabs mid-scrape (which unmounts
     * ScrapeTab) doesn't drop the Running/Cancel state or the completion status.
     */
    runningLoginName: string | null;
    /** Latest status line (running / completed / failed / canceled), or null. */
    scrapeStatus: string | null;
    /** Live driver output lines for the selected login. */
    consoleLines: string[];
};

export type PipelineTabSession = {
    selectedLoginAccount: LoginAccountRef | null;
    pipelineStatus: string | null;
    pipelineSubTab: PipelineSubTab;
    evidenceRowsDocument: string;
    pipelineSelectedEntryIds: string[];
    pipelineGlAccountDraft: string;
    transferModalEntryId: string | null;
    splitModalEntryId: string | null;
    splitDraftRows: SplitDraftRow[];
    transferModalSearch: string;
};

export type ReportsTabSession = {
    /** The current (possibly unrun) report options. */
    config: ReportConfig;
    /** Last successful report output, or null if none has run. */
    result: HledgerReportResult | null;
    /** Last run's error message, or null. */
    error: string | null;
    /** The config that produced `result`/`error`; drives the stale-result hint. */
    lastRunConfig: ReportConfig | null;
    /** Whether the default report has auto-run once for this ledger. */
    hasAutoRun: boolean;
};

export type PipelineBulkAccountStat = {
    loginName: string;
    label: string;
    extract: {
        eligible: boolean;
        documentCount: number;
        skipReason:
            | 'missing-extension'
            | 'missing-extractor'
            | 'broken-extractor'
            | 'no-documents'
            | null;
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

/** Reason an account was skipped during bulk extract (non-null variants only). */
export type ExtractSkipReason = NonNullable<
    PipelineBulkAccountStat['extract']['skipReason']
>;

/** Reason an account was skipped during bulk post (non-null variants only). */
export type PostSkipReason = NonNullable<
    PipelineBulkAccountStat['post']['skipReason']
>;

export type PipelineBulkSummary = {
    eligibleAccounts: number;
    totalDocuments: number;
    totalUnpostedEntries: number;
    skippedMissingExtension: number;
    skippedMissingExtractor: number;
    skippedNoDocuments: number;
    skippedMissingGlAccount: number;
    skippedNoUnposted: number;
    inspectFailures: number;
    lockedAccounts: number;
};

export type PipelineBulkStats = {
    accounts: PipelineBulkAccountStat[];
    gl: LockStatus;
    extract: PipelineBulkSummary;
    post: PipelineBulkSummary;
};

export type SimilarRecategorizeSeed = {
    newAccount: string;
    description: string;
    balancingAccount: string;
};

export type SimilarRecategorizePlan = {
    newAccount: string;
    searchQuery: string;
    seedQuery: string;
    currentFilterQuery: string;
    description: string;
    balancingAccount: string;
    includeAll: boolean;
    queryCustomized: boolean;
};

export type RecategorizeTab = {
    id: number;
    plan: SimilarRecategorizePlan;
    queryResults: TransactionRow[] | null;
    queryError: string | null;
    selectedPostingIndexByTxn: Record<string, number | null>;
};

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function normalizeLoginAccountConfig(
    value: unknown,
): LoginAccountConfig {
    if (!isRecord(value)) {
        return {};
    }
    const glAccount = value['glAccount'];
    if (typeof glAccount === 'string' || glAccount === null) {
        return { glAccount };
    }
    return {};
}

export function normalizeLoginConfig(
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

export function suggestGlAccountName(label: string): string {
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

export function createEmptyPipelineBulkSummary(): PipelineBulkSummary {
    return {
        eligibleAccounts: 0,
        totalDocuments: 0,
        totalUnpostedEntries: 0,
        skippedMissingExtension: 0,
        skippedMissingExtractor: 0,
        skippedNoDocuments: 0,
        skippedMissingGlAccount: 0,
        skippedNoUnposted: 0,
        inspectFailures: 0,
        lockedAccounts: 0,
    };
}

export function createEmptyTransactionsTabSession(): TransactionsTabSession {
    return {
        unpostedOnly: false,
        bookkeepingFilter: 'all',
        transactionDraft: null,
        rawDraft: '',
        entryMode: 'form',
        transactionsSearch: '',
        selectedTransactionIds: [],
        transactionsTableScrollTop: 0,
        isNewTxnExpandedOverride: null,
        glTransferModalTxnId: null,
        glTransferModalSearch: '',
    };
}

export function createEmptyReportsTabSession(): ReportsTabSession {
    return {
        config: createDefaultReportConfig(),
        result: null,
        error: null,
        lastRunConfig: null,
        hasAutoRun: false,
    };
}

export function createEmptyScrapeTabSession(): ScrapeTabSession {
    return {
        runningLoginName: null,
        scrapeStatus: null,
        consoleLines: [],
    };
}

export function createEmptyPipelineTabSession(): PipelineTabSession {
    return {
        selectedLoginAccount: null,
        pipelineStatus: null,
        pipelineSubTab: 'account-rows',
        evidenceRowsDocument: '',
        pipelineSelectedEntryIds: [],
        pipelineGlAccountDraft: '',
        transferModalEntryId: null,
        splitModalEntryId: null,
        splitDraftRows: [],
        transferModalSearch: '',
    };
}
