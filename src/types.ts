import type {
    LoginConfig,
    LoginAccountConfig,
    LockStatus,
    TransactionRow,
} from './tauri-commands.ts';

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

export type PostDraft = {
    counterpartAccount: string;
    postingIndex: string;
};

export type TransferDraft = {
    account1: string;
    entryId1: string;
    account2: string;
    entryId2: string;
};

export type SecretPromptState = {
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
};

export type LoginAccountMapping = {
    loginName: string;
    label: string;
    extension: string;
};

export type LoginAccountRef = {
    loginName: string;
    label: string;
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

export type SimilarRecategorizePlan = {
    entries: Array<{ txnId: string; oldAccount: string }>;
    allEntries: Array<{ txnId: string; oldAccount: string }>;
    newAccount: string;
    searchQuery: string;
    description: string;
    balancingAccount: string;
    includeAll: boolean;
};

export type RecategorizeTab = {
    id: number;
    plan: SimilarRecategorizePlan;
    queryResults: TransactionRow[] | null;
    queryError: string | null;
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
