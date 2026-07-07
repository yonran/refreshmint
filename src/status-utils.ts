// Pure helpers backing status banners across tabs. Extracted so the
// error-vs-info classification (and other banner logic) is unit-testable
// (React rendering is not).

/**
 * Classify a free-form status message as an error or informational message.
 *
 * Mirrors the inline heuristic previously used by ScrapeTab's status line:
 * a message that mentions "failed" or "error" (case-insensitive) is an error.
 */
export function classifyStatusMessage(message: string): 'error' | 'info' {
    const lower = message.toLowerCase();
    return lower.includes('failed') || lower.includes('error')
        ? 'error'
        : 'info';
}
