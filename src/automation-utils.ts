import type { AutomationProposal, NewResolutionInput } from './tauri-commands';

/**
 * Map an automation proposal to the durable resolution it would create when
 * saved as a standing decision, or `null` if the proposal cannot/should not be
 * promoted.
 *
 * Returns `null` when the proposal is already backed by a resolution
 * (`proposedResult.resolutionId` is set): re-saving it is a silent no-op
 * because `create_resolution` deduplicates by fingerprint, so the "Save
 * decision" action would mislead the user into thinking they created a new
 * rule. Only Review-level heuristics (model suggestions, transfer matches)
 * lack a `resolutionId` and are genuine candidates for promotion.
 */
export function resolutionInputFromProposal(
    proposal: AutomationProposal,
): NewResolutionInput | null {
    if (proposal.proposedResult.resolutionId != null) return null;
    switch (proposal.kind) {
        case 'merge-source':
            return {
                kind: 'same-source',
                subjectRefs: proposal.subjectRefs,
                parts: [],
                notes: 'Saved from automation proposal',
            };
        case 'prevent-merge':
            return {
                kind: 'not-same-source',
                subjectRefs: proposal.subjectRefs,
                parts: [],
                notes: 'Saved from automation proposal',
            };
        case 'retire-pending':
            return {
                kind: 'pending-retired',
                subjectRefs: proposal.subjectRefs,
                parts: [],
                notes: 'Saved from automation proposal',
            };
        case 'post-category': {
            const account = proposal.proposedResult.suggestedAccount?.trim();
            if (account == null || account.length === 0) return null;
            return {
                kind: 'category',
                subjectRefs: proposal.subjectRefs,
                parts: [{ account, amount: null, ref: null, notes: null }],
                notes: 'Saved from automation proposal',
            };
        }
        case 'post-split':
            return {
                kind: 'posting-split',
                subjectRefs: proposal.subjectRefs,
                parts: proposal.proposedResult.parts,
                notes: 'Saved from automation proposal',
            };
        case 'link-transfer':
            return {
                kind: 'transfer-link',
                subjectRefs: proposal.subjectRefs,
                parts: [],
                notes: 'Saved from automation proposal',
            };
        default:
            return null;
    }
}
