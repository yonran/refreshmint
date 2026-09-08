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

/**
 * True when an extraction error is the "no glAccount is configured" failure:
 * the extractor didn't supply explicit tpostings, the login/label has no
 * prior journal entries to infer an account from, and no GL account is
 * mapped for it in Settings. Keep this substring in sync with the error text
 * raised in src-tauri/src/lib.rs (run_login_account_extraction) and
 * src-tauri/src/cli.rs (the `account extract` subcommand).
 */
export function isMissingGlAccountError(error: unknown): boolean {
    return String(error).includes('no glAccount is configured');
}
