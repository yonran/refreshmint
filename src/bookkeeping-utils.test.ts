import { describe, expect, it } from 'vitest';
import { appendIdToList } from './bookkeeping-utils.ts';

describe('appendIdToList', () => {
    it('adds the id to an empty list', () => {
        expect(appendIdToList('', 'sess-1')).toBe('sess-1');
    });

    it('appends the id to an existing comma-separated list', () => {
        expect(appendIdToList('sess-1, sess-2', 'sess-3')).toBe(
            'sess-1, sess-2, sess-3',
        );
    });

    it('does not add a duplicate id', () => {
        expect(appendIdToList('sess-1, sess-2', 'sess-2')).toBe(
            'sess-1, sess-2',
        );
    });

    it('trims surrounding whitespace on the added id', () => {
        expect(appendIdToList('sess-1', '  sess-2  ')).toBe('sess-1, sess-2');
    });

    it('normalizes newline-separated entries when appending', () => {
        expect(appendIdToList('sess-1\nsess-2', 'sess-3')).toBe(
            'sess-1, sess-2, sess-3',
        );
    });

    it('returns the list unchanged for an empty id', () => {
        expect(appendIdToList('sess-1', '   ')).toBe('sess-1');
    });
});
