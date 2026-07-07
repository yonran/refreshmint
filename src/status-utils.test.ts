import { describe, it, expect } from 'vitest';
import { classifyStatusMessage, shouldCloseOnKey } from './status-utils.ts';

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

describe('shouldCloseOnKey', () => {
    it('closes on a fresh Escape when enabled', () => {
        expect(shouldCloseOnKey('Escape', false, true)).toBe(true);
    });

    it('does not close on non-Escape keys', () => {
        expect(shouldCloseOnKey('Enter', false, true)).toBe(false);
        expect(shouldCloseOnKey('a', false, true)).toBe(false);
    });

    it('does not close when the event was already handled', () => {
        // An inner control (e.g. AccountInput dismissing its suggestions)
        // preventDefaults the first Escape; the modal must ignore it.
        expect(shouldCloseOnKey('Escape', true, true)).toBe(false);
    });

    it('does not close when closeOnEscape is disabled', () => {
        expect(shouldCloseOnKey('Escape', false, false)).toBe(false);
    });
});
