import { describe, it, expect } from 'vitest';
import { classifyStatusMessage } from './status-utils.ts';

describe('classifyStatusMessage', () => {
    it('classifies messages mentioning "failed" as errors', () => {
        expect(classifyStatusMessage('Scrape failed')).toBe('error');
        expect(classifyStatusMessage('Login FAILED for chase')).toBe('error');
    });

    it('classifies messages mentioning "error" as errors', () => {
        expect(classifyStatusMessage('Unexpected error')).toBe('error');
        expect(classifyStatusMessage('HTTP Error 500')).toBe('error');
    });

    it('classifies other messages as info', () => {
        expect(classifyStatusMessage('Scrape complete')).toBe('info');
        expect(classifyStatusMessage('Posting 3 entries…')).toBe('info');
        expect(classifyStatusMessage('')).toBe('info');
    });
});
