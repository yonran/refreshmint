import { useCallback, useEffect, useMemo, useState } from 'react';
import {
    createBookkeepingLink,
    createReconciliationSession,
    deleteBookkeepingLink,
    finalizeReconciliationSession,
    listBookkeepingLinks,
    listPeriodCloses,
    listReconciliationSessions,
    reopenPeriodClose,
    reopenReconciliationSession,
    type AccountRow,
    type LinkKind,
    type LinkRecord,
    type NewLinkRecordInput,
    type NewReconciliationSessionInput,
    type PeriodClose,
    type PeriodCloseStatus,
    queryReconciliationCandidates,
    type ReconciliationSession,
    type TransactionRow,
    type TypedRef,
    type TypedRefKind,
    upsertPeriodClose,
} from '../tauri-commands.ts';

interface Props {
    ledger: string;
    accounts: AccountRow[];
}

type TypedRefDraft = {
    kind: TypedRefKind;
    id: string;
    locator: string;
    entryId: string;
    loginName: string;
    label: string;
    filename: string;
};

const TYPED_REF_KINDS: TypedRefKind[] = ['gl-txn', 'login-entry', 'document'];
const PERIOD_CLOSE_STATUSES: PeriodCloseStatus[] = [
    'draft',
    'soft-closed',
    'reopened',
];
const LINK_KINDS: LinkKind[] = [
    'settlement-link',
    'evidence-link',
    'source-link',
];

function createEmptyTypedRefDraft(
    kind: TypedRefKind = 'gl-txn',
): TypedRefDraft {
    return {
        kind,
        id: '',
        locator: '',
        entryId: '',
        loginName: '',
        label: '',
        filename: '',
    };
}

function parseTypedRefKind(value: string): TypedRefKind {
    return TYPED_REF_KINDS.find((kind) => kind === value) ?? 'gl-txn';
}

function parsePeriodCloseStatus(value: string): PeriodCloseStatus {
    return PERIOD_CLOSE_STATUSES.find((status) => status === value) ?? 'draft';
}

function parseLinkKind(value: string): LinkKind {
    return LINK_KINDS.find((kind) => kind === value) ?? 'settlement-link';
}

function localIsoDate(): string {
    const now = new Date();
    const offset = now.getTimezoneOffset() * 60_000;
    return new Date(now.getTime() - offset).toISOString().slice(0, 10);
}

function currentPeriodId(): string {
    return localIsoDate().slice(0, 7);
}

function parseListInput(value: string): string[] {
    return value
        .split(/[\n,]/)
        .map((part) => part.trim())
        .filter((part) => part.length > 0);
}

function trimToNull(value: string): string | null {
    const trimmed = value.trim();
    return trimmed.length === 0 ? null : trimmed;
}

function buildTypedRef(draft: TypedRefDraft): TypedRef {
    const base = { kind: draft.kind } as TypedRef;
    if (draft.kind === 'gl-txn') {
        return { ...base, id: trimToNull(draft.id) };
    }
    if (draft.kind === 'login-entry') {
        return {
            ...base,
            locator: trimToNull(draft.locator),
            entryId: trimToNull(draft.entryId),
        };
    }
    return {
        ...base,
        loginName: trimToNull(draft.loginName),
        label: trimToNull(draft.label),
        filename: trimToNull(draft.filename),
    };
}

function typedRefSummary(value: TypedRef): string {
    if (value.kind === 'gl-txn') {
        return `gl-txn:${value.id ?? 'missing-id'}`;
    }
    if (value.kind === 'login-entry') {
        return `login-entry:${value.locator ?? 'missing-locator'}:${value.entryId ?? 'missing-entry'}`;
    }
    return `document:${value.loginName ?? 'missing-login'}/${value.label ?? 'missing-label'}/${value.filename ?? 'missing-file'}`;
}

function TypedRefFields({
    label,
    draft,
    onChange,
}: {
    label: string;
    draft: TypedRefDraft;
    onChange: (next: TypedRefDraft) => void;
}) {
    return (
        <div className="field">
            <span>{label}</span>
            <select
                value={draft.kind}
                onChange={(event) => {
                    onChange(
                        createEmptyTypedRefDraft(
                            parseTypedRefKind(event.target.value),
                        ),
                    );
                }}
            >
                <option value="gl-txn">GL transaction</option>
                <option value="login-entry">Login entry</option>
                <option value="document">Document</option>
            </select>
            {draft.kind === 'gl-txn' ? (
                <input
                    type="text"
                    value={draft.id}
                    placeholder="gl-txn-id"
                    onChange={(event) => {
                        onChange({ ...draft, id: event.target.value });
                    }}
                />
            ) : draft.kind === 'login-entry' ? (
                <>
                    <input
                        type="text"
                        value={draft.locator}
                        placeholder="logins/bank/accounts/checking"
                        onChange={(event) => {
                            onChange({ ...draft, locator: event.target.value });
                        }}
                    />
                    <input
                        type="text"
                        value={draft.entryId}
                        placeholder="entry id"
                        onChange={(event) => {
                            onChange({ ...draft, entryId: event.target.value });
                        }}
                    />
                </>
            ) : (
                <>
                    <input
                        type="text"
                        value={draft.loginName}
                        placeholder="login name"
                        onChange={(event) => {
                            onChange({
                                ...draft,
                                loginName: event.target.value,
                            });
                        }}
                    />
                    <input
                        type="text"
                        value={draft.label}
                        placeholder="account label"
                        onChange={(event) => {
                            onChange({ ...draft, label: event.target.value });
                        }}
                    />
                    <input
                        type="text"
                        value={draft.filename}
                        placeholder="filename.pdf"
                        onChange={(event) => {
                            onChange({
                                ...draft,
                                filename: event.target.value,
                            });
                        }}
                    />
                </>
            )}
        </div>
    );
}

export function BookkeepingTab({ ledger, accounts }: Props) {
    const [sessions, setSessions] = useState<ReconciliationSession[]>([]);
    const [links, setLinks] = useState<LinkRecord[]>([]);
    const [periodCloses, setPeriodCloses] = useState<PeriodClose[]>([]);
    const [reconciliationCandidates, setReconciliationCandidates] = useState<
        TransactionRow[]
    >([]);
    const [loading, setLoading] = useState(false);
    const [status, setStatus] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);

    const [glAccount, setGlAccount] = useState('');
    const [statementStartDate, setStatementStartDate] = useState('');
    const [statementEndDate, setStatementEndDate] = useState(localIsoDate());
    const [statementStartingBalance, setStatementStartingBalance] =
        useState('');
    const [statementEndingBalance, setStatementEndingBalance] = useState('');
    const [statementCurrency, setStatementCurrency] = useState('');
    const [selectedTxnIds, setSelectedTxnIds] = useState<string[]>([]);
    const [sessionNotes, setSessionNotes] = useState('');

    const [periodId, setPeriodId] = useState(currentPeriodId());
    const [periodStatus, setPeriodStatus] =
        useState<PeriodCloseStatus>('soft-closed');
    const [closedBy, setClosedBy] = useState('owner');
    const [periodNotes, setPeriodNotes] = useState('');
    const [closeSessionIds, setCloseSessionIds] = useState('');
    const [adjustmentTxnIds, setAdjustmentTxnIds] = useState('');

    const [linkKind, setLinkKind] = useState<LinkKind>('settlement-link');
    const [leftRef, setLeftRef] = useState<TypedRefDraft>(
        createEmptyTypedRefDraft('gl-txn'),
    );
    const [rightRef, setRightRef] = useState<TypedRefDraft>(
        createEmptyTypedRefDraft('document'),
    );
    const [linkAmount, setLinkAmount] = useState('');
    const [linkNotes, setLinkNotes] = useState('');

    const accountNames = useMemo(
        () =>
            accounts
                .map((account) => account.name)
                .sort((a, b) => a.localeCompare(b)),
        [accounts],
    );
    const selectableCandidateIds = useMemo(
        () =>
            reconciliationCandidates
                .filter(
                    (txn) => txn.bookkeeping.reconciledSessionIds.length === 0,
                )
                .map((txn) => txn.id),
        [reconciliationCandidates],
    );
    const selectedCandidateCount = useMemo(
        () =>
            selectableCandidateIds.filter((txnId) =>
                selectedTxnIds.includes(txnId),
            ).length,
        [selectableCandidateIds, selectedTxnIds],
    );
    const allCandidatesSelected =
        selectableCandidateIds.length > 0 &&
        selectedCandidateCount === selectableCandidateIds.length;

    const reloadCandidates = useCallback(async () => {
        if (glAccount.trim().length === 0) {
            setReconciliationCandidates([]);
            return;
        }
        const nextCandidates = await queryReconciliationCandidates(
            ledger,
            glAccount,
            trimToNull(statementStartDate),
            statementEndDate,
        );
        setReconciliationCandidates(nextCandidates);
    }, [glAccount, ledger, statementEndDate, statementStartDate]);

    const reload = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const [nextSessions, nextLinks, nextPeriodCloses] =
                await Promise.all([
                    listReconciliationSessions(ledger),
                    listBookkeepingLinks(ledger),
                    listPeriodCloses(ledger),
                ]);
            setSessions(nextSessions);
            setLinks(nextLinks);
            setPeriodCloses(nextPeriodCloses);
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setLoading(false);
        }
    }, [ledger]);

    useEffect(() => {
        void reload();
    }, [reload]);

    useEffect(() => {
        void reloadCandidates().catch((err: unknown) => {
            setError(err instanceof Error ? err.message : String(err));
        });
    }, [reloadCandidates]);

    useEffect(() => {
        const visibleIds = new Set(selectableCandidateIds);
        setSelectedTxnIds((current) =>
            current.filter((txnId) => visibleIds.has(txnId)),
        );
    }, [selectableCandidateIds]);

    async function handleCreateSession() {
        setError(null);
        setStatus('Creating reconciliation session…');
        try {
            const input: NewReconciliationSessionInput = {
                glAccount,
                statementStartDate: trimToNull(statementStartDate),
                statementEndDate,
                statementStartingBalance: trimToNull(statementStartingBalance),
                statementEndingBalance,
                currency: trimToNull(statementCurrency),
                reconciledTxnIds: selectedTxnIds,
                notes: trimToNull(sessionNotes),
            };
            await createReconciliationSession(ledger, input);
            setStatus('Reconciliation session created.');
            setStatementStartingBalance('');
            setStatementEndingBalance('');
            setSelectedTxnIds([]);
            setSessionNotes('');
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    function toggleTxnSelection(txnId: string) {
        setSelectedTxnIds((current) =>
            current.includes(txnId)
                ? current.filter((id) => id !== txnId)
                : [...current, txnId],
        );
    }

    async function handleFinalizeSession(id: string) {
        setError(null);
        setStatus(`Finalizing reconciliation session ${id}…`);
        try {
            await finalizeReconciliationSession(ledger, id);
            setStatus(`Reconciliation session ${id} finalized.`);
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    async function handleReopenSession(id: string) {
        setError(null);
        setStatus(`Reopening reconciliation session ${id}…`);
        try {
            await reopenReconciliationSession(ledger, id);
            setStatus(`Reconciliation session ${id} reopened.`);
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    async function handleUpsertPeriodClose() {
        setError(null);
        setStatus(`Saving close state for ${periodId}…`);
        try {
            await upsertPeriodClose(ledger, {
                periodId,
                status: periodStatus,
                closedBy: trimToNull(closedBy),
                notes: trimToNull(periodNotes),
                reconciliationSessionIds: parseListInput(closeSessionIds),
                adjustmentTxnIds: parseListInput(adjustmentTxnIds),
            });
            setStatus(`Saved close state for ${periodId}.`);
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    async function handleReopenPeriodClose(periodId: string) {
        setError(null);
        setStatus(`Reopening ${periodId}…`);
        try {
            await reopenPeriodClose(ledger, periodId);
            setStatus(`Reopened ${periodId}.`);
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    async function handleCreateLink() {
        setError(null);
        setStatus('Creating bookkeeping link…');
        try {
            const input: NewLinkRecordInput = {
                kind: linkKind,
                leftRef: buildTypedRef(leftRef),
                rightRef: buildTypedRef(rightRef),
                amount: trimToNull(linkAmount),
                notes: trimToNull(linkNotes),
            };
            await createBookkeepingLink(ledger, input);
            setStatus('Bookkeeping link created.');
            setLeftRef(createEmptyTypedRefDraft('gl-txn'));
            setRightRef(createEmptyTypedRefDraft('document'));
            setLinkAmount('');
            setLinkNotes('');
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    async function handleDeleteLink(id: string) {
        setError(null);
        setStatus(`Deleting link ${id}…`);
        try {
            await deleteBookkeepingLink(ledger, id);
            setStatus(`Deleted link ${id}.`);
            await reload();
            await reloadCandidates();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setStatus(null);
        }
    }

    return (
        <div className="transactions-panel">
            <section className="pipeline-panel">
                <div className="txn-form-header">
                    <div>
                        <h2>Bookkeeping</h2>
                        <p>
                            Reconciliation sessions, settlement links, and
                            soft-close records live here.
                        </p>
                    </div>
                    <div className="header-actions">
                        <button
                            type="button"
                            className="ghost-button"
                            disabled={loading}
                            onClick={() => {
                                void reload();
                            }}
                        >
                            {loading ? 'Refreshing…' : 'Refresh'}
                        </button>
                    </div>
                </div>
                {status !== null ? (
                    <div className="status">{status}</div>
                ) : null}
                {error !== null ? (
                    <div className="status status-error">{error}</div>
                ) : null}
            </section>

            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h3>Reconciliation Sessions</h3>
                        <p>
                            `Cleared` comes from hledger status. `Reconciled`
                            means the GL transaction ids are captured in one
                            finalized statement session. This section is still a
                            manual session editor, not the guided reconciliation
                            workflow yet.
                        </p>
                    </div>
                </div>
                <div className="txn-grid">
                    <label className="field">
                        <span>GL account</span>
                        <input
                            type="text"
                            value={glAccount}
                            list="bookkeeping-gl-accounts"
                            placeholder="Assets:Checking"
                            onChange={(event) => {
                                setGlAccount(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Statement start date</span>
                        <input
                            type="date"
                            value={statementStartDate}
                            onChange={(event) => {
                                setStatementStartDate(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Statement end date</span>
                        <input
                            type="date"
                            value={statementEndDate}
                            onChange={(event) => {
                                setStatementEndDate(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Starting balance</span>
                        <input
                            type="text"
                            value={statementStartingBalance}
                            placeholder="1000.00 USD"
                            onChange={(event) => {
                                setStatementStartingBalance(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Ending balance</span>
                        <input
                            type="text"
                            value={statementEndingBalance}
                            placeholder="950.25 USD"
                            onChange={(event) => {
                                setStatementEndingBalance(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Currency</span>
                        <input
                            type="text"
                            value={statementCurrency}
                            placeholder="USD"
                            onChange={(event) => {
                                setStatementCurrency(event.target.value);
                            }}
                        />
                    </label>
                </div>
                <label className="field">
                    <span>Candidate summary</span>
                    <div className="status-dim">
                        {glAccount.trim().length === 0
                            ? 'Choose a GL account to load candidate transactions.'
                            : `${selectedCandidateCount} of ${selectableCandidateIds.length} open candidate transaction(s) selected`}
                    </div>
                </label>
                <label className="field">
                    <span>Notes</span>
                    <textarea
                        value={sessionNotes}
                        placeholder="March checking statement"
                        onChange={(event) => {
                            setSessionNotes(event.target.value);
                        }}
                    />
                </label>
                <div className="pipeline-actions">
                    <button
                        type="button"
                        className="ghost-button"
                        disabled={selectableCandidateIds.length === 0}
                        onClick={() => {
                            setSelectedTxnIds(
                                allCandidatesSelected
                                    ? []
                                    : selectableCandidateIds,
                            );
                        }}
                    >
                        {allCandidatesSelected
                            ? 'Clear Candidate Selection'
                            : 'Select All Candidates'}
                    </button>
                    <button
                        type="button"
                        className="primary-button"
                        disabled={selectedTxnIds.length === 0}
                        onClick={() => {
                            void handleCreateSession();
                        }}
                    >
                        Create Session
                    </button>
                </div>
                <div className="table-wrap">
                    <table className="ledger-table">
                        <thead>
                            <tr>
                                <th>
                                    <input
                                        type="checkbox"
                                        checked={allCandidatesSelected}
                                        disabled={
                                            selectableCandidateIds.length === 0
                                        }
                                        onChange={() => {
                                            setSelectedTxnIds(
                                                allCandidatesSelected
                                                    ? []
                                                    : selectableCandidateIds,
                                            );
                                        }}
                                    />
                                </th>
                                <th>Date</th>
                                <th>Description</th>
                                <th>Amount</th>
                                <th>Status</th>
                            </tr>
                        </thead>
                        <tbody>
                            {reconciliationCandidates.length === 0 ? (
                                <tr>
                                    <td colSpan={5} className="status-dim">
                                        No candidate transactions for this
                                        account/date range.
                                    </td>
                                </tr>
                            ) : (
                                reconciliationCandidates.map((txn) => {
                                    const isSelectable =
                                        txn.bookkeeping.reconciledSessionIds
                                            .length === 0;
                                    const accountAmounts = txn.postings
                                        .filter(
                                            (posting) =>
                                                posting.account === glAccount,
                                        )
                                        .map((posting) => posting.amount ?? '—')
                                        .join(', ');
                                    return (
                                        <tr key={txn.id}>
                                            <td>
                                                <input
                                                    type="checkbox"
                                                    checked={selectedTxnIds.includes(
                                                        txn.id,
                                                    )}
                                                    disabled={!isSelectable}
                                                    onChange={() => {
                                                        toggleTxnSelection(
                                                            txn.id,
                                                        );
                                                    }}
                                                />
                                            </td>
                                            <td className="mono">{txn.date}</td>
                                            <td>
                                                <div>{txn.description}</div>
                                                <div className="status-dim mono">
                                                    {txn.id}
                                                </div>
                                            </td>
                                            <td className="mono">
                                                {accountAmounts}
                                            </td>
                                            <td>
                                                {txn.bookkeeping
                                                    .reconciledSessionIds
                                                    .length > 0 ? (
                                                    <span className="status-chip status-chip-warning">
                                                        reconciled elsewhere
                                                    </span>
                                                ) : (
                                                    <span className="status-chip">
                                                        open
                                                    </span>
                                                )}
                                            </td>
                                        </tr>
                                    );
                                })
                            )}
                        </tbody>
                    </table>
                </div>
                <div className="table-wrap">
                    <table className="ledger-table">
                        <thead>
                            <tr>
                                <th>Account</th>
                                <th>Statement end</th>
                                <th>Status</th>
                                <th>Txn ids</th>
                                <th>Actions</th>
                            </tr>
                        </thead>
                        <tbody>
                            {sessions.length === 0 ? (
                                <tr>
                                    <td colSpan={5} className="status-dim">
                                        No statement sessions yet.
                                    </td>
                                </tr>
                            ) : (
                                sessions.map((session) => (
                                    <tr key={session.id}>
                                        <td>
                                            <div>{session.glAccount}</div>
                                            <div className="status-dim">
                                                {session.id}
                                            </div>
                                        </td>
                                        <td>{session.statementEndDate}</td>
                                        <td>{session.status}</td>
                                        <td>
                                            {session.reconciledTxnIds.length}
                                        </td>
                                        <td>
                                            <div className="pipeline-row-actions">
                                                {session.status !==
                                                'finalized' ? (
                                                    <button
                                                        type="button"
                                                        className="ghost-button"
                                                        onClick={() => {
                                                            void handleFinalizeSession(
                                                                session.id,
                                                            );
                                                        }}
                                                    >
                                                        Finalize
                                                    </button>
                                                ) : null}
                                                {session.status !==
                                                'reopened' ? (
                                                    <button
                                                        type="button"
                                                        className="ghost-button"
                                                        onClick={() => {
                                                            void handleReopenSession(
                                                                session.id,
                                                            );
                                                        }}
                                                    >
                                                        Reopen
                                                    </button>
                                                ) : null}
                                            </div>
                                        </td>
                                    </tr>
                                ))
                            )}
                        </tbody>
                    </table>
                </div>
            </section>

            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h3>Soft Close</h3>
                        <p>
                            Track reviewed periods separately from hledger
                            cleared state and finalized reconciliation-session
                            membership.
                        </p>
                    </div>
                </div>
                <div className="txn-grid">
                    <label className="field">
                        <span>Period id</span>
                        <input
                            type="text"
                            value={periodId}
                            placeholder="2026-03"
                            onChange={(event) => {
                                setPeriodId(event.target.value);
                            }}
                        />
                    </label>
                    <label className="field">
                        <span>Status</span>
                        <select
                            value={periodStatus}
                            onChange={(event) => {
                                setPeriodStatus(
                                    parsePeriodCloseStatus(event.target.value),
                                );
                            }}
                        >
                            <option value="draft">draft</option>
                            <option value="soft-closed">soft-closed</option>
                            <option value="reopened">reopened</option>
                        </select>
                    </label>
                    <label className="field">
                        <span>Closed by</span>
                        <input
                            type="text"
                            value={closedBy}
                            placeholder="owner"
                            onChange={(event) => {
                                setClosedBy(event.target.value);
                            }}
                        />
                    </label>
                </div>
                <label className="field">
                    <span>Reconciliation session ids</span>
                    <textarea
                        value={closeSessionIds}
                        placeholder="session-id-1, session-id-2"
                        onChange={(event) => {
                            setCloseSessionIds(event.target.value);
                        }}
                    />
                </label>
                <label className="field">
                    <span>Adjustment GL transaction ids</span>
                    <textarea
                        value={adjustmentTxnIds}
                        placeholder="gl-adjust-1"
                        onChange={(event) => {
                            setAdjustmentTxnIds(event.target.value);
                        }}
                    />
                </label>
                <label className="field">
                    <span>Notes</span>
                    <textarea
                        value={periodNotes}
                        placeholder="March books reviewed after statement close."
                        onChange={(event) => {
                            setPeriodNotes(event.target.value);
                        }}
                    />
                </label>
                <div className="pipeline-actions">
                    <button
                        type="button"
                        className="primary-button"
                        onClick={() => {
                            void handleUpsertPeriodClose();
                        }}
                    >
                        Save Close State
                    </button>
                </div>
                <div className="table-wrap">
                    <table className="ledger-table">
                        <thead>
                            <tr>
                                <th>Period</th>
                                <th>Status</th>
                                <th>Sessions</th>
                                <th>Adjustments</th>
                                <th>Actions</th>
                            </tr>
                        </thead>
                        <tbody>
                            {periodCloses.length === 0 ? (
                                <tr>
                                    <td colSpan={5} className="status-dim">
                                        No period closes yet.
                                    </td>
                                </tr>
                            ) : (
                                periodCloses.map((close) => (
                                    <tr key={close.periodId}>
                                        <td>{close.periodId}</td>
                                        <td>{close.status}</td>
                                        <td>
                                            {
                                                close.reconciliationSessionIds
                                                    .length
                                            }
                                        </td>
                                        <td>{close.adjustmentTxnIds.length}</td>
                                        <td>
                                            {close.status !== 'reopened' ? (
                                                <button
                                                    type="button"
                                                    className="ghost-button"
                                                    onClick={() => {
                                                        void handleReopenPeriodClose(
                                                            close.periodId,
                                                        );
                                                    }}
                                                >
                                                    Reopen
                                                </button>
                                            ) : null}
                                        </td>
                                    </tr>
                                ))
                            )}
                        </tbody>
                    </table>
                </div>
            </section>

            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h3>Links and Settlements</h3>
                        <p>
                            `settlement-link` is for clearing open balances.
                            Other link kinds stay descriptive only.
                        </p>
                    </div>
                </div>
                <div className="txn-grid">
                    <label className="field">
                        <span>Link kind</span>
                        <select
                            value={linkKind}
                            onChange={(event) => {
                                setLinkKind(parseLinkKind(event.target.value));
                            }}
                        >
                            <option value="settlement-link">
                                settlement-link
                            </option>
                            <option value="evidence-link">evidence-link</option>
                            <option value="source-link">source-link</option>
                        </select>
                    </label>
                    <label className="field">
                        <span>Amount</span>
                        <input
                            type="text"
                            value={linkAmount}
                            placeholder="25.00 USD"
                            onChange={(event) => {
                                setLinkAmount(event.target.value);
                            }}
                        />
                    </label>
                </div>
                <div className="txn-grid">
                    <TypedRefFields
                        label="Left ref"
                        draft={leftRef}
                        onChange={setLeftRef}
                    />
                    <TypedRefFields
                        label="Right ref"
                        draft={rightRef}
                        onChange={setRightRef}
                    />
                </div>
                <label className="field">
                    <span>Notes</span>
                    <textarea
                        value={linkNotes}
                        placeholder="Imported cash settles the March accrual."
                        onChange={(event) => {
                            setLinkNotes(event.target.value);
                        }}
                    />
                </label>
                <div className="pipeline-actions">
                    <button
                        type="button"
                        className="primary-button"
                        onClick={() => {
                            void handleCreateLink();
                        }}
                    >
                        Create Link
                    </button>
                </div>
                <div className="table-wrap">
                    <table className="ledger-table">
                        <thead>
                            <tr>
                                <th>Kind</th>
                                <th>Left</th>
                                <th>Right</th>
                                <th>Amount</th>
                                <th>Actions</th>
                            </tr>
                        </thead>
                        <tbody>
                            {links.length === 0 ? (
                                <tr>
                                    <td colSpan={5} className="status-dim">
                                        No bookkeeping links yet.
                                    </td>
                                </tr>
                            ) : (
                                links.map((link) => (
                                    <tr key={link.id}>
                                        <td>
                                            <div>{link.kind}</div>
                                            <div className="status-dim">
                                                {link.id}
                                            </div>
                                        </td>
                                        <td>{typedRefSummary(link.leftRef)}</td>
                                        <td>
                                            {typedRefSummary(link.rightRef)}
                                        </td>
                                        <td>{link.amount ?? '—'}</td>
                                        <td>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    void handleDeleteLink(
                                                        link.id,
                                                    );
                                                }}
                                            >
                                                Delete
                                            </button>
                                        </td>
                                    </tr>
                                ))
                            )}
                        </tbody>
                    </table>
                </div>
            </section>

            <datalist id="bookkeeping-gl-accounts">
                {accountNames.map((accountName) => (
                    <option key={accountName} value={accountName} />
                ))}
            </datalist>
        </div>
    );
}
