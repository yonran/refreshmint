import {
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
} from 'react';
import {
    type AmountStyleHint,
    type AmountTotal,
    type AccountRow,
    type GlCategoryResult,
    type PostingRow,
    type TransactionRow,
    readAttachmentDataUrl,
} from '../tauri-commands.ts';
import {
    checkAccountTypeChange,
    getAccountSuggestions,
    quoteHledgerValue,
} from '../search-utils.ts';
import type { SimilarRecategorizeSeed } from '../types.ts';

function formatScaled(mantissa: string, scale: number): string {
    let negative = false;
    let digits = mantissa;
    if (digits.startsWith('-')) {
        negative = true;
        digits = digits.slice(1);
    }
    if (scale > 0) {
        const scaleInt = Math.max(scale, 0);
        if (digits.length <= scaleInt) {
            const needed = scaleInt + 1 - digits.length;
            digits = `${'0'.repeat(needed)}${digits}`;
        }
        const split = digits.length - scaleInt;
        const value = `${digits.slice(0, split)}.${digits.slice(split)}`;
        return negative ? `-${value}` : value;
    }
    return negative ? `-${digits}` : digits;
}

function normalizeStyle(style: AmountStyleHint | null) {
    if (style === null) {
        return { side: 'R' as const, spaced: true };
    }
    return style;
}

function formatTotal(total: AmountTotal): string {
    const value = formatScaled(total.mantissa, total.scale);
    const { side, spaced } = normalizeStyle(total.style);
    const separator = spaced ? ' ' : '';
    return side === 'L'
        ? `${total.commodity}${separator}${value}`
        : `${value}${separator}${total.commodity}`;
}

function formatTotals(totals: AmountTotal[] | null): string {
    if (!totals || totals.length === 0) {
        return 'N/A';
    }
    return totals.map(formatTotal).join(', ');
}

const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp'];

function isImageAttachmentRef(ref: string): boolean {
    if (!ref.endsWith('#attachment')) return false;
    const filename = ref.slice(0, -'#attachment'.length);
    return IMAGE_EXTENSIONS.some((ext) => filename.toLowerCase().endsWith(ext));
}

function attachmentFilename(ref: string): string {
    return ref.endsWith('#attachment')
        ? ref.slice(0, -'#attachment'.length)
        : ref;
}

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
    if (!posting || posting.account !== 'Expenses:Unknown') return null;
    return similarityGroupKey(txn);
}

export function AccountInput({
    value,
    onChange,
    onKeyDown,
    accounts,
    oldAccount,
    autoFocus,
    placeholder = 'Account name…',
}: {
    value: string;
    onChange: (value: string) => void;
    onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
    accounts: string[];
    /** Old account(s) being replaced — used to filter suggestions and show type-change warning. */
    oldAccount?: string | string[];
    autoFocus?: boolean;
    placeholder?: string;
}) {
    const [suggestions, setSuggestions] = useState<string[]>([]);
    const [activeIndex, setActiveIndex] = useState(-1);

    const warning =
        value.trim() !== ''
            ? checkAccountTypeChange(oldAccount ?? [], value.trim())
            : null;

    function computeSuggestions(draft: string) {
        setSuggestions(getAccountSuggestions(draft, accounts, oldAccount));
        setActiveIndex(-1);
    }

    function applyCompletion(sug: string) {
        onChange(sug);
        // Re-compute for the chosen value (e.g. "Expenses:" → show sub-accounts)
        const next = getAccountSuggestions(sug, accounts, oldAccount);
        setSuggestions(next);
        setActiveIndex(-1);
    }

    // ArrowDown/Up navigate, Enter/Tab (with active item) select, Escape dismisses.
    // Unhandled keys pass through to onKeyDown so the parent's Enter (commit)
    // and Escape (cancel editing) still fire when no suggestion is active.
    function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
        if (suggestions.length === 0) {
            onKeyDown?.(e);
            return;
        }
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setActiveIndex((i) => Math.min(i + 1, suggestions.length - 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setActiveIndex((i) => Math.max(i - 1, 0));
        } else if ((e.key === 'Enter' || e.key === 'Tab') && activeIndex >= 0) {
            e.preventDefault();
            const sug = suggestions[activeIndex];
            if (sug !== undefined) applyCompletion(sug);
        } else if (e.key === 'Escape') {
            // First Escape dismisses suggestions; second Escape reaches parent.
            setSuggestions([]);
            setActiveIndex(-1);
        } else {
            onKeyDown?.(e);
        }
    }

    return (
        <div className="account-input-wrap">
            <input
                type="text"
                value={value}
                placeholder={placeholder}
                autoFocus={autoFocus}
                onFocus={(e) => {
                    e.target.select();
                    computeSuggestions(e.target.value);
                }}
                onChange={(e) => {
                    onChange(e.target.value);
                    computeSuggestions(e.target.value);
                }}
                onKeyDown={handleKeyDown}
                onBlur={() => {
                    // Allow mousedown on a suggestion item to fire before blur
                    // closes the list.
                    setTimeout(() => {
                        setSuggestions([]);
                        setActiveIndex(-1);
                    }, 150);
                }}
            />
            {suggestions.length > 0 && (
                <div className="account-suggestions" role="listbox">
                    {suggestions.map((sug, i) => (
                        <div
                            key={sug}
                            className={`ac-item${i === activeIndex ? ' active' : ''}`}
                            role="option"
                            aria-selected={i === activeIndex}
                            onMouseDown={(e) => {
                                e.preventDefault(); // keep input focus
                                applyCompletion(sug);
                            }}
                        >
                            {sug}
                        </div>
                    ))}
                </div>
            )}
            {warning != null && warning !== '' && (
                <div className="account-warning">{warning}</div>
            )}
        </div>
    );
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

export function AccountsTable({
    accounts,
    onSelectAccount,
}: {
    accounts: AccountRow[];
    onSelectAccount: (name: string) => void;
}) {
    return (
        <table className="ledger-table">
            <thead>
                <tr>
                    <th>Account</th>
                    <th>Balance</th>
                    <th>Extraction</th>
                </tr>
            </thead>
            <tbody>
                {accounts.length === 0 ? (
                    <tr>
                        <td colSpan={3} className="table-empty">
                            No accounts found.
                        </td>
                    </tr>
                ) : (
                    accounts.map((account) => (
                        <tr key={account.name}>
                            <td>
                                <button
                                    className="link-button mono"
                                    onClick={() => {
                                        onSelectAccount(account.name);
                                    }}
                                >
                                    {account.name}
                                </button>
                            </td>
                            <td className="amount">
                                {formatTotals(account.totals)}
                            </td>
                            <td>
                                {account.unpostedCount > 0 ? (
                                    <span className="secret-chip warning">
                                        {account.unpostedCount} unposted
                                    </span>
                                ) : (
                                    <span className="status-dim">
                                        up to date
                                    </span>
                                )}
                            </td>
                        </tr>
                    ))
                )}
            </tbody>
        </table>
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
    onBulkRecategorize,
    onOpenSimilarRecategorize,
    hideObviousAmounts = true,
    onAddSearchTerm,
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
    ) => void;
    onMergeTransfer?: (txnId1: string, txnId2: string) => void;
    onOpenLinkTransfer?: (txnId: string) => void;
    onBulkRecategorize?: (
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
        }>,
        newAccount: string,
    ) => void;
    onOpenSimilarRecategorize?: (seed: SimilarRecategorizeSeed) => void;
    hideObviousAmounts?: boolean;
    onAddSearchTerm?: (term: string) => void;
}) {
    const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
    const [lightboxFilename, setLightboxFilename] = useState<string | null>(
        null,
    );
    const [lightboxLoading, setLightboxLoading] = useState(false);
    const [lightboxError, setLightboxError] = useState<string | null>(null);
    const [editingKey, setEditingKey] = useState<string | null>(null); // `${txnId}:${postingIndex}`
    const [categoryDraft, setCategoryDraft] = useState('');
    const [uncontrolledSelectedIds, setUncontrolledSelectedIds] = useState<
        ReadonlySet<string>
    >(new Set());
    const [bulkDraft, setBulkDraft] = useState('');
    const [bulkConfirm, setBulkConfirm] = useState<{
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
        }>;
        newAccount: string;
    } | null>(null);

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

    async function handleAttachmentClick(filename: string) {
        if (ledgerPath === null) return;
        setLightboxFilename(filename);
        setLightboxSrc(null);
        setLightboxError(null);
        setLightboxLoading(true);
        try {
            const src = await readAttachmentDataUrl(ledgerPath, filename);
            setLightboxSrc(src);
        } catch (e) {
            setLightboxError(String(e));
        } finally {
            setLightboxLoading(false);
        }
    }

    function closeLightbox() {
        setLightboxSrc(null);
        setLightboxFilename(null);
        setLightboxError(null);
        setLightboxLoading(false);
    }

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
        if (best !== undefined) setBulkDraft(best);
    }, [selectedIds]); // eslint-disable-line react-hooks/exhaustive-deps

    function applyBulk(
        entries: Array<{
            txnId: string;
            postingIndex: number;
            oldAccount: string;
        }>,
        newAccount: string,
    ) {
        if (onBulkRecategorize === undefined) return;
        const accounts = new Set(entries.map((e) => e.oldAccount));
        if (accounts.size <= 1) {
            onBulkRecategorize(entries, newAccount);
            updateSelectedIds(() => new Set());
            setBulkDraft('');
        } else {
            setBulkConfirm({ entries, newAccount });
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
                  },
              ]
            : [];
    });
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
                    <button
                        type="button"
                        className="ghost-button"
                        onClick={() => {
                            updateSelectedIds(() => new Set());
                            setBulkDraft('');
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
                                    (p) => p.account === 'Expenses:Unknown',
                                );
                                const glSuggestion =
                                    glCategorySuggestions[txn.id];
                                const transferMatch =
                                    glSuggestion?.transferMatch ?? null;
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
                                                        label: `Filter: date:>=${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:>=${txn.date}`,
                                                            ),
                                                    },
                                                    {
                                                        label: `Filter: date:<=${txn.date}`,
                                                        action: () =>
                                                            onAddSearchTerm?.(
                                                                `date:<=${txn.date}`,
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
                                            {txn.description}
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
                                                                'Expenses:Unknown';
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
                                                                    p.account !==
                                                                        'Expenses:Unknown' &&
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
                                                                isUncategorized &&
                                                                transferMatch !==
                                                                    null &&
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
                                                                    {isUnknown &&
                                                                        !isEditing &&
                                                                        transferMatch !==
                                                                            null &&
                                                                        onMergeTransfer !==
                                                                            undefined && (
                                                                            <button
                                                                                type="button"
                                                                                className="ghost-button"
                                                                                onClick={() => {
                                                                                    onMergeTransfer(
                                                                                        txn.id,
                                                                                        transferMatch.txnId,
                                                                                    );
                                                                                }}
                                                                            >
                                                                                ↔{' '}
                                                                                {
                                                                                    transferMatch.date
                                                                                }{' '}
                                                                                {
                                                                                    transferMatch.description
                                                                                }
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
                                                                                    onClick={() => {
                                                                                        onRecategorize(
                                                                                            txn.id,
                                                                                            postingIndex,
                                                                                            suggested,
                                                                                        );
                                                                                    }}
                                                                                >
                                                                                    {
                                                                                        suggested
                                                                                    }
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
                                                <div className="evidence-list">
                                                    {txn.evidence.map(
                                                        (evidenceRef) =>
                                                            isImageAttachmentRef(
                                                                evidenceRef,
                                                            ) ? (
                                                                <button
                                                                    key={`${txn.id}-${evidenceRef}`}
                                                                    className="evidence-chip evidence-chip-image"
                                                                    type="button"
                                                                    onClick={() => {
                                                                        void handleAttachmentClick(
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
                                                            ) : (
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
                                                </div>
                                            )}
                                        </td>
                                    </tr>
                                );
                            })
                        )}
                    </tbody>
                </table>
            </div>
            {(lightboxLoading ||
                lightboxSrc !== null ||
                lightboxError !== null) && (
                <div
                    className="modal-overlay"
                    onClick={closeLightbox}
                    role="dialog"
                    aria-modal="true"
                    aria-label={lightboxFilename ?? 'Attachment'}
                >
                    <div
                        className="modal-dialog attachment-lightbox"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>{lightboxFilename}</h3>
                            <button
                                type="button"
                                onClick={closeLightbox}
                                className="ghost-button"
                            >
                                Close
                            </button>
                        </div>
                        {lightboxLoading ? (
                            <p className="status">Loading…</p>
                        ) : lightboxError !== null ? (
                            <p className="status">{lightboxError}</p>
                        ) : lightboxSrc !== null ? (
                            <img
                                src={lightboxSrc}
                                alt={lightboxFilename ?? 'attachment'}
                                className="attachment-image"
                            />
                        ) : null}
                    </div>
                </div>
            )}
            {bulkConfirm !== null && (
                <div
                    className="modal-overlay"
                    onClick={() => {
                        setBulkConfirm(null);
                    }}
                >
                    <div
                        className="modal-dialog"
                        onClick={(e) => {
                            e.stopPropagation();
                        }}
                    >
                        <div className="modal-header">
                            <h3>Confirm bulk recategorize</h3>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    setBulkConfirm(null);
                                }}
                            >
                                Close
                            </button>
                        </div>
                        <p>
                            The selected rows have different current accounts.
                            All will be changed to{' '}
                            <strong>{bulkConfirm.newAccount}</strong>:
                        </p>
                        <ul>
                            {[
                                ...new Map(
                                    bulkConfirm.entries.map((e) => [
                                        e.oldAccount,
                                        0,
                                    ]),
                                ).entries(),
                            ].map(([acct]) => {
                                const count = bulkConfirm.entries.filter(
                                    (e) => e.oldAccount === acct,
                                ).length;
                                return (
                                    <li key={acct}>
                                        {count} × {acct} →{' '}
                                        {bulkConfirm.newAccount}
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
                                    setBulkConfirm(null);
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                className="ghost-button"
                                onClick={() => {
                                    onBulkRecategorize?.(
                                        bulkConfirm.entries,
                                        bulkConfirm.newAccount,
                                    );
                                    updateSelectedIds(() => new Set());
                                    setBulkDraft('');
                                    setBulkConfirm(null);
                                }}
                            >
                                Confirm
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
