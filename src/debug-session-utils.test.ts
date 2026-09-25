import { describe, it, expect } from 'vitest';
import { describeDebugSessionStatus } from './debug-session-utils.ts';

describe('describeDebugSessionStatus', () => {
    it('reports an unanswered status', () => {
        expect(describeDebugSessionStatus(null, 0, 0)).toBe(
            'status unavailable',
        );
    });

    it('omits expiry for sessions without a time limit', () => {
        expect(
            describeDebugSessionStatus(
                { execRunning: false, expiresInSecs: null },
                0,
                0,
            ),
        ).toBe('idle');
    });

    it('counts down from when the status was fetched', () => {
        const status = { execRunning: true, expiresInSecs: 1740 };
        expect(describeDebugSessionStatus(status, 1_000, 1_000)).toBe(
            'running script · expires in 29m',
        );
        expect(
            describeDebugSessionStatus(status, 1_000, 1_000 + 1_700_000),
        ).toBe('running script · expires in 40s');
    });

    it('never shows a negative countdown', () => {
        expect(
            describeDebugSessionStatus(
                { execRunning: false, expiresInSecs: 5 },
                0,
                60_000,
            ),
        ).toBe('idle · expires in 0s');
    });

    it('treats a clock reading older than the fetch as no time elapsed', () => {
        expect(
            describeDebugSessionStatus(
                { execRunning: false, expiresInSecs: 600 },
                60_000,
                0,
            ),
        ).toBe('idle · expires in 10m');
    });
});
