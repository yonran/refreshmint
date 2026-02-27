import type { AccountRow } from './tauri-commands.ts';

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
