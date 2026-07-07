import {
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
} from 'react';
import {
    type GlCategoryResult,
    type PostingRow,
    type TransactionRow,
    UNCATEGORIZED_GL_ACCOUNT,
} from '../tauri-commands.ts';
import { quoteHledgerValue } from '../search-utils.ts';
import { parseGlSourceRefs, type GlSourceRef } from '../evidence-nav-utils.ts';
import type { SimilarRecategorizeSeed } from '../types.ts';
import {
    type AcceptAllEdit,
    buildAcceptAllEdits,
    categorizeChipLabel,
    mergeTransferChipLabel,
} from '../categorize-utils.ts';
import { formatScaled, formatTotals } from '../amount-utils.ts';
import { AccountInput } from '../components/AccountInput.tsx';
import { AttachmentLightbox } from '../components/AttachmentLightbox.tsx';
import { useAttachmentLightbox } from '../components/useAttachmentLightbox.ts';
import { BulkRecategorizeConfirmModal } from '../components/BulkRecategorizeConfirmModal.tsx';
import {
    attachmentFilename,
    isImageAttachmentRef,
} from '../attachment-utils.ts';

// A transaction's posting amounts are "obvious" — and therefore redundant to
// display — when there are exactly 2 postings and exactly 1 is a balance-sheet
// account (Assets/Liabilities). In that case both amounts equal the net-worth
// effect shown in the Amount column (one is it, the other is its negation),
// so showing them clutters the Postings column without adding information.
function hasObviousAmounts(txn: TransactionRow): boolean {
    if (txn.postings.length !== 2) return false;
    const balanceSheetCount = txn.postings.filter(
        (p) =>
            p.account.startsWith('Assets:') ||
            p.account.startsWith('Liabilities:'),
    ).length;
    return balanceSheetCount === 1;
}

function singleNonBalancingPosting(txn: TransactionRow): PostingRow | null {
    const candidates = txn.postings.filter(
        (p) =>
            !p.account.startsWith('Assets:') &&
            !p.account.startsWith('Liabilities:'),
    );
    return candidates.length === 1 ? (candidates[0] ?? null) : null;
}

function singleNonBalancingPostingWithIndex(
    txn: TransactionRow,
): { posting: PostingRow; postingIndex: number } | null {
    const candidates = txn.postings
        .map((posting, postingIndex) => ({ posting, postingIndex }))
        .filter(
            ({ posting }) =>
                !posting.account.startsWith('Assets:') &&
                !posting.account.startsWith('Liabilities:'),
        );
    return candidates.length === 1 ? (candidates[0] ?? null) : null;
}

function similarityGroupKey(txn: TransactionRow): string | null {
    if (!singleNonBalancingPosting(txn)) return null;
    const balancing = txn.postings.find(
        (p) =>
            p.account.startsWith('Assets:') ||
            p.account.startsWith('Liabilities:'),
    );
    if (!balancing) return null;
    return `${txn.description}\0${balancing.account}`;
}

/** Key for grouping "similar" uncategorized transactions: same description + same balancing account. */
function similarKey(txn: TransactionRow): string | null {
    const posting = singleNonBalancingPosting(txn);
    if (!posting || posting.account !== UNCATEGORIZED_GL_ACCOUNT) return null;
    return similarityGroupKey(txn);
}

function badgeLabel(base: string, count: number): string {
    return count > 1 ? `${base} ×${count}` : base;
}

function bookkeepingBadges(txn: TransactionRow): string[] {
    const badges: string[] = [];
    if (txn.bookkeeping.generated) {
        badges.push('generated');
    }
    if (txn.bookkeeping.reconciledSessionIds.length > 0) {
        badges.push(
            badgeLabel(
                'reconciled',
                txn.bookkeeping.reconciledSessionIds.length,
            ),
        );
    }
    if (txn.bookkeeping.settlementLinkIds.length > 0) {
        badges.push(
            badgeLabel('settled', txn.bookkeeping.settlementLinkIds.length),
        );
    } else if (txn.bookkeeping.linkedRecordIds.length > 0) {
        badges.push(
            badgeLabel('linked', txn.bookkeeping.linkedRecordIds.length),
        );
    }
    if (txn.bookkeeping.softClosedPeriodId !== null) {
        badges.push(`soft-closed ${txn.bookkeeping.softClosedPeriodId}`);
    }
    return badges;
}

export function PostingsList({
    postings,
    hideAmounts = false,
}: {
    postings: PostingRow[];
    hideAmounts?: boolean;
}) {
    return (
        <div className="postings-list">
            {postings.map((posting) => (
                <div key={posting.account} className="postings-item">
                    <span>{posting.account}</span>
                    {!hideAmounts && (
                        <span className="amount">
                            {formatTotals(posting.totals)}
                        </span>
                    )}
                </div>
            ))}
        </div>
    );
}

export function TransactionsTable({
    transactions,
    ledgerPath,
    accountNames = [],
    glCategorySuggestions = {},
    selectedTransactionIds,
    onSelectedTransactionIdsChange,
    initialScrollTop,
    onScrollTopChange,
    onRecategorize,
    onMergeTransfer,
    onOpenLinkTransfer,
    onUnmergeTransfer,
    onNotATransfer,
    onBulkRecategorize,
    onAcceptSuggestions,
    acceptSuggestionsBusy = false,
    recategorizeBusy = false,
    transferActionBusy = false,
    onOpenSimilarRecategorize,
    hideObviousAmounts = true,
    onAddSearchTerm,
    onOpenEvidence,
}: {
    transactions: TransactionRow[];
    ledgerPath: string | null;
    accountNames?: string[];
    glCategorySuggestions?: Record<string, GlCategoryResult>;
    selectedTransactionIds?: string[];
    onSelectedTransactionIdsChange?: (ids: string[]) => void;
    initialScrollTop?: number;
    onScrollTopChange?: (scrollTop: number) => void;
    onRecategorize?: (
        txnId: string,
        postingIndex: number,
        newAccount: string,
        // Current account of the posting, so callers can build an exact Undo.
        oldAccount: string,
    ) => void;
    onMergeTransfer?: (txnId1: string, txnId2: string) => void;
    onOpenLinkTransfer?: (txnId: string) => void;
    // Unpost a generated multi-source (transfer) txn back to its account
    // entries (context menu "Unmerge transfer").
    onUnmergeTransfer?: (txnId: string) => void;
    // Record not-a-transfer negative memory for (txn, its transferMatch)
    // (context menu "Not a transfer").
    onNotATransfer?: (txnId1: string, txnId2: string) => void;
    onBulkRecategorize?: (
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
            // Raw bank description, used to build CategoryRules when createRule is set.
            description: string;
        }>,
        newAccount: string,
        createRule: boolean,
    ) => void;
    // Apply every visible ML suggestion (per-row target account) in one batch.
    onAcceptSuggestions?: (edits: AcceptAllEdit[]) => void;
    // True while a batch accept is running, to disable the trigger buttons.
    acceptSuggestionsBusy?: boolean;
    // True while a single-row categorize chip's recategorize is running, to
    // disable the categorize chip so a double-click can't fire a second
    // concurrent recategorize.
    recategorizeBusy?: boolean;
    // True while a merge/unmerge/not-a-transfer action is running, to disable the
    // ↔ merge chip so a double-click can't fire a second concurrent merge.
    transferActionBusy?: boolean;
    onOpenSimilarRecategorize?: (seed: SimilarRecategorizeSeed) => void;
    hideObviousAmounts?: boolean;
    onAddSearchTerm?: (term: string) => void;
    // Navigate from a GL transaction's `; source:` ref to the originating
    // account entry's evidence rows (Pipeline tab). Threaded App → TransactionsTab.
    onOpenEvidence?: (ref: GlSourceRef) => void;
}) {
    const lightbox = useAttachmentLightbox(ledgerPath);
    const [expandedEvidenceIds, setExpandedEvidenceIds] = useState<
        ReadonlySet<string>
    >(new Set());
    const [editingKey, setEditingKey] = useState<string | null>(null); // `${txnId}:${postingIndex}`
    const [categoryDraft, setCategoryDraft] = useState('');
    const [uncontrolledSelectedIds, setUncontrolledSelectedIds] = useState<
        ReadonlySet<string>
    >(new Set());
    const [bulkDraft, setBulkDraft] = useState('');
    // When set, also create a standing CategoryRule per selected payee on apply.
    const [bulkCreateRule, setBulkCreateRule] = useState(false);
    const [bulkConfirm, setBulkConfirm] = useState<{
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
            description: string;
        }>;
        newAccount: string;
        createRule: boolean;
    } | null>(null);
    // Pending "Accept N suggestions" batch, awaiting confirm.
    const [acceptAllConfirm, setAcceptAllConfirm] = useState<
        AcceptAllEdit[] | null
    >(null);

    const similarGroupIds = useMemo(() => {
        const map = new Map<string, string[]>();
        for (const txn of transactions) {
            const key = similarKey(txn);
            if (key == null) continue;
            let arr = map.get(key);
            if (!arr) {
                arr = [];
                map.set(key, arr);
            }
            arr.push(txn.id);
        }
        return map;
    }, [transactions]);

    type ContextMenuItem = { label: string; action: () => void };
    const [contextMenu, setContextMenu] = useState<{
        x: number;
        y: number;
        items: ContextMenuItem[];
    } | null>(null);

    function openContextMenu(e: React.MouseEvent, items: ContextMenuItem[]) {
        e.preventDefault();
        e.stopPropagation();
        setContextMenu({ x: e.clientX, y: e.clientY, items });
    }

    useEffect(() => {
        if (!contextMenu) return;
        const close = () => {
            setContextMenu(null);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') setContextMenu(null);
        };
        document.addEventListener('mousedown', close);
        document.addEventListener('keydown', onKey);
        return () => {
            document.removeEventListener('mousedown', close);
            document.removeEventListener('keydown', onKey);
        };
    }, [contextMenu]);

    const hasActions =
        onRecategorize !== undefined ||
        onMergeTransfer !== undefined ||
        onOpenLinkTransfer !== undefined;
    const hasCheckbox = onBulkRecategorize !== undefined;
    const tableWrapRef = useRef<HTMLDivElement>(null);
    const selectedIds = useMemo(
        () =>
            selectedTransactionIds !== undefined
                ? new Set(selectedTransactionIds)
                : uncontrolledSelectedIds,
        [selectedTransactionIds, uncontrolledSelectedIds],
    );

    const updateSelectedIds = useCallback(
        (updater: (prev: ReadonlySet<string>) => ReadonlySet<string>) => {
            const next = updater(selectedIds);
            if (selectedTransactionIds !== undefined) {
                onSelectedTransactionIdsChange?.([...next]);
                return;
            }
            setUncontrolledSelectedIds(next);
        },
        [onSelectedTransactionIdsChange, selectedIds, selectedTransactionIds],
    );

    useLayoutEffect(() => {
        if (initialScrollTop === undefined) {
            return;
        }
        const node = tableWrapRef.current;
        if (!node) {
            return;
        }
        if (Math.abs(node.scrollTop - initialScrollTop) > 1) {
            node.scrollTop = initialScrollTop;
        }
    }, [initialScrollTop]);

    useEffect(() => {
        if (selectedIds.size === 0) {
            return;
        }
        const visibleIds = new Set(transactions.map((txn) => txn.id));
        const nextSelectedIds = [...selectedIds].filter((id) =>
            visibleIds.has(id),
        );
        if (nextSelectedIds.length === selectedIds.size) {
            return;
        }
        // Pre-existing derived-state sync (prune selection to visible rows);
        // surfaced by the React Compiler lint once this component became
        // simpler to analyze. Behavior unchanged.
        // eslint-disable-next-line react-hooks/set-state-in-effect
        updateSelectedIds(() => new Set(nextSelectedIds));
    }, [selectedIds, transactions, updateSelectedIds]);

    useEffect(() => {
        if (selectedIds.size === 0 || bulkDraft !== '') return;
        const tally = new Map<string, number>();
        for (const id of selectedIds) {
            const s = glCategorySuggestions[id]?.suggested;
            if (s != null) tally.set(s, (tally.get(s) ?? 0) + 1);
        }
        if (tally.size === 0) return;
        const best = [...tally.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
        // Pre-existing derived-state sync (seed bulk draft from suggestions);
        // surfaced by the React Compiler lint. Behavior unchanged.
        // eslint-disable-next-line react-hooks/set-state-in-effect
        if (best !== undefined) setBulkDraft(best);
    }, [selectedIds]); // eslint-disable-line react-hooks/exhaustive-deps

    function applyBulk(
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
            description: string;
        }>,
        newAccount: string,
    ) {
        if (onBulkRecategorize === undefined) return;
        const accounts = new Set(entries.map((e) => e.oldAccount));
        if (accounts.size <= 1) {
            onBulkRecategorize(entries, newAccount, bulkCreateRule);
            updateSelectedIds(() => new Set());
            setBulkDraft('');
            setBulkCreateRule(false);
        } else {
            setBulkConfirm({ entries, newAccount, createRule: bulkCreateRule });
        }
    }

    const eligibleIds = transactions
        .filter((t) => singleNonBalancingPostingWithIndex(t) !== null)
        .map((t) => t.id);
    const allSelected =
        eligibleIds.length > 0 &&
        eligibleIds.every((id) => selectedIds.has(id));
    const someSelected = eligibleIds.some((id) => selectedIds.has(id));
    const bulkEntries = [...selectedIds].flatMap((id) => {
        const txn = transactions.find((t) => t.id === id);
        if (!txn) return [];
        const match = singleNonBalancingPostingWithIndex(txn);
        return match
            ? [
                  {
                      txnId: id,
                      postingIndex: match.postingIndex,
                      oldAccount: match.posting.account,
                      description: txn.descriptionRaw || txn.description,
                  },
              ]
            : [];
    });
    // Visible rows with an ML suggestion that can be accepted in one batch.
    const acceptAllEdits =
        onAcceptSuggestions === undefined
            ? []
            : buildAcceptAllEdits(
                  transactions,
                  glCategorySuggestions,
                  singleNonBalancingPostingWithIndex,
              );
    const colCount = 5 + (hasCheckbox ? 1 : 0);

    function openSimilarConfirmForTxn(
        txn: TransactionRow,
        targetAccount: string,
    ) {
        const key = similarityGroupKey(txn);
        if (key === null || onOpenSimilarRecategorize === undefined) return;
        const filteredSimilarIds = similarGroupIds.get(key) ?? [];
        if (filteredSimilarIds.length <= 1) return;
        const balancingAccount =
            txn.postings.find(
                (posting) =>
                    posting.account.startsWith('Assets:') ||
                    posting.account.startsWith('Liabilities:'),
            )?.account ?? '';
        onOpenSimilarRecategorize({
            newAccount: targetAccount,
            description: txn.description,
            balancingAccount,
        });
    }

    return (
        <>
            {selectedIds.size === 0 && acceptAllEdits.length > 0 && (
                <div className="bulk-action-bar">
                    <span className="count-label">
                        {acceptAllEdits.length} ML suggestion
                        {acceptAllEdits.length === 1 ? '' : 's'} available
                    </span>
                    <button
                        type="button"
                        className="ghost-button"
                        disabled={acceptSuggestionsBusy}
                        onClick={() => {
                            setAcceptAllConfirm(acceptAllEdits);
                        }}
                    >
                        {acceptSuggestionsBusy
                            ? 'Accepting…'
                            : `Accept ${acceptAllEdits.length} suggestion${
                                  acceptAllEdits.length === 1 ? '' : 's'
                              }`}
                    </button>
                </div>
            )}
            {hasCheckbox && selectedIds.size > 0 && (
                <div className="bulk-action-bar">
                    <span className="count-label">
                        {selectedIds.size} selected
                        {bulkEntries.length < selectedIds.size &&
                            ` (${bulkEntries.length} eligible)`}
                    </span>
                    <AccountInput
                        value={bulkDraft}
                        onChange={(v) => {
                            setBulkDraft(v);
                        }}
                        onKeyDown={(e) => {
                            if (
                                e.key === 'Enter' &&
                                bulkDraft.trim() &&
                                bulkEntries.length > 0
                            ) {
                                applyBulk(bulkEntries, bulkDraft.trim());
                            } else if (e.key === 'Escape') {
                                updateSelectedIds(() => new Set());
                                setBulkDraft('');
                            }
                        }}
                        accounts={accountNames}
                        oldAccount={bulkEntries.map((e) => e.oldAccount)}
                        placeholder="New account…"
                    />
                    <button
                        type="button"
                        className="ghost-button"
                        disabled={!bulkDraft.trim() || bulkEntries.length === 0}
                        onClick={() => {
                            applyBulk(bulkEntries, bulkDraft.trim());
                        }}
                    >
                        Set Category
                    </button>
                    <label
                        className="count-label"
                        title="Also create a standing rule for each selected payee, so future matches post here automatically."
                    >
                        <input
                            type="checkbox"
                            checked={bulkCreateRule}
                            onChange={(e) => {
                                setBulkCreateRule(e.target.checked);
                            }}
                        />{' '}
                        Create rule
                    </label>
                    <button
                        type="button"
                        className="ghost-button"
                        onClick={() => {
                            updateSelectedIds(() => new Set());
                            setBulkDraft('');
                            setBulkCreateRule(false);
                        }}
                    >
                        Clear
                    </button>
                </div>
            )}
            <div
                ref={tableWrapRef}
                className="table-wrap"
                onScroll={(e) => {
                    onScrollTopChange?.(e.currentTarget.scrollTop);
                }}
            >
                <table className="ledger-table">
                    <thead>
                        <tr>
                            {hasCheckbox && (
                                <th>
                                    <input
                                        type="checkbox"
                                        checked={allSelected}
                                        ref={(el) => {
                                            if (el)
                                                el.indeterminate =
                                                    someSelected &&
                                                    !allSelected;
                                        }}
                                        onChange={() => {
                                            updateSelectedIds(() =>
                                                allSelected
                                                    ? new Set()
                                                    : new Set(eligibleIds),
                                            );
                                        }}
                                    />
                                </th>
                            )}
                            <th>Date</th>
                            <th>Description</th>
                            <th>Postings</th>
                            <th>Amount</th>
                            <th>Attachments</th>
                        </tr>
                    </thead>
                    <tbody>
                        {transactions.length === 0 ? (
                            <tr>
                                <td colSpan={colCount} className="table-empty">
                                    No transactions found.
                                </td>
                            </tr>
                        ) : (
                            transactions.map((txn) => {
                                const isUncategorized = txn.postings.some(
                                    (p) =>
                                        p.account === UNCATEGORIZED_GL_ACCOUNT,
                                );
                                const glSuggestion =
                                    glCategorySuggestions[txn.id];
                                const transferMatch =
                                    glSuggestion?.transferMatch ?? null;
                                // Near-miss transfer candidates (2+ ambiguous
                                // matches; see GlCategoryResult.transferCandidates).
                                const transferCandidateCount =
                                    glSuggestion?.transferCandidates.length ??
                                    0;
                                const suggested =
                                    glSuggestion?.suggested ?? null;
                                const eligible =
                                    singleNonBalancingPosting(txn) !== null;
                                return (
                                    <tr
                                        key={txn.id}
                                        className={
                                            isUncategorized
                                                ? 'row-uncategorized'
                                                : undefined
                                        }
                                    >
                                        {hasCheckbox && (
                                            <td>
                                                {eligible && (
                                                    <input
                                                        type="checkbox"
                                                        checked={selectedIds.has(
                                                            txn.id,
                                                        )}
                                                        onChange={() => {
                                                            updateSelectedIds(
                                                                (prev) => {
                                                                    const next =
                                                                        new Set(
                                                                            prev,
                                                                        );
                                                                    if (
                                                                        next.has(
                                                                            txn.id,
                                                                        )
                                                                    )
                                                                        next.delete(
                                                                            txn.id,
                                                                        );
                                                                    else
                                                                        next.add(
                                                                            txn.id,
                                                                        );
                                                                    return next;
                                                                },
                                                            );
                                                        }}
                                                    />
                                                )}
                                            </td>
                                        )}
                                        <td
                                            className="mono"
                                            onContextMenu={(e) => {
                                                openContextMenu(e, [
                                                    {
                                                        label: `Filter: date:${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:${txn.date}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: date:${txn.date}..`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:${txn.date}..`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: date:..${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:..${txn.date}`,
                                                            ),
                                                    },
                                                ]);
                                            }}
                                        >
                                            {txn.date}
                                        </td>
                                        <td
                                            onContextMenu={(e) => {
                                                const key = similarKey(txn);
                                                const similarIds =
                                                    key !== null
                                                        ? (similarGroupIds.get(
                                                              key,
                                                          ) ?? [])
                                                        : [];
                                                const balancingAccount =
                                                    txn.postings.find(
                                                        (p) =>
                                                            p.account.startsWith(
                                                                'Assets:',
                                                            ) ||
                                                            p.account.startsWith(
                                                                'Liabilities:',
                                                            ),
                                                    )?.account ?? '';
                                                const items: ContextMenuItem[] =
                                                    [
                                                        {
                                                            label: `Filter: desc:${quoteHledgerValue(txn.description)}`,
                                                            action: () =>
                                                                onAddSearchTerm?.(
                                                                    `desc:${quoteHledgerValue(txn.description)}`,
                                                                ),
                                                        },
                                                    ];
                                                if (
                                                    hasCheckbox &&
                                                    similarIds.length > 1
                                                ) {
                                                    items.push({
                                                        label: `Check ${similarIds.length} uncategorized ${txn.description} transactions from ${balancingAccount}`,
                                                        action: () => {
                                                            updateSelectedIds(
                                                                () =>
                                                                    new Set(
                                                                        similarIds,
                                                                    ),
                                                            );
                                                        },
                                                    });
                                                }
                                                openContextMenu(e, [...items]);
                                            }}
                                        >
                                            <div>{txn.description}</div>
                                            {onOpenEvidence !== undefined &&
                                                parseGlSourceRefs(
                                                    txn.comment,
                                                ).map((ref) => (
                                                    <button
                                                        key={`${txn.id}:src:${ref.locator}:${ref.entryId}`}
                                                        type="button"
                                                        className="link-button source-chip"
                                                        title={`Open source evidence (${ref.locator})`}
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            onOpenEvidence(ref);
                                                        }}
                                                    >
                                                        source
                                                    </button>
                                                ))}
                                            {(() => {
                                                const badges =
                                                    bookkeepingBadges(txn);
                                                return badges.length ===
                                                    0 ? null : (
                                                    <div className="transaction-bookkeeping-badges">
                                                        {badges.map((badge) => (
                                                            <span
                                                                key={`${txn.id}:${badge}`}
                                                                className="status-chip"
                                                            >
                                                                {badge}
                                                            </span>
                                                        ))}
                                                    </div>
                                                );
                                            })()}
                                        </td>
                                        <td>
                                            {hasActions ? (
                                                <div className="postings-list">
                                                    {txn.postings.map(
                                                        (p, postingIndex) => {
                                                            const key = `${txn.id}:${postingIndex}`;
                                                            const isEditing =
                                                                editingKey ===
                                                                key;
                                                            const isUnknown =
                                                                p.account ===
                                                                UNCATEGORIZED_GL_ACCOUNT;
                                                            // Only counterpart legs are recategorizable;
                                                            // the backend enforces the same rule in
                                                            // apply_recategorizations (src-tauri/src/post.rs).
                                                            const isNonBalanceSheet =
                                                                !p.account.startsWith(
                                                                    'Assets:',
                                                                ) &&
                                                                !p.account.startsWith(
                                                                    'Liabilities:',
                                                                );
                                                            const postingMenuItems: ContextMenuItem[] =
                                                                [
                                                                    {
                                                                        label: `Filter: acct:${quoteHledgerValue(p.account)}`,
                                                                        action: () =>
                                                                            onAddSearchTerm?.(
                                                                                `acct:${quoteHledgerValue(p.account)}`,
                                                                            ),
                                                                    },
                                                                ];
                                                            if (
                                                                isNonBalanceSheet
                                                            ) {
                                                                postingMenuItems.push(
                                                                    {
                                                                        label: 'Set Category',
                                                                        action: () => {
                                                                            setCategoryDraft(
                                                                                isUnknown &&
                                                                                    suggested !==
                                                                                        null
                                                                                    ? suggested
                                                                                    : '',
                                                                            );
                                                                            setEditingKey(
                                                                                key,
                                                                            );
                                                                        },
                                                                    },
                                                                );
                                                                const keyForSimilarMenu =
                                                                    similarityGroupKey(
                                                                        txn,
                                                                    );
                                                                const similarIdsForMenu =
                                                                    keyForSimilarMenu !==
                                                                    null
                                                                        ? (similarGroupIds.get(
                                                                              keyForSimilarMenu,
                                                                          ) ??
                                                                          [])
                                                                        : [];
                                                                if (
                                                                    !isUnknown &&
                                                                    onOpenSimilarRecategorize !==
                                                                        undefined &&
                                                                    similarIdsForMenu.length >
                                                                        1
                                                                ) {
                                                                    postingMenuItems.push(
                                                                        {
                                                                            label: `Categorize ${similarIdsForMenu.length} similar transactions to ${p.account}`,
                                                                            action: () => {
                                                                                openSimilarConfirmForTxn(
                                                                                    txn,
                                                                                    p.account,
                                                                                );
                                                                            },
                                                                        },
                                                                    );
                                                                }
                                                            }
                                                            if (
                                                                isNonBalanceSheet &&
                                                                onOpenLinkTransfer !==
                                                                    undefined
                                                            ) {
                                                                postingMenuItems.push(
                                                                    {
                                                                        label: 'Link Transfer',
                                                                        action: () => {
                                                                            onOpenLinkTransfer(
                                                                                txn.id,
                                                                            );
                                                                        },
                                                                    },
                                                                );
                                                            }
                                                            // A generated txn with 2+ source tags is a merged
                                                            // transfer; offer server-side unmerge (see
                                                            // post::unpost_gl_transaction).
                                                            if (
                                                                onUnmergeTransfer !==
                                                                    undefined &&
                                                                txn.bookkeeping
                                                                    .generated &&
                                                                (
                                                                    txn.comment.match(
                                                                        /; source:/g,
                                                                    ) ?? []
                                                                ).length >= 2
                                                            ) {
                                                                postingMenuItems.push(
                                                                    {
                                                                        label: 'Unmerge transfer',
                                                                        action: () => {
                                                                            onUnmergeTransfer(
                                                                                txn.id,
                                                                            );
                                                                        },
                                                                    },
                                                                );
                                                            }
                                                            // Negative memory for a suggested (not yet merged)
                                                            // transfer pair; drops the ↔ chip.
                                                            if (
                                                                onNotATransfer !==
                                                                    undefined &&
                                                                transferMatch !==
                                                                    null
                                                            ) {
                                                                postingMenuItems.push(
                                                                    {
                                                                        label: 'Not a transfer',
                                                                        action: () => {
                                                                            onNotATransfer(
                                                                                txn.id,
                                                                                transferMatch.txnId,
                                                                            );
                                                                        },
                                                                    },
                                                                );
                                                            }
                                                            const hideAmounts =
                                                                hideObviousAmounts &&
                                                                hasObviousAmounts(
                                                                    txn,
                                                                );
                                                            const keyForSimilar =
                                                                similarKey(txn);
                                                            const filteredSimilarIds =
                                                                keyForSimilar !==
                                                                null
                                                                    ? (similarGroupIds.get(
                                                                          keyForSimilar,
                                                                      ) ?? [])
                                                                    : [];
                                                            const canShowSimilarPill =
                                                                onOpenSimilarRecategorize !==
                                                                    undefined &&
                                                                filteredSimilarIds.length >
                                                                    1;
                                                            return (
                                                                <div
                                                                    key={`${txn.id}:${postingIndex}`}
                                                                    className="postings-item"
                                                                >
                                                                    {isEditing ? (
                                                                        <>
                                                                            <AccountInput
                                                                                value={
                                                                                    categoryDraft
                                                                                }
                                                                                onChange={(
                                                                                    v,
                                                                                ) => {
                                                                                    setCategoryDraft(
                                                                                        v,
                                                                                    );
                                                                                }}
                                                                                onKeyDown={(
                                                                                    e,
                                                                                ) => {
                                                                                    if (
                                                                                        e.key ===
                                                                                            'Enter' &&
                                                                                        categoryDraft.trim()
                                                                                    ) {
                                                                                        onRecategorize?.(
                                                                                            txn.id,
                                                                                            postingIndex,
                                                                                            categoryDraft.trim(),
                                                                                            p.account,
                                                                                        );
                                                                                        setEditingKey(
                                                                                            null,
                                                                                        );
                                                                                    } else if (
                                                                                        e.key ===
                                                                                        'Escape'
                                                                                    ) {
                                                                                        setEditingKey(
                                                                                            null,
                                                                                        );
                                                                                    }
                                                                                }}
                                                                                accounts={
                                                                                    accountNames
                                                                                }
                                                                                oldAccount={
                                                                                    p.account
                                                                                }
                                                                                autoFocus
                                                                            />
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                disabled={
                                                                                    !categoryDraft.trim()
                                                                                }
                                                                                onClick={() => {
                                                                                    if (
                                                                                        categoryDraft.trim()
                                                                                    ) {
                                                                                        onRecategorize?.(
                                                                                            txn.id,
                                                                                            postingIndex,
                                                                                            categoryDraft.trim(),
                                                                                            p.account,
                                                                                        );
                                                                                        setEditingKey(
                                                                                            null,
                                                                                        );
                                                                                    }
                                                                                }}
                                                                            >
                                                                                Set
                                                                            </button>
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    setEditingKey(
                                                                                        null,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                Cancel
                                                                            </button>
                                                                            {canShowSimilarPill && (
                                                                                <button
                                                                                    type="button"
                                                                                    className="similar-count-pill"
                                                                                    disabled={
                                                                                        !categoryDraft.trim()
                                                                                    }
                                                                                    onClick={() => {
                                                                                        if (
                                                                                            categoryDraft.trim()
                                                                                        ) {
                                                                                            openSimilarConfirmForTxn(
                                                                                                txn,
                                                                                                categoryDraft.trim(),
                                                                                            );
                                                                                        }
                                                                                    }}
                                                                                >
                                                                                    ×
                                                                                    {
                                                                                        filteredSimilarIds.length
                                                                                    }{' '}
                                                                                    similar
                                                                                </button>
                                                                            )}
                                                                        </>
                                                                    ) : isUnknown ? (
                                                                        <button
                                                                            type="button"
                                                                            className="posting-account posting-account-unknown"
                                                                            title="Click to set category"
                                                                            onClick={() => {
                                                                                setCategoryDraft(
                                                                                    suggested !==
                                                                                        null
                                                                                        ? suggested
                                                                                        : '',
                                                                                );
                                                                                setEditingKey(
                                                                                    key,
                                                                                );
                                                                            }}
                                                                            onContextMenu={(
                                                                                e,
                                                                            ) => {
                                                                                openContextMenu(
                                                                                    e,
                                                                                    postingMenuItems,
                                                                                );
                                                                            }}
                                                                        >
                                                                            {
                                                                                p.account
                                                                            }
                                                                        </button>
                                                                    ) : (
                                                                        <span
                                                                            onContextMenu={(
                                                                                e,
                                                                            ) => {
                                                                                openContextMenu(
                                                                                    e,
                                                                                    postingMenuItems,
                                                                                );
                                                                            }}
                                                                        >
                                                                            {
                                                                                p.account
                                                                            }
                                                                        </span>
                                                                    )}
                                                                    {isNonBalanceSheet &&
                                                                        !isEditing &&
                                                                        transferMatch !==
                                                                            null &&
                                                                        onMergeTransfer !==
                                                                            undefined && (
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                disabled={
                                                                                    transferActionBusy
                                                                                }
                                                                                onClick={() => {
                                                                                    onMergeTransfer(
                                                                                        txn.id,
                                                                                        transferMatch.txnId,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                {mergeTransferChipLabel(
                                                                                    {
                                                                                        date: transferMatch.date,
                                                                                        description:
                                                                                            transferMatch.description,
                                                                                    },
                                                                                )}
                                                                            </button>
                                                                        )}
                                                                    {isNonBalanceSheet &&
                                                                        !isEditing &&
                                                                        transferMatch ===
                                                                            null &&
                                                                        transferCandidateCount >=
                                                                            2 &&
                                                                        onOpenLinkTransfer !==
                                                                            undefined && (
                                                                            // Ambiguous
                                                                            // near-miss:
                                                                            // open the
                                                                            // Link
                                                                            // Transfer
                                                                            // modal, no
                                                                            // one-click
                                                                            // merge.
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                title="Multiple possible transfer counterparts; open Link Transfer to pick one"
                                                                                onClick={() => {
                                                                                    onOpenLinkTransfer(
                                                                                        txn.id,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                ↔{' '}
                                                                                {
                                                                                    transferCandidateCount
                                                                                }{' '}
                                                                                possible
                                                                            </button>
                                                                        )}
                                                                    {isUnknown &&
                                                                        !isEditing &&
                                                                        suggested !==
                                                                            null &&
                                                                        transferMatch ===
                                                                            null &&
                                                                        onRecategorize !==
                                                                            undefined && (
                                                                            <div className="categorize-chip">
                                                                                <button
                                                                                    type="button"
                                                                                    className="ghost-button"
                                                                                    disabled={
                                                                                        recategorizeBusy
                                                                                    }
                                                                                    onClick={() => {
                                                                                        onRecategorize(
                                                                                            txn.id,
                                                                                            postingIndex,
                                                                                            suggested,
                                                                                            p.account,
                                                                                        );
                                                                                    }}
                                                                                >
                                                                                    {categorizeChipLabel(
                                                                                        suggested,
                                                                                    )}
                                                                                </button>
                                                                                {canShowSimilarPill && (
                                                                                    <button
                                                                                        type="button"
                                                                                        className="similar-count-pill"
                                                                                        onClick={() => {
                                                                                            openSimilarConfirmForTxn(
                                                                                                txn,
                                                                                                suggested,
                                                                                            );
                                                                                        }}
                                                                                    >
                                                                                        ×
                                                                                        {
                                                                                            filteredSimilarIds.length
                                                                                        }{' '}
                                                                                        similar
                                                                                    </button>
                                                                                )}
                                                                            </div>
                                                                        )}
                                                                    {!hideAmounts && (
                                                                        <span className="amount">
                                                                            {formatTotals(
                                                                                p.totals,
                                                                            )}
                                                                        </span>
                                                                    )}
                                                                </div>
                                                            );
                                                        },
                                                    )}
                                                </div>
                                            ) : (
                                                <PostingsList
                                                    postings={txn.postings}
                                                    hideAmounts={
                                                        hideObviousAmounts &&
                                                        hasObviousAmounts(txn)
                                                    }
                                                />
                                            )}
                                        </td>
                                        <td
                                            className="amount"
                                            onContextMenu={(e) => {
                                                const totals = txn.totals;
                                                if (
                                                    totals == null ||
                                                    totals.length === 0
                                                )
                                                    return;
                                                const t = totals[0];
                                                if (t == null) return;
                                                const total = formatScaled(
                                                    t.mantissa,
                                                    t.scale,
                                                );
                                                openContextMenu(e, [
                                                    {
                                                        label: `Filter: amt:${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:${total}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: amt:>=${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:>=${total}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: amt:<=${total}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `amt:<=${total}`,
                                                            ),
                                                    },
                                                ]);
                                            }}
                                        >
                                            {formatTotals(txn.totals)}
                                        </td>
                                        <td>
                                            {txn.evidence.length === 0 ? (
                                                <span className="text-muted">
                                                    -
                                                </span>
                                            ) : (
                                                (() => {
                                                    const imageRefs =
                                                        txn.evidence.filter(
                                                            isImageAttachmentRef,
                                                        );
                                                    const otherRefs =
                                                        txn.evidence.filter(
                                                            (r) =>
                                                                !isImageAttachmentRef(
                                                                    r,
                                                                ),
                                                        );
                                                    const evidenceExpanded =
                                                        expandedEvidenceIds.has(
                                                            txn.id,
                                                        );
                                                    return (
                                                        <div className="evidence-list">
                                                            {imageRefs.map(
                                                                (
                                                                    evidenceRef,
                                                                ) => (
                                                                    <button
                                                                        key={`${txn.id}-${evidenceRef}`}
                                                                        className="evidence-chip evidence-chip-image"
                                                                        type="button"
                                                                        onClick={() => {
                                                                            void lightbox.openImage(
                                                                                attachmentFilename(
                                                                                    evidenceRef,
                                                                                ),
                                                                            );
                                                                        }}
                                                                    >
                                                                        {attachmentFilename(
                                                                            evidenceRef,
                                                                        )}
                                                                    </button>
                                                                ),
                                                            )}
                                                            {otherRefs.length >
                                                                0 && (
                                                                <>
                                                                    <button
                                                                        className="evidence-chip evidence-chip-toggle"
                                                                        type="button"
                                                                        onClick={() => {
                                                                            setExpandedEvidenceIds(
                                                                                (
                                                                                    prev,
                                                                                ) => {
                                                                                    const next =
                                                                                        new Set(
                                                                                            prev,
                                                                                        );
                                                                                    if (
                                                                                        evidenceExpanded
                                                                                    ) {
                                                                                        next.delete(
                                                                                            txn.id,
                                                                                        );
                                                                                    } else {
                                                                                        next.add(
                                                                                            txn.id,
                                                                                        );
                                                                                    }
                                                                                    return next;
                                                                                },
                                                                            );
                                                                        }}
                                                                    >
                                                                        {evidenceExpanded
                                                                            ? '▾'
                                                                            : '▸'}{' '}
                                                                        {
                                                                            otherRefs.length
                                                                        }{' '}
                                                                        source
                                                                        {otherRefs.length !==
                                                                        1
                                                                            ? 's'
                                                                            : ''}
                                                                    </button>
                                                                    {evidenceExpanded &&
                                                                        otherRefs.map(
                                                                            (
                                                                                evidenceRef,
                                                                            ) => (
                                                                                <span
                                                                                    key={`${txn.id}-${evidenceRef}`}
                                                                                    className="evidence-chip"
                                                                                    title={
                                                                                        evidenceRef
                                                                                    }
                                                                                >
                                                                                    {
                                                                                        evidenceRef
                                                                                    }
                                                                                </span>
                                                                            ),
                                                                        )}
                                                                </>
                                                            )}
                                                        </div>
                                                    );
                                                })()
                                            )}
                                        </td>
                                    </tr>
                                );
                            })
                        )}
                    </tbody>
                </table>
            </div>
            <AttachmentLightbox
                filename={lightbox.filename}
                src={lightbox.src}
                loading={lightbox.loading}
                error={lightbox.error}
                onClose={lightbox.close}
            />
            {bulkConfirm !== null && (
                <BulkRecategorizeConfirmModal
                    newAccount={bulkConfirm.newAccount}
                    entries={bulkConfirm.entries}
                    onCancel={() => {
                        setBulkConfirm(null);
                    }}
                    onConfirm={() => {
                        onBulkRecategorize?.(
                            bulkConfirm.entries,
                            bulkConfirm.newAccount,
                            bulkConfirm.createRule,
                        );
                        updateSelectedIds(() => new Set());
                        setBulkDraft('');
                        setBulkCreateRule(false);
                        setBulkConfirm(null);
                    }}
                />
            )}
            {acceptAllConfirm !== null && (
                <div
                    className="modal-overlay"
                    onClick={() => {
                        setAcceptAllConfirm(null);
                    }}
                >
                    <div
                        className="modal-dialog"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>Accept ML suggestions</h3>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setAcceptAllConfirm(null);
                                }}
                            >
                                Close
                            </button>
                        </div>
                        <p>
                            Apply {acceptAllConfirm.length} suggested categor
                            {acceptAllConfirm.length === 1 ? 'y' : 'ies'}:
                        </p>
                        <ul>
                            {[
                                ...new Map(
                                    acceptAllConfirm.map((e) => [
                                        e.newAccount,
                                        0,
                                    ]),
                                ).keys(),
                            ].map((acct) => {
                                const count = acceptAllConfirm.filter(
                                    (e) => e.newAccount === acct,
                                ).length;
                                return (
                                    <li key={acct}>
                                        {count} → {acct}
                                    </li>
                                );
                            })}
                        </ul>
                        <div
                            style={{
                                display: 'flex',
                                gap: '0.5rem',
                                justifyContent: 'flex-end',
                            }}
                        >
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setAcceptAllConfirm(null);
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                className="ghost-button"
                                disabled={acceptSuggestionsBusy}
                                onClick={() => {
                                    onAcceptSuggestions?.(acceptAllConfirm);
                                    setAcceptAllConfirm(null);
                                }}
                            >
                                Accept {acceptAllConfirm.length}
                            </button>
                        </div>
                    </div>
                </div>
            )}
            {contextMenu && (
                <div
                    className="context-menu"
                    style={{ left: contextMenu.x, top: contextMenu.y }}
                    onMouseDown={(e) => {
                        e.stopPropagation();
                    }}
                >
                    {contextMenu.items.map((item, i) => (
                        <button
                            key={i}
                            type="button"
                            className="context-menu-item"
                            onClick={() => {
                                item.action();
                                setContextMenu(null);
                            }}
                        >
                            {item.label}
                        </button>
                    ))}
                </div>
            )}
        </>
    );
}
