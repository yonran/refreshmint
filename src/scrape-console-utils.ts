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
