import { load } from '@tauri-apps/plugin-store';

const STORE_NAME = 'settings.json';
const RECENT_LEDGER_KEY = 'recentLedgerPaths';
const ACTIVE_TAB_KEY = 'recentActiveTab';

export type ActiveTab = 'accounts' | 'transactions' | 'scrape';

async function getStore() {
    return await load(STORE_NAME, { autoSave: true, defaults: {} });
}

function normalizeRecentLedgerEntries(entries: string[]): string[] {
    const normalized: string[] = [];
    const seen = new Set<string>();
    for (const entry of entries) {
        const path = entry.trim();
        if (path.length === 0 || seen.has(path)) {
            continue;
        }
        seen.add(path);
        normalized.push(path);
    }
    return normalized;
}

export async function getRecentLedgers(): Promise<string[]> {
    const store = await getStore();
    const value = await store.get<unknown>(RECENT_LEDGER_KEY);
    if (Array.isArray(value)) {
        return normalizeRecentLedgerEntries(
            value.filter((entry): entry is string => typeof entry === 'string'),
        );
    }
    if (typeof value === 'string') {
        return normalizeRecentLedgerEntries([value]);
    }
    return [];
}

export async function setRecentLedgers(entries: string[]): Promise<void> {
    const store = await getStore();
    await store.set(RECENT_LEDGER_KEY, normalizeRecentLedgerEntries(entries));
    await store.save();
}

export function addRecentLedger(entries: string[], path: string): string[] {
    const normalizedPath = path.trim();
    if (normalizedPath.length === 0) {
        return normalizeRecentLedgerEntries(entries);
    }
    return normalizeRecentLedgerEntries([normalizedPath, ...entries]);
}

export function removeRecentLedger(entries: string[], path: string): string[] {
    const normalizedPath = path.trim();
    if (normalizedPath.length === 0) {
        return normalizeRecentLedgerEntries(entries);
    }
    return normalizeRecentLedgerEntries(entries).filter(
        (entry) => entry !== normalizedPath,
    );
}

export async function getLastActiveTab(): Promise<ActiveTab | null> {
    const store = await getStore();
    const value = await store.get<unknown>(ACTIVE_TAB_KEY);
    if (
        value === 'accounts' ||
        value === 'transactions' ||
        value === 'scrape'
    ) {
        return value;
    }
    return null;
}

export async function setLastActiveTab(tab: ActiveTab): Promise<void> {
    const store = await getStore();
    await store.set(ACTIVE_TAB_KEY, tab);
    await store.save();
}
