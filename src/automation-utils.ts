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

// Login-account scope ref for a CategoryRule (no entryId — matches the Rust
// automation::is_login_scope_ref check via category_rule_matches_scope). Restricts
// a rule to one bank account. Shared by PipelineTab's "Always" action and the
// Rules panel.
export function loginScopeRef(loginName: string, label: string): TypedRef {
    return {
        kind: 'login-entry',
        locator: `logins/${loginName}/accounts/${label}`,
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
    notes = 'Created from Review tab transfer link',
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

/** Editable form state for a CategoryRule, used by the Rules panel. */
export interface CategoryRuleFormValues {
    normalizedPayee: string;
    descriptionRegex: string;
    amountMin: string;
    amountMax: string;
    account: string;
    notes: string;
    /** Read-only in the Rules panel today: carried over from the rule being
     * edited, or null (global) when creating a new rule. */
    scope: TypedRef | null;
}

export const EMPTY_CATEGORY_RULE_FORM: CategoryRuleFormValues = {
    normalizedPayee: '',
    descriptionRegex: '',
    amountMin: '',
    amountMax: '',
    account: '',
    notes: '',
    scope: null,
};

/** Populate Rules-panel edit-form fields from an existing CategoryRule resolution. */
export function categoryRuleFormValuesFromResolution(
    rule: Resolution,
): CategoryRuleFormValues {
    const predicate = rule.predicate;
    return {
        normalizedPayee: predicate?.normalizedPayee ?? '',
        descriptionRegex: predicate?.descriptionRegex ?? '',
        amountMin: predicate?.amountMin ?? '',
        amountMax: predicate?.amountMax ?? '',
        account:
            rule.parts.find((p) => p.account != null && p.account !== '')
                ?.account ?? '',
        notes: rule.notes ?? '',
        scope: rule.subjectRefs[0] ?? null,
    };
}

/**
 * Build a NewResolutionInput (kind: 'category-rule') from Rules-panel form
 * values, or null if the form doesn't meet the backend's validation (mirrors
 * Rust automation::validate_category_rule_input): an account and at least one
 * of normalizedPayee/descriptionRegex are required.
 *
 * Pure so it can be vitest-tested. The Rules panel saves an edit by disabling
 * the resolution being edited (if active) and creating a new one from this
 * input — CategoryRule ids are content-addressed by (kind, subjectRefs, parts,
 * predicate), so an in-place field change is a new id, not an update.
 */
export function buildCategoryRuleInput(
    form: CategoryRuleFormValues,
): NewResolutionInput | null {
    const account = form.account.trim();
    const normalizedPayee = form.normalizedPayee.trim();
    const descriptionRegex = form.descriptionRegex.trim();
    if (account === '' || (normalizedPayee === '' && descriptionRegex === '')) {
        return null;
    }
    const amountMin = form.amountMin.trim();
    const amountMax = form.amountMax.trim();
    const notes = form.notes.trim();
    return {
        kind: 'category-rule',
        subjectRefs: form.scope ? [form.scope] : [],
        parts: [{ account, amount: null, ref: null, notes: null }],
        notes: notes === '' ? 'Created from Rules panel' : notes,
        predicate: {
            descriptionRegex: descriptionRegex === '' ? null : descriptionRegex,
            normalizedPayee: normalizedPayee === '' ? null : normalizedPayee,
            amountMin: amountMin === '' ? null : amountMin,
            amountMax: amountMax === '' ? null : amountMax,
        },
    };
}

/** One-line human-readable summary of a CategoryRule for the Rules panel list. */
export function categoryRuleSummary(rule: Resolution): string {
    const predicate = rule.predicate;
    const account =
        rule.parts.find((p) => p.account != null && p.account !== '')
            ?.account ?? '(no account)';
    const matchParts: string[] = [];
    if (
        predicate?.normalizedPayee != null &&
        predicate.normalizedPayee !== ''
    ) {
        matchParts.push(`"${predicate.normalizedPayee}"`);
    }
    if (
        predicate?.descriptionRegex != null &&
        predicate.descriptionRegex !== ''
    ) {
        matchParts.push(`/${predicate.descriptionRegex}/`);
    }
    if (predicate?.amountMin != null || predicate?.amountMax != null) {
        matchParts.push(
            `amount ${predicate.amountMin ?? ''}–${predicate.amountMax ?? ''}`,
        );
    }
    const match = matchParts.length > 0 ? matchParts.join(' ') : '(any)';
    return `${match} → ${account}`;
}
