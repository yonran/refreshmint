// Pure helpers for the Bookkeeping (Reconcile & Close) tab.

/** Split a comma/newline-separated id list into trimmed, non-empty ids. */
function parseIdList(listText: string): string[] {
    return listText
        .split(/[\n,]/)
        .map((part) => part.trim())
        .filter((part) => part.length > 0);
}

/**
 * Append `id` to a comma/newline-separated `listText`, deduping. Returns the
 * list unchanged when `id` is blank or already present. The result is
 * normalized to a ", "-joined list.
 */
export function appendIdToList(listText: string, id: string): string {
    const trimmed = id.trim();
    const existing = parseIdList(listText);
    if (trimmed.length === 0 || existing.includes(trimmed)) {
        return listText;
    }
    return [...existing, trimmed].join(', ');
}
