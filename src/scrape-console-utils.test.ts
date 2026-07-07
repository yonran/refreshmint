import { describe, it, expect } from 'vitest';
import {
    appendLogLine,
    clampPromptTimeoutMinutes,
    formatScrapeOutputLine,
    partitionArtifacts,
} from './scrape-console-utils.ts';

describe('clampPromptTimeoutMinutes', () => {
    it('falls back to the default for non-finite input', () => {
        expect(clampPromptTimeoutMinutes(NaN)).toBe(5);
        expect(clampPromptTimeoutMinutes(Infinity)).toBe(5);
    });

    it('clamps below 1 up to 1 and floors fractional values', () => {
        expect(clampPromptTimeoutMinutes(0)).toBe(1);
        expect(clampPromptTimeoutMinutes(-3)).toBe(1);
        expect(clampPromptTimeoutMinutes(2.7)).toBe(2);
    });

    it('clamps above the max down to the max', () => {
        expect(clampPromptTimeoutMinutes(100)).toBe(60);
        expect(clampPromptTimeoutMinutes(7)).toBe(7);
    });
});

describe('partitionArtifacts', () => {
    it('separates the png screenshot from text artifacts', () => {
        expect(
            partitionArtifacts(['log-tail.txt', 'screenshot.png', 'url.txt']),
        ).toEqual({
            imageName: 'screenshot.png',
            textNames: ['log-tail.txt', 'url.txt'],
        });
    });

    it('returns null image when no png is present', () => {
        expect(partitionArtifacts(['url.txt', 'log-tail.txt'])).toEqual({
            imageName: null,
            textNames: ['log-tail.txt', 'url.txt'],
        });
    });
});

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
