import { load } from '@tauri-apps/plugin-store';

const STORE_NAME = 'settings.json';
const RECENT_LEDGER_KEY = 'recentLedgerPaths';
const ACTIVE_TAB_KEY = 'recentActiveTab';

export type ActiveTab = 'accounts' | 'transactions';

async function getStore() {
    return await load(STORE_NAME, { autoSave: true, defaults: {} });
}

export async function getRecentLedgers(): Promise<string[]> {
    const store = await getStore();
    const value = await store.get<unknown>(RECENT_LEDGER_KEY);
    if (Array.isArray(value)) {
        return value.filter(
            (entry): entry is string => typeof entry === 'string',
        );
    }
    if (typeof value === 'string') {
        return [value];
    }
    return [];
}

export async function setRecentLedgers(entries: string[]): Promise<void> {
    const store = await getStore();
    await store.set(RECENT_LEDGER_KEY, entries);
    await store.save();
}

export function addRecentLedger(entries: string[], path: string): string[] {
    if (path.length === 0) {
        return entries;
    }
    return [path, ...entries];
}

export function removeRecentLedger(entries: string[], path: string): string[] {
    return entries.filter((entry) => entry !== path);
}

export async function getLastActiveTab(): Promise<ActiveTab | null> {
    const store = await getStore();
    const value = await store.get<unknown>(ACTIVE_TAB_KEY);
    if (value === 'accounts' || value === 'transactions') {
        return value;
    }
    return null;
}

export async function setLastActiveTab(tab: ActiveTab): Promise<void> {
    const store = await getStore();
    await store.set(ACTIVE_TAB_KEY, tab);
    await store.save();
}
