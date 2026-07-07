// Pure helpers for navigating from a GL transaction back to the source account
// entry and its evidence rows. Extracted so the parsing is unit-testable.
//
// Keep parseGlSourceRefs aligned with post::parse_sources_from_block
// (src-tauri/src/post.rs) and pickEvidenceTarget aligned with PipelineTab's
// evidencedRowNumbers (src/tabs/PipelineTab.tsx) / lib.rs evidence parsing.

export interface GlSourceRef {
    /** Account-journal locator, e.g. `logins/chase/accounts/checking`. */
    locator: string;
    entryId: string;
}

/**
 * Parse the `; source: <locator>:<entryId>` lines out of a GL transaction
 * comment. Mirrors post::parse_sources_from_block: `:posting:` (per-leg) refs
 * are skipped and the locator/entryId are split on the LAST colon.
 */
export function parseGlSourceRefs(comment: string): GlSourceRef[] {
    const refs: GlSourceRef[] = [];
    const prefix = '; source: ';
    for (const line of comment.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed.startsWith(prefix)) continue;
        const rest = trimmed.slice(prefix.length);
        if (rest.includes(':posting:')) continue; // per-leg source, no single entry
        const colonPos = rest.lastIndexOf(':');
        if (colonPos < 0) continue;
        const locator = rest.slice(0, colonPos);
        const entryId = rest.slice(colonPos + 1);
        if (locator.length > 0 && entryId.length > 0) {
            refs.push({ locator, entryId });
        }
    }
    return refs;
}

export interface LoginAccountLocator {
    loginName: string;
    label: string;
}

/**
 * Split a source locator `logins/<login>/accounts/<label>` into its parts.
 * Returns null for locators that are not login-account journals.
 */
export function parseLoginAccountLocator(
    locator: string,
): LoginAccountLocator | null {
    const match = /^logins\/(.+)\/accounts\/(.+)$/.exec(locator);
    if (match === null) return null;
    const loginName = match[1];
    const label = match[2];
    if (loginName === undefined || label === undefined) return null;
    return { loginName, label };
}

export interface EvidenceTarget {
    /** The evidence document (e.g. `statements/2026/jan.pdf`). */
    document: string;
    /** The 1-based source row, if the evidence encodes one. */
    row: number | null;
}

/**
 * Pick a navigation target from an account entry's evidence refs. Each ref is
 * `<document>:<row>:<col>`; the document (which may contain `/` but not `:`) is
 * everything before the final two colon-separated fields. Returns the first
 * parseable ref, or null.
 */
export function pickEvidenceTarget(evidence: string[]): EvidenceTarget | null {
    for (const ref of evidence) {
        const parts = ref.split(':');
        if (parts.length < 3) continue;
        const document = parts.slice(0, parts.length - 2).join(':');
        if (document.length === 0) continue;
        const rowText = parts[parts.length - 2] ?? '';
        const row = Number.parseInt(rowText, 10);
        return { document, row: Number.isNaN(row) ? null : row };
    }
    return null;
}
