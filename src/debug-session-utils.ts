import type { DebugSessionStatusView } from './tauri-commands.ts';

/**
 * One-line summary of a debug session for the Scrape tab, e.g.
 * "running script · expires in 27m". `fetchedAtMs` is when `status` was
 * read, so the countdown keeps moving between refreshes; a `nowMs` older
 * than the fetch (the clock hadn't ticked yet) counts as no time elapsed.
 */
export function describeDebugSessionStatus(
    status: DebugSessionStatusView | null,
    fetchedAtMs: number,
    nowMs: number,
): string {
    if (status === null) return 'status unavailable';
    const activity = status.execRunning ? 'running script' : 'idle';
    if (status.expiresInSecs === null) return activity;
    const remainingSecs = Math.max(
        0,
        status.expiresInSecs -
            Math.floor(Math.max(0, nowMs - fetchedAtMs) / 1000),
    );
    return `${activity} · expires in ${formatRemaining(remainingSecs)}`;
}

function formatRemaining(secs: number): string {
    if (secs < 60) return `${String(secs)}s`;
    return `${String(Math.ceil(secs / 60))}m`;
}
