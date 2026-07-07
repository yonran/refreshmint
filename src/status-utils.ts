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

/**
 * Whether a keydown should close a Modal. True only for a fresh Escape
 * (defaultPrevented === false) when closeOnEscape is enabled. Ignoring
 * already-handled Escapes lets an inner control (e.g. AccountInput dismissing
 * its suggestions via preventDefault) consume the first Escape without also
 * closing the surrounding modal. See src/components/Modal.tsx.
 */
export function shouldCloseOnKey(
    key: string,
    defaultPrevented: boolean,
    closeOnEscape: boolean,
): boolean {
    return closeOnEscape && key === 'Escape' && !defaultPrevented;
}
