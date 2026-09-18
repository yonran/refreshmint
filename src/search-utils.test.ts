import { describe, it, expect } from 'vitest';
import {
    escapeHledgerRegex,
    getCurrentToken,
    getSearchSuggestions,
    quoteHledgerRegex,
    quoteHledgerValue,
} from './search-utils.ts';
import type { AccountRow } from './tauri-commands.ts';

const NO_ACCOUNTS: AccountRow[] = [];
const ACCOUNTS: AccountRow[] = [
    { name: 'Expenses:Food', totals: null, unpostedCount: 0 },
    { name: 'Expenses:Transport', totals: null, unpostedCount: 0 },
    { name: 'Assets:Checking', totals: null, unpostedCount: 0 },
];

describe('getCurrentToken', () => {
    it('finds simple token at cursor', () => {
        expect(getCurrentToken('desc:amazon date:2024', 10)).toEqual({
            token: 'desc:amazon',
            start: 0,
            end: 11,
        });
    });
    it('handles cursor at space boundary', () => {
        expect(getCurrentToken('desc:amazon date:2024', 11)).toEqual({
            token: 'date:2024',
            start: 12,
            end: 21,
        });
    });
    it('handles quoted token spanning whitespace', () => {
        // cursor inside "amazon prime"
        expect(
            getCurrentToken('desc:"amazon prime" date:2024', 14),
        ).toMatchObject({ start: 0, end: 19 });
    });
    it('empty string', () => {
        expect(getCurrentToken('', 0)).toEqual({
            token: '',
            start: 0,
            end: 0,
        });
    });
    it('does not end a double-quoted token at an escaped quote', () => {
        // desc:"a \" b" x  -- cursor inside the quoted span
        expect(getCurrentToken('desc:"a \\" b" x', 8)).toEqual({
            token: 'desc:"a \\" b"',
            start: 0,
            end: 13,
        });
    });
});

// Mirror of the grammar in src-tauri/src/ledger_open.rs tokenize_query.
describe('quoteHledgerValue', () => {
    it('leaves plain values alone', () => {
        expect(quoteHledgerValue('amazon')).toBe('amazon');
        expect(quoteHledgerValue('Expenses:Food')).toBe('Expenses:Food');
    });
    it('single-quotes values with whitespace so backslashes stay readable', () => {
        expect(quoteHledgerValue('amazon prime')).toBe("'amazon prime'");
        expect(quoteHledgerValue('x \\* y')).toBe("'x \\* y'");
    });
    it('falls back to double quotes with escapes when value has a single quote', () => {
        expect(quoteHledgerValue("Trader Joe's")).toBe('"Trader Joe\'s"');
        expect(quoteHledgerValue('it\'s "x" \\ y')).toBe(
            '"it\'s \\"x\\" \\\\ y"',
        );
    });
    it('quotes a value containing a double quote', () => {
        expect(quoteHledgerValue('a"b')).toBe("'a\"b'");
    });
});

describe('escapeHledgerRegex', () => {
    it('escapes every regex metacharacter', () => {
        expect(escapeHledgerRegex('a+b(c)[d]{e}|f^g$h?i.j*k\\l')).toBe(
            'a\\+b\\(c\\)\\[d\\]\\{e\\}\\|f\\^g\\$h\\?i\\.j\\*k\\\\l',
        );
    });
    it('leaves plain text alone', () => {
        expect(escapeHledgerRegex('OPENAI CHATGPT')).toBe('OPENAI CHATGPT');
    });
});

describe('quoteHledgerRegex', () => {
    it('regex-escapes then quotes a bank description', () => {
        expect(
            quoteHledgerRegex('OPENAI *CHATGPT SUBSCR   OPENAI.COM   CA'),
        ).toBe("'OPENAI \\*CHATGPT SUBSCR   OPENAI\\.COM   CA'");
    });
    it('does not quote when no whitespace or quotes', () => {
        expect(quoteHledgerRegex('OPENAI.COM')).toBe('OPENAI\\.COM');
    });
});

describe('getSearchSuggestions', () => {
    it('suggests keyword prefixes for bare text', () => {
        const sugs = getSearchSuggestions('ac', 2, NO_ACCOUNTS);
        expect(sugs).toContain('acct:');
        expect(sugs).not.toContain('desc:');
    });
    it('suggests all keywords for empty token', () => {
        const sugs = getSearchSuggestions('', 0, NO_ACCOUNTS);
        expect(sugs).toContain('desc:');
        expect(sugs).toContain('acct:');
        expect(sugs).toContain('date:');
    });
    it('suggests account names for acct:', () => {
        const sugs = getSearchSuggestions('acct:Exp', 8, ACCOUNTS);
        expect(sugs).toContain('acct:Expenses:Food');
        expect(sugs).toContain('acct:Expenses:Transport');
        expect(sugs).not.toContain('acct:Assets:Checking');
    });
    it('suggests date smart terms', () => {
        const sugs = getSearchSuggestions('date:this', 9, NO_ACCOUNTS);
        expect(sugs).toContain('date:thismonth');
        expect(sugs).toContain('date:thisweek');
        expect(sugs).not.toContain('date:lastmonth');
    });
    it('cursor before colon suggests keywords', () => {
        // token is "acct:Expenses", cursor at position 2 (before colon)
        const sugs = getSearchSuggestions('acct:Expenses', 2, ACCOUNTS);
        expect(sugs).toContain('acct:');
        expect(sugs).not.toContain('acct:Expenses:Food');
    });
    it('suggests status values', () => {
        const sugs = getSearchSuggestions('status:', 7, NO_ACCOUNTS);
        expect(sugs).toContain('status:*');
        expect(sugs).toContain('status:!');
    });
});
