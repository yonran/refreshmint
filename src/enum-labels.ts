// Human-readable labels for backend enum unions. This module is the single
// source of truth for how proposal kinds, policy decisions, reversibility,
// resolution kinds/statuses, anomaly kinds, link kinds, period-close statuses,
// and extract/post skip reasons appear in the UI. See docs/glossary.md.
//
// Each table is a typed-exhaustive Record over the corresponding union imported
// from tauri-commands.ts / types.ts, so adding a variant to a union is a compile
// error here until a label is provided. Look up labels via enumLabel(); unknown
// values fall back to the raw string.

import type {
    AutomationProposalKind,
    ImportAnomalyKind,
    LinkKind,
    PeriodCloseStatus,
    ProposalPolicyDecision,
    ProposalReversibility,
    ReconciliationSession,
    ResolutionKind,
    ResolutionStatus,
} from './tauri-commands.ts';
import type { ExtractSkipReason, PostSkipReason } from './types.ts';

export interface EnumLabel {
    label: string;
    description?: string;
}

export type EnumLabelTable<K extends string> = Record<K, EnumLabel>;

/** Human label for `value` in `table`, falling back to the raw string. */
export function enumLabel<K extends string>(
    table: EnumLabelTable<K>,
    value: string,
): string {
    return (
        (table as Record<string, EnumLabel | undefined>)[value]?.label ?? value
    );
}

/** Optional long-form description for `value` in `table`, if one is defined. */
export function enumDescription<K extends string>(
    table: EnumLabelTable<K>,
    value: string,
): string | undefined {
    return (table as Record<string, EnumLabel | undefined>)[value]?.description;
}

export const PROPOSAL_KIND_LABELS: EnumLabelTable<AutomationProposalKind> = {
    'merge-source': { label: 'Merge sources' },
    'prevent-merge': { label: 'Keep separate' },
    'retire-pending': { label: 'Retire pending entry' },
    'post-category': { label: 'Post with category' },
    'post-split': { label: 'Post as split' },
    'link-transfer': { label: 'Link transfer' },
    'merge-gl-transfer': { label: 'Merge as GL transfer' },
    'recategorize-gl': { label: 'Recategorize' },
    'sync-posted': { label: 'Sync posted' },
    'review-anomaly': { label: 'Review anomaly' },
};

export const POLICY_DECISION_LABELS: EnumLabelTable<ProposalPolicyDecision> = {
    auto: { label: 'Automatic' },
    review: { label: 'Needs review' },
    blocked: { label: 'Blocked' },
    skip: { label: 'Skipped' },
};

export const REVERSIBILITY_LABELS: EnumLabelTable<ProposalReversibility> = {
    yes: { label: 'Undoable' },
    conditional: { label: 'Conditionally undoable' },
    no: { label: 'Not undoable' },
};

export const RESOLUTION_KIND_LABELS: EnumLabelTable<ResolutionKind> = {
    'same-source': { label: 'Same source' },
    'not-same-source': { label: 'Not same source' },
    category: { label: 'Category' },
    'category-rule': { label: 'Category rule' },
    'posting-split': { label: 'Split' },
    'transfer-link': { label: 'Transfer' },
    'not-transfer-link': { label: 'Not a transfer' },
    'transfer-split': { label: 'Transfer split' },
    'ignore-source': { label: 'Ignore source' },
    'pending-retired': { label: 'Pending retired' },
    'reversal-link': { label: 'Reversal' },
};

export const RESOLUTION_STATUS_LABELS: EnumLabelTable<ResolutionStatus> = {
    active: { label: 'Active' },
    disabled: { label: 'Disabled' },
};

export const ANOMALY_KIND_LABELS: EnumLabelTable<ImportAnomalyKind> = {
    'finalized-missing-from-covered-export': {
        label: 'Finalized entry missing from export',
    },
    'unsafe-pending-retirement': { label: 'Unsafe pending retirement' },
    'duplicate-import-repair-skipped': {
        label: 'Duplicate import (repair skipped)',
    },
    'posted-leg-amount-drift': { label: 'Posted amount drift' },
    'coverage-info-missing': { label: 'Coverage info missing' },
};

export const LINK_KIND_LABELS: EnumLabelTable<LinkKind> = {
    'settlement-link': {
        label: 'Settlement',
        description:
            'Resolves an open balance-sheet position such as an accrual, deferral, receivable, or payable.',
    },
    'evidence-link': {
        label: 'Evidence',
        description: 'Supporting document or statement line for a transaction.',
    },
    'source-link': {
        label: 'Source',
        description:
            'Ties a GL transaction back to the source entry it was posted from.',
    },
};

export const PERIOD_CLOSE_STATUS_LABELS: EnumLabelTable<PeriodCloseStatus> = {
    draft: { label: 'Draft' },
    'soft-closed': { label: 'Soft closed' },
    reopened: { label: 'Reopened' },
};

export const RECONCILIATION_SESSION_STATUS_LABELS: EnumLabelTable<
    ReconciliationSession['status']
> = {
    draft: { label: 'Draft' },
    finalized: { label: 'Finalized' },
    reopened: { label: 'Reopened' },
};

export const EXTRACT_SKIP_REASON_LABELS: EnumLabelTable<ExtractSkipReason> = {
    'missing-extension': { label: 'No extension' },
    'missing-extractor': { label: 'No extractor' },
    'broken-extractor': { label: 'Extractor broken' },
    'no-documents': { label: 'No documents' },
};

export const POST_SKIP_REASON_LABELS: EnumLabelTable<PostSkipReason> = {
    'missing-gl-account': { label: 'No GL mapping' },
    'no-unposted': { label: 'Up to date' },
};
