import { describe, expect, it } from 'vitest';
import {
    ANOMALY_KIND_LABELS,
    enumDescription,
    enumLabel,
    LINK_KIND_LABELS,
    POLICY_DECISION_LABELS,
    PROPOSAL_KIND_LABELS,
    RESOLUTION_KIND_LABELS,
    RECONCILIATION_SESSION_STATUS_LABELS,
} from './enum-labels.ts';

describe('enumLabel', () => {
    it('returns the human label for a known value', () => {
        expect(enumLabel(PROPOSAL_KIND_LABELS, 'merge-source')).toBe(
            'Merge sources',
        );
        expect(enumLabel(POLICY_DECISION_LABELS, 'auto')).toBe('Automatic');
        expect(enumLabel(RESOLUTION_KIND_LABELS, 'transfer-link')).toBe(
            'Transfer',
        );
    });

    it('falls back to the raw string for an unknown value', () => {
        expect(enumLabel(PROPOSAL_KIND_LABELS, 'no-such-kind')).toBe(
            'no-such-kind',
        );
        expect(enumLabel(RESOLUTION_KIND_LABELS, '')).toBe('');
    });
});

describe('enumDescription', () => {
    it('returns the description when one is defined', () => {
        expect(enumDescription(LINK_KIND_LABELS, 'settlement-link')).toContain(
            'balance-sheet',
        );
    });

    it('returns undefined when no description is defined', () => {
        expect(
            enumDescription(
                ANOMALY_KIND_LABELS,
                'finalized-missing-from-covered-export',
            ),
        ).toBeUndefined();
    });

    it('returns undefined for an unknown value', () => {
        expect(
            enumDescription(LINK_KIND_LABELS, 'no-such-kind'),
        ).toBeUndefined();
    });
});

describe('RECONCILIATION_SESSION_STATUS_LABELS', () => {
    it('maps session statuses to human labels', () => {
        expect(
            enumLabel(RECONCILIATION_SESSION_STATUS_LABELS, 'finalized'),
        ).toBe('Finalized');
        expect(enumLabel(RECONCILIATION_SESSION_STATUS_LABELS, 'draft')).toBe(
            'Draft',
        );
    });
});
