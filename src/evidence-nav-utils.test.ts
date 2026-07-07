import { describe, it, expect } from 'vitest';
import {
    parseGlSourceRefs,
    parseLoginAccountLocator,
    pickEvidenceTarget,
} from './evidence-nav-utils.ts';

describe('parseGlSourceRefs', () => {
    it('parses source lines and splits on the last colon', () => {
        const comment = [
            '; generated-by: refreshmint-post',
            '; source: logins/chase/accounts/checking:abc-123',
            '; source: logins/amex/accounts/gold:def-456',
        ].join('\n');
        expect(parseGlSourceRefs(comment)).toEqual([
            { locator: 'logins/chase/accounts/checking', entryId: 'abc-123' },
            { locator: 'logins/amex/accounts/gold', entryId: 'def-456' },
        ]);
    });

    it('skips per-leg :posting: refs', () => {
        const comment =
            '; source: logins/chase/accounts/checking:abc-123:posting:0';
        expect(parseGlSourceRefs(comment)).toEqual([]);
    });

    it('ignores non-source and malformed lines', () => {
        const comment = ['; id: 1', '; source: ', 'not a comment'].join('\n');
        expect(parseGlSourceRefs(comment)).toEqual([]);
    });
});

describe('parseLoginAccountLocator', () => {
    it('splits a login-account locator', () => {
        expect(
            parseLoginAccountLocator('logins/chase/accounts/checking'),
        ).toEqual({ loginName: 'chase', label: 'checking' });
    });

    it('returns null for non-login-account locators', () => {
        expect(parseLoginAccountLocator('accounts/checking')).toBeNull();
    });
});

describe('pickEvidenceTarget', () => {
    it('extracts document and row from the first parseable ref', () => {
        expect(pickEvidenceTarget(['statements/2026/jan.pdf:12:3'])).toEqual({
            document: 'statements/2026/jan.pdf',
            row: 12,
        });
    });

    it('returns null row when the row is not numeric', () => {
        expect(pickEvidenceTarget(['doc.pdf:x:3'])).toEqual({
            document: 'doc.pdf',
            row: null,
        });
    });

    it('returns null when no ref has the document:row:col shape', () => {
        expect(pickEvidenceTarget(['doc.pdf', ''])).toBeNull();
    });
});
