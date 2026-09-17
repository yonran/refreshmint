import { describe, it, expect } from 'vitest';
import {
    buildCategoryRuleInput,
    categoryRuleFormValuesFromResolution,
    categoryRuleSummary,
    categoryRulesFromBulkRows,
    EMPTY_CATEGORY_RULE_FORM,
    resolutionInputFromProposal,
} from './automation-utils.ts';
import type {
    AutomationProposal,
    AutomationProposalKind,
    ProposalResult,
    Resolution,
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

describe('categoryRulesFromBulkRows', () => {
    it('produces one global rule per distinct payee/account pair', () => {
        const rules = categoryRulesFromBulkRows([
            { normalizedPayee: 'SAFEWAY', account: 'Expenses:Groceries' },
            { normalizedPayee: 'STARBUCKS', account: 'Expenses:Dining' },
        ]);
        expect(rules).toHaveLength(2);
        expect(rules[0]).toMatchObject({
            kind: 'category-rule',
            subjectRefs: [],
            parts: [{ account: 'Expenses:Groceries' }],
            predicate: { normalizedPayee: 'SAFEWAY' },
        });
    });

    it('dedups repeated payee/account pairs', () => {
        const rules = categoryRulesFromBulkRows([
            { normalizedPayee: 'SAFEWAY', account: 'Expenses:Groceries' },
            { normalizedPayee: 'SAFEWAY', account: 'Expenses:Groceries' },
        ]);
        expect(rules).toHaveLength(1);
    });

    it('keeps distinct accounts for the same payee as separate rules', () => {
        const rules = categoryRulesFromBulkRows([
            { normalizedPayee: 'AMAZON', account: 'Expenses:Shopping' },
            { normalizedPayee: 'AMAZON', account: 'Expenses:Office' },
        ]);
        expect(rules).toHaveLength(2);
    });

    it('skips rows with an empty payee or account', () => {
        const rules = categoryRulesFromBulkRows([
            { normalizedPayee: '', account: 'Expenses:Groceries' },
            { normalizedPayee: 'SAFEWAY', account: '  ' },
            { normalizedPayee: 'COSTCO', account: 'Expenses:Groceries' },
        ]);
        expect(rules).toHaveLength(1);
        expect(rules[0]?.predicate?.normalizedPayee).toBe('COSTCO');
    });
});

function makeRule(overrides: Partial<Resolution> = {}): Resolution {
    return {
        id: 'r1',
        kind: 'category-rule',
        status: 'active',
        subjectRefs: [],
        parts: [
            {
                account: 'Expenses:Groceries',
                amount: null,
                ref: null,
                notes: null,
            },
        ],
        notes: 'Created from Rules panel',
        predicate: { normalizedPayee: 'SAFEWAY' },
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
        ...overrides,
    };
}

describe('buildCategoryRuleInput', () => {
    it('rejects an empty account', () => {
        expect(
            buildCategoryRuleInput({
                ...EMPTY_CATEGORY_RULE_FORM,
                normalizedPayee: 'SAFEWAY',
            }),
        ).toBeNull();
    });

    it('rejects a form with no predicate field set', () => {
        expect(
            buildCategoryRuleInput({
                ...EMPTY_CATEGORY_RULE_FORM,
                account: 'Expenses:Groceries',
            }),
        ).toBeNull();
    });

    it('builds a global rule from a normalizedPayee match', () => {
        const input = buildCategoryRuleInput({
            ...EMPTY_CATEGORY_RULE_FORM,
            normalizedPayee: '  SAFEWAY  ',
            account: '  Expenses:Groceries  ',
        });
        expect(input).toEqual({
            kind: 'category-rule',
            subjectRefs: [],
            parts: [
                {
                    account: 'Expenses:Groceries',
                    amount: null,
                    ref: null,
                    notes: null,
                },
            ],
            notes: 'Created from Rules panel',
            predicate: {
                descriptionRegex: null,
                normalizedPayee: 'SAFEWAY',
                amountMin: null,
                amountMax: null,
            },
        });
    });

    it('accepts a descriptionRegex-only match and carries amount bounds + scope', () => {
        const scope = {
            kind: 'login-entry' as const,
            loginName: 'chase',
            label: 'checking',
        };
        const input = buildCategoryRuleInput({
            ...EMPTY_CATEGORY_RULE_FORM,
            descriptionRegex: 'STARBUCKS.*',
            account: 'Expenses:Dining',
            amountMin: '1',
            amountMax: '50',
            notes: 'coffee',
            scope,
        });
        expect(input?.subjectRefs).toEqual([scope]);
        expect(input?.notes).toBe('coffee');
        expect(input?.predicate).toEqual({
            descriptionRegex: 'STARBUCKS.*',
            normalizedPayee: null,
            amountMin: '1',
            amountMax: '50',
        });
    });
});

describe('categoryRuleFormValuesFromResolution', () => {
    it('round-trips a rule into editable form fields', () => {
        const rule = makeRule({
            predicate: {
                normalizedPayee: 'SAFEWAY',
                descriptionRegex: null,
                amountMin: '1',
                amountMax: null,
            },
        });
        expect(categoryRuleFormValuesFromResolution(rule)).toEqual({
            normalizedPayee: 'SAFEWAY',
            descriptionRegex: '',
            amountMin: '1',
            amountMax: '',
            account: 'Expenses:Groceries',
            notes: 'Created from Rules panel',
            scope: null,
        });
    });
});

describe('categoryRuleSummary', () => {
    it('summarizes a payee-matched rule', () => {
        expect(categoryRuleSummary(makeRule())).toBe(
            '"SAFEWAY" → Expenses:Groceries',
        );
    });

    it('includes amount bounds when set', () => {
        const rule = makeRule({
            predicate: {
                normalizedPayee: 'STARBUCKS',
                amountMin: '1',
                amountMax: '10',
            },
        });
        expect(categoryRuleSummary(rule)).toBe(
            '"STARBUCKS" amount 1–10 → Expenses:Groceries',
        );
    });

    it('falls back to (any) when no predicate field is set', () => {
        const rule = makeRule({ predicate: null });
        expect(categoryRuleSummary(rule)).toBe('(any) → Expenses:Groceries');
    });
});
