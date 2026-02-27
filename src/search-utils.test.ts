import { describe, it, expect } from 'vitest';
import { getCurrentToken, getSearchSuggestions } from './search-utils.ts';
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
