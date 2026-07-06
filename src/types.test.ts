import { describe, expect, it } from 'vitest';
import { createEmptyPipelineTabSession } from './types.ts';

describe('createEmptyPipelineTabSession', () => {
    it('defaults to the account-rows review queue', () => {
        expect(createEmptyPipelineTabSession().pipelineSubTab).toBe(
            'account-rows',
        );
    });
});
