export interface ScrapeLogEntry {
    loginName: string;
    timestamp: string; // ISO
    success: boolean;
    error?: string;
    source: 'manual' | 'auto';
}

const MAX_PER_LOGIN = 100;

export function readScrapeLog(loginName: string): ScrapeLogEntry[] {
    const raw = localStorage.getItem(`scrapeLog:${loginName}`);
    if (raw === null) return [];
    try {
        const parsed: unknown = JSON.parse(raw);
        if (!Array.isArray(parsed)) return [];
        return parsed.filter((item): item is ScrapeLogEntry => {
            const e: unknown = item;
            return (
                typeof e === 'object' &&
                e !== null &&
                'loginName' in e &&
                typeof e.loginName === 'string' &&
                'timestamp' in e &&
                typeof e.timestamp === 'string' &&
                'success' in e &&
                typeof e.success === 'boolean'
            );
        });
    } catch {
        return [];
    }
}

export function appendScrapeLog(entry: ScrapeLogEntry): void {
    const entries = readScrapeLog(entry.loginName);
    entries.unshift(entry);
    if (entries.length > MAX_PER_LOGIN) entries.length = MAX_PER_LOGIN;
    localStorage.setItem(
        `scrapeLog:${entry.loginName}`,
        JSON.stringify(entries),
    );
}
