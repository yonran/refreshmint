import { describe, expect, it } from 'vitest';
import { resolveSidecarTarget } from './sidecar-target.mjs';

const host = 'aarch64-apple-darwin';

describe('resolveSidecarTarget', () => {
    it('builds into the plain profile dir when no target is requested', () => {
        expect(
            resolveSidecarTarget({ explicitTarget: undefined, host }),
        ).toEqual({
            target: host,
            cargoTargetArg: undefined,
            builtSubdir: [],
        });
    });

    it('drops --target when the requested target is the host (tauri dev sets it)', () => {
        expect(resolveSidecarTarget({ explicitTarget: host, host })).toEqual({
            target: host,
            cargoTargetArg: undefined,
            builtSubdir: [],
        });
    });

    it('keeps --target and the per-triple dir for a cross-compile', () => {
        const cross = 'x86_64-unknown-linux-gnu';
        expect(resolveSidecarTarget({ explicitTarget: cross, host })).toEqual({
            target: cross,
            cargoTargetArg: cross,
            builtSubdir: [cross],
        });
    });
});
