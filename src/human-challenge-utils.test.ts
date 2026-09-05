import { describe, expect, it } from 'vitest';

import { mapChallengePointer } from './human-challenge-utils.ts';

describe('mapChallengePointer', () => {
    const rect = { left: 100, top: 50, width: 640, height: 450 };
    const viewport = { width: 1280, height: 900 };

    it('maps scaled image coordinates to viewport coordinates', () => {
        expect(mapChallengePointer(420, 275, rect, viewport)).toEqual({
            x: 640,
            y: 450,
        });
    });

    it('clamps pointer coordinates to the streamed viewport', () => {
        expect(mapChallengePointer(0, 600, rect, viewport)).toEqual({
            x: 0,
            y: 900,
        });
    });

    it('rejects empty display or viewport dimensions', () => {
        expect(
            mapChallengePointer(0, 0, { ...rect, width: 0 }, viewport),
        ).toBeNull();
        expect(
            mapChallengePointer(0, 0, rect, { ...viewport, height: 0 }),
        ).toBeNull();
    });
});
