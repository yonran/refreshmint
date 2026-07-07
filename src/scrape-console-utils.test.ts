import { describe, it, expect } from 'vitest';
import {
    appendLogLine,
    clampPromptTimeoutMinutes,
    computeStaleLogins,
    formatScrapeOutputLine,
    partitionArtifacts,
    shouldStickToBottom,
} from './scrape-console-utils.ts';

describe('computeStaleLogins', () => {
    const now = Date.parse('2026-07-07T12:00:00Z');

    it('treats a login with no successful scrape as stale', () => {
        expect(
            computeStaleLogins({ chase: { lastSuccess: null } }, 24, now),
        ).toEqual(['chase']);
        expect(computeStaleLogins({ chase: {} }, 24, now)).toEqual(['chase']);
    });

    it('excludes logins scraped within the interval and includes older ones', () => {
        const summaries = {
            fresh: { lastSuccess: '2026-07-07T06:00:00Z' }, // 6h ago
            stale: { lastSuccess: '2026-07-05T06:00:00Z' }, // ~54h ago
        };
        expect(computeStaleLogins(summaries, 24, now)).toEqual(['stale']);
    });
});

describe('shouldStickToBottom', () => {
    it('sticks when scrolled to the exact bottom', () => {
        // scrollTop === scrollHeight - clientHeight => distance 0.
        expect(shouldStickToBottom(900, 1000, 100)).toBe(true);
    });

    it('sticks when within the near-bottom threshold', () => {
        // 20px from the bottom, default threshold 32.
        expect(shouldStickToBottom(880, 1000, 100)).toBe(true);
    });

    it('does not stick when the user has scrolled up past the threshold', () => {
        // 400px from the bottom.
        expect(shouldStickToBottom(500, 1000, 100)).toBe(false);
    });
});

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
