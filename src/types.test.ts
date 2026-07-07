import { describe, expect, it } from 'vitest';
import {
    createEmptyPipelineTabSession,
    createEmptyReportsTabSession,
} from './types.ts';
import { createDefaultReportConfig } from './report-utils.ts';

describe('createEmptyPipelineTabSession', () => {
    it('defaults to the account-rows review queue', () => {
        expect(createEmptyPipelineTabSession().pipelineSubTab).toBe(
            'account-rows',
        );
    });
});

describe('createEmptyReportsTabSession', () => {
    it('starts with the default config and no run yet', () => {
        const session = createEmptyReportsTabSession();
        expect(session.config).toEqual(createDefaultReportConfig());
        expect(session.result).toBeNull();
        expect(session.error).toBeNull();
        expect(session.lastRunConfig).toBeNull();
        expect(session.hasAutoRun).toBe(false);
    });
});
