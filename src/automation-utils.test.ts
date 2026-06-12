import { describe, it, expect } from 'vitest';
import { resolutionInputFromProposal } from './automation-utils.ts';
import type {
    AutomationProposal,
    AutomationProposalKind,
    ProposalResult,
} from './tauri-commands.ts';

function makeProposal(
    kind: AutomationProposalKind,
    proposedResult: Partial<ProposalResult> = {},
): AutomationProposal {
    return {
        id: 'p1',
        kind,
        subjectRefs: [{ kind: 'login-entry', entryId: 'entry-1' }],
        proposedResult: {
            suggestedAccount: null,
            transferMatch: null,
            parts: [],
            importAnomalyId: null,
            resolutionId: null,
            notes: null,
            ...proposedResult,
        },
        reasons: [],
        blockers: [],
        canApply: true,
        policyDecision: 'review',
        reversible: 'yes',
    };
}

describe('resolutionInputFromProposal', () => {
    it('promotes a model-suggested category proposal', () => {
        const input = resolutionInputFromProposal(
            makeProposal('post-category', {
                suggestedAccount: 'Expenses:Dining',
            }),
        );
        expect(input?.kind).toBe('category');
        expect(input?.parts).toEqual([
            {
                account: 'Expenses:Dining',
                amount: null,
                ref: null,
                notes: null,
            },
        ]);
    });

    it('refuses a proposal already backed by a resolution', () => {
        // An Auto proposal derived from an existing resolution carries a
        // resolutionId; re-saving it would be a silent no-op, so it must not
        // be offered as a new decision.
        const input = resolutionInputFromProposal(
            makeProposal('post-category', {
                suggestedAccount: 'Expenses:Dining',
                resolutionId: 'res-1',
            }),
        );
        expect(input).toBeNull();
    });

    it('refuses a category proposal with no suggested account', () => {
        expect(
            resolutionInputFromProposal(makeProposal('post-category')),
        ).toBeNull();
    });

    it('refuses non-promotable proposal kinds', () => {
        expect(
            resolutionInputFromProposal(makeProposal('merge-gl-transfer')),
        ).toBeNull();
        expect(
            resolutionInputFromProposal(makeProposal('sync-posted')),
        ).toBeNull();
    });
});
