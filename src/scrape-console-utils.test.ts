import { describe, it, expect } from 'vitest';
import {
    appendLogLine,
    formatScrapeOutputLine,
} from './scrape-console-utils.ts';

describe('formatScrapeOutputLine', () => {
    it('passes stdout lines through unchanged', () => {
        expect(
            formatScrapeOutputLine({ stream: 'stdout', line: 'hello' }),
        ).toBe('hello');
    });

    it('prefixes stderr lines', () => {
        expect(formatScrapeOutputLine({ stream: 'stderr', line: 'boom' })).toBe(
            '[stderr] boom',
        );
    });
});

describe('appendLogLine', () => {
    it('appends without mutating the input array', () => {
        const lines = ['a'];
        const next = appendLogLine(lines, { stream: 'stdout', line: 'b' });
        expect(next).toEqual(['a', 'b']);
        expect(lines).toEqual(['a']);
    });

    it('caps at max, evicting the oldest lines and preserving order', () => {
        let lines: string[] = [];
        for (let i = 0; i < 5; i++) {
            lines = appendLogLine(
                lines,
                { stream: 'stdout', line: `l${i}` },
                3,
            );
        }
        expect(lines).toEqual(['l2', 'l3', 'l4']);
    });
});
