// Pure helpers backing the live scrape console pane in ScrapeTab. Extracted so
// the ring-buffer/formatting logic is unit-testable (React rendering is not).
//
// The event shape mirrors the `refreshmint://scrape-output` payload emitted by
// `run_scrape_for_login` in `src-tauri/src/lib.rs` (ScrapeOutputPayload).

export interface ScrapeOutputLine {
    stream: 'stdout' | 'stderr';
    line: string;
}

/** Default maximum number of retained console lines. */
export const DEFAULT_SCRAPE_CONSOLE_MAX = 1000;

/**
 * Render a single scrape-output event as a console line. stderr lines are
 * prefixed so warnings/errors stand out from ordinary stdout output.
 */
export function formatScrapeOutputLine(entry: ScrapeOutputLine): string {
    return entry.stream === 'stderr' ? `[stderr] ${entry.line}` : entry.line;
}

/** Minimal shape of a per-login scrape summary needed to judge staleness. */
export interface StaleSummaryLike {
    lastSuccess?: string | null;
}

/**
 * Return the login names that are stale: never successfully scraped, or whose
 * most recent success is older than `intervalHours`. Pure so the scheduler's
 * source-of-truth logic (backed by scrape-log.jsonl via getLastScrapeSummaries)
 * is unit-testable.
 */
export function computeStaleLogins(
    summaries: Record<string, StaleSummaryLike>,
    intervalHours: number,
    now: number,
): string[] {
    const intervalMs = intervalHours * 60 * 60 * 1000;
    return Object.entries(summaries)
        .filter(([, summary]) => {
            const lastSuccess = summary.lastSuccess ?? null;
            if (lastSuccess === null) return true;
            return now - new Date(lastSuccess).getTime() > intervalMs;
        })
        .map(([loginName]) => loginName);
}

/** Default MFA-prompt timeout in minutes (matches DEFAULT_PROMPT_TIMEOUT_SECS / 60 on the Rust side). */
export const DEFAULT_MFA_PROMPT_TIMEOUT_MINUTES = 5;
/** Upper bound on the configurable MFA-prompt timeout. */
export const MAX_MFA_PROMPT_TIMEOUT_MINUTES = 60;

/**
 * Clamp a user-entered MFA-prompt timeout (minutes) into a sane whole-minute
 * range, falling back to the default for non-finite input.
 */
export function clampPromptTimeoutMinutes(value: number): number {
    if (!Number.isFinite(value)) return DEFAULT_MFA_PROMPT_TIMEOUT_MINUTES;
    const rounded = Math.floor(value);
    if (rounded < 1) return 1;
    if (rounded > MAX_MFA_PROMPT_TIMEOUT_MINUTES)
        return MAX_MFA_PROMPT_TIMEOUT_MINUTES;
    return rounded;
}

/**
 * Whether the console should auto-scroll to the newest line: only when the view
 * is already at (or within `threshold` px of) the bottom. This keeps a user who
 * has scrolled up to read earlier output from being yanked back down when new
 * lines arrive. Pure so the sticky-scroll decision is unit-testable.
 */
export function shouldStickToBottom(
    scrollTop: number,
    scrollHeight: number,
    clientHeight: number,
    threshold = 32,
): boolean {
    const distanceFromBottom = scrollHeight - clientHeight - scrollTop;
    return distanceFromBottom <= threshold;
}

export interface PartitionedArtifacts {
    /** The screenshot artifact (a `.png`), if present. */
    imageName: string | null;
    /** Remaining artifacts (url.txt, log-tail.txt, …), sorted. */
    textNames: string[];
}

/**
 * Split a failure-artifacts directory listing into the screenshot (shown as an
 * image) and the text artifacts (shown as text). Keeps the viewer logic pure
 * and unit-testable.
 */
export function partitionArtifacts(names: string[]): PartitionedArtifacts {
    const imageName =
        names.find((name) => name.toLowerCase().endsWith('.png')) ?? null;
    const textNames = names.filter((name) => name !== imageName).sort();
    return { imageName, textNames };
}

/**
 * Append a formatted scrape-output line to a bounded console buffer, evicting
 * the oldest lines once `max` is exceeded. Returns a new array (never mutates
 * the input) so it can drive React state.
 */
export function appendLogLine(
    lines: string[],
    entry: ScrapeOutputLine,
    max = DEFAULT_SCRAPE_CONSOLE_MAX,
): string[] {
    const next = [...lines, formatScrapeOutputLine(entry)];
    if (next.length > max) {
        next.splice(0, next.length - max);
    }
    return next;
}
