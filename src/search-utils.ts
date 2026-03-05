import type { AccountRow } from './tauri-commands.ts';

/**
 * Quotes a value for use in an hledger query predicate (e.g. `desc:VALUE`).
 *
 * hledger tokenizes queries like a POSIX shell command line: predicates are
 * separated by whitespace, and a value containing whitespace must be wrapped
 * in double or single quotes.  Double-quote wrapping is used here; internal
 * backslashes and double quotes are backslash-escaped.
 *
 * Reference: https://hledger.org/hledger.html#queries (section "Query arguments")
 */
export function quoteHledgerValue(value: string): string {
    if (!/[\s"]/.test(value)) return value;
    return '"' + value.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';
}

export const QUERY_PREFIXES = [
    'desc:',
    'acct:',
    'date:',
    'date2:',
    'payee:',
    'note:',
    'tag:',
    'amt:',
    'cur:',
    'code:',
    'status:',
    'real:',
    'type:',
    'expr:',
    'depth:',
];

const DATE_SMART_TERMS = [
    'today',
    'yesterday',
    'thisweek',
    'lastweek',
    'thismonth',
    'lastmonth',
    'thisquarter',
    'lastquarter',
    'thisyear',
    'lastyear',
    'q1',
    'q2',
    'q3',
    'q4',
];

/**
 * Given the full input value and the cursor position, return the token
 * under the cursor along with its start and end character indices.
 *
 * Tokens are whitespace-separated; single- and double-quoted strings
 * are treated as a single token (quotes are included in the span).
 *
 * When the cursor lands on whitespace between tokens, the NEXT token
 * is returned (forward preference, so autocomplete suggests for the
 * token the user is about to type).
 */
export function getCurrentToken(
    value: string,
    cursorPos: number,
): { token: string; start: number; end: number } {
    let i = 0;
    let lastTokenEnd = 0;

    while (i < value.length) {
        // Skip whitespace between tokens
        while (i < value.length && (value[i] === ' ' || value[i] === '\t')) {
            i++;
        }
        if (i >= value.length) break;

        const tokenStart = i;

        // Scan to end of token, respecting quoted strings
        while (i < value.length) {
            const ch = value[i];
            if (ch === ' ' || ch === '\t') break;
            if (ch === '"' || ch === "'") {
                const q = ch;
                i++;
                while (i < value.length && value[i] !== q) i++;
                if (i < value.length) i++; // consume closing quote
            } else {
                i++;
            }
        }
        const tokenEnd = i;

        // Cursor is associated with this token when:
        //   lastTokenEnd <= cursorPos < tokenEnd  (cursor in gap before OR inside token)
        // OR when cursor is at tokenEnd and that's end-of-string (no trailing space)
        if (cursorPos >= lastTokenEnd && cursorPos < tokenEnd) {
            return {
                token: value.substring(tokenStart, tokenEnd),
                start: tokenStart,
                end: tokenEnd,
            };
        }
        if (cursorPos === tokenEnd && tokenEnd === value.length) {
            return {
                token: value.substring(tokenStart, tokenEnd),
                start: tokenStart,
                end: tokenEnd,
            };
        }

        lastTokenEnd = tokenEnd;
    }

    return { token: '', start: cursorPos, end: cursorPos };
}

/** The five standard hledger top-level account types. */
export const GL_ACCOUNT_TYPES = [
    'Assets',
    'Liabilities',
    'Equity',
    'Income',
    'Expenses',
];

/**
 * Returns a human-readable warning when a proposed account name change is
 * potentially wrong, or null if the change looks fine.
 *
 * "Balance-sheet" here mirrors the `isNonBalanceSheet` convention in App.tsx:
 *   balance-sheet = starts with Assets: or Liabilities:
 *   non-balance-sheet = everything else (Equity, Income, Expenses, …)
 *
 * Warns when:
 *  - newAccount doesn't start with a recognized standard type
 *  - the change crosses the balance-sheet / non-balance-sheet boundary
 *
 * @param oldAccounts  The existing account(s) being replaced.
 * @param newAccount   The proposed new account name (may be partial/empty).
 */
export function checkAccountTypeChange(
    oldAccounts: string | string[],
    newAccount: string,
): string | null {
    const trimmed = newAccount.trim();
    if (!trimmed) return null;

    const olds = Array.isArray(oldAccounts) ? oldAccounts : [oldAccounts];

    const recognized = GL_ACCOUNT_TYPES.find((t) =>
        trimmed.startsWith(t + ':'),
    );
    if (recognized === undefined) {
        return `Not a standard account type — expected one of: ${GL_ACCOUNT_TYPES.join(', ')}.`;
    }

    const newIsBalanceSheet =
        trimmed.startsWith('Assets:') || trimmed.startsWith('Liabilities:');
    for (const old of olds) {
        const oldIsBalanceSheet =
            old.startsWith('Assets:') || old.startsWith('Liabilities:');
        if (oldIsBalanceSheet !== newIsBalanceSheet) {
            return newIsBalanceSheet
                ? 'Converting to a balance-sheet account (Assets/Liabilities) from an income/expense account.'
                : 'Converting away from a balance-sheet account to an income/expense account.';
        }
    }
    return null;
}

/**
 * Suggests account completions for a raw account name draft.
 *
 * - If the draft has no colon yet (user is typing the top-level segment),
 *   prepend matching type prefixes (e.g. "Exp" → "Expenses:") before the
 *   full-account matches so they appear first.
 * - Otherwise filter the accounts list normally.
 * - When oldAccount is provided, suggestions that would trigger a
 *   checkAccountTypeChange warning are silently excluded.
 */
export function getAccountSuggestions(
    draft: string,
    accounts: string[],
    oldAccount?: string | string[],
    maxResults = 8,
): string[] {
    const lower = draft.toLowerCase();
    const suggestions: string[] = [];

    const wouldWarn = (sug: string) =>
        oldAccount != null && checkAccountTypeChange(oldAccount, sug) !== null;

    // Top-level type stubs (only before the first colon)
    if (!draft.includes(':')) {
        for (const type of GL_ACCOUNT_TYPES) {
            if (type.toLowerCase().startsWith(lower)) {
                const stub = type + ':';
                if (!wouldWarn(stub)) suggestions.push(stub);
            }
        }
    }

    // Full account name matches
    const remaining = maxResults - suggestions.length;
    if (remaining > 0) {
        const matches = accounts
            .filter((n) => n.toLowerCase().includes(lower) && !wouldWarn(n))
            .slice(0, remaining);
        suggestions.push(...matches);
    }

    return suggestions;
}

/**
 * Given the current token under the cursor and the cursor's offset
 * within that token, return a list of autocomplete suggestions.
 */
export function getSearchSuggestions(
    token: string,
    cursorOffsetInToken: number,
    accounts: AccountRow[],
): string[] {
    const colonIdx = token.indexOf(':');
    const cursorBeforeOrAtColon =
        colonIdx === -1 || cursorOffsetInToken <= colonIdx;

    if (cursorBeforeOrAtColon) {
        // Cursor is in the prefix part → suggest keyword prefixes
        const typedPrefix =
            colonIdx === -1
                ? token.toLowerCase()
                : token.substring(0, colonIdx).toLowerCase();
        return QUERY_PREFIXES.filter((p) => p.startsWith(typedPrefix));
    }

    // Cursor is after the colon → suggest values
    const prefix = token.substring(0, colonIdx + 1).toLowerCase();
    const after = token.substring(colonIdx + 1);
    const afterLower = after.toLowerCase();

    if (prefix === 'date:' || prefix === 'date2:') {
        return DATE_SMART_TERMS.filter((t) => t.startsWith(afterLower)).map(
            (t) => prefix + t,
        );
    }
    if (prefix === 'acct:' || prefix === 'payee:') {
        return accounts
            .map((a) => a.name)
            .filter((n) => n.toLowerCase().includes(afterLower))
            .slice(0, 10)
            .map((n) => prefix + n);
    }
    if (prefix === 'status:') {
        return ['status:*', 'status:!'].filter((s) =>
            s.startsWith(token.toLowerCase()),
        );
    }
    return [];
}
