import type {
    AutomationProposal,
    NewResolutionInput,
    Resolution,
    TypedRef,
} from './tauri-commands';
import { createResolution } from './tauri-commands';

/** A login-entry TypedRef. Mirrors Rust automation::login_entry_ref. */
export function loginEntryRef(
    loginName: string,
    label: string,
    entryId: string,
): TypedRef {
    return {
        kind: 'login-entry',
        locator: `logins/${loginName}/accounts/${label}`,
        entryId,
        loginName,
        label,
    };
}

/**
 * Record a transfer decision as a durable transfer-link resolution. Idempotent
 * via backend fingerprint dedup; creating it disables any active
 * not-transfer-link twin (mutual exclusion in Rust create_resolution). Shared
 * by PipelineTab's Link Transfer actions and App.tsx auto-ETL; CLI post-all
 * records its own via Rust automation::create_transfer_link.
 */
export async function createTransferLinkResolution(
    ledgerPath: string,
    left: { loginName: string; label: string; entryId: string },
    right: { loginName: string; label: string; entryId: string },
    notes = 'Created from Pipeline transfer link',
): Promise<Resolution> {
    return createResolution(ledgerPath, {
        kind: 'transfer-link',
        subjectRefs: [
            loginEntryRef(left.loginName, left.label, left.entryId),
            loginEntryRef(right.loginName, right.label, right.entryId),
        ],
        parts: [],
        notes,
    });
}

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

/**
 * Group bulk-recategorized rows into standing global CategoryRule inputs — one per
 * distinct (normalizedPayee, account) pair. Rows with an empty normalized payee or
 * account are skipped. Dedup here avoids redundant createResolution calls; the
 * backend also dedups by fingerprint, so repeated saves are idempotent.
 *
 * Pure so it can be vitest-tested; callers compute `normalizedPayee` via the
 * normalize_payee Tauri command (mirrors Rust payee_normalize::normalize_payee).
 */
export function categoryRulesFromBulkRows(
    rows: Array<{ normalizedPayee: string; account: string }>,
): NewResolutionInput[] {
    const seen = new Set<string>();
    const rules: NewResolutionInput[] = [];
    for (const row of rows) {
        const normalizedPayee = row.normalizedPayee.trim();
        const account = row.account.trim();
        if (normalizedPayee === '' || account === '') continue;
        const key = `${normalizedPayee}\0${account}`;
        if (seen.has(key)) continue;
        seen.add(key);
        rules.push({
            kind: 'category-rule',
            subjectRefs: [],
            parts: [{ account, amount: null, ref: null, notes: null }],
            notes: 'Created from Transactions bulk recategorize',
            predicate: {
                descriptionRegex: null,
                normalizedPayee,
                amountMin: null,
                amountMax: null,
            },
        });
    }
    return rules;
}
