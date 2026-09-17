import { useCallback, useEffect, useState } from 'react';
import {
    buildCategoryRuleInput,
    categoryRuleFormValuesFromResolution,
    categoryRuleSummary,
    EMPTY_CATEGORY_RULE_FORM,
    type CategoryRuleFormValues,
} from '../automation-utils.ts';
import { AccountInput } from '../components/AccountInput.tsx';
import {
    createResolution,
    disableResolution,
    enableResolution,
    listResolutions,
    type Resolution,
} from '../tauri-commands.ts';

interface Props {
    ledger: string;
    accountNames: string[];
}

function scopeLabel(rule: Resolution): string {
    const scope = rule.subjectRefs[0];
    if (scope == null) return 'All accounts';
    return `${scope.loginName ?? '?'}/${scope.label ?? '?'}`;
}

export function RulesTab({ ledger, accountNames }: Props) {
    const [rules, setRules] = useState<Resolution[]>([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [busyId, setBusyId] = useState<string | null>(null);

    // 'new' opens a blank creation form; a rule id opens that rule for editing.
    const [editingId, setEditingId] = useState<string | null>(null);
    const [form, setForm] = useState<CategoryRuleFormValues>(
        EMPTY_CATEGORY_RULE_FORM,
    );
    const [formError, setFormError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);

    const refresh = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const all = await listResolutions(ledger);
            setRules(all.filter((r) => r.kind === 'category-rule'));
        } catch (err) {
            setError(`Failed to load rules: ${String(err)}`);
        } finally {
            setLoading(false);
        }
    }, [ledger]);

    useEffect(() => {
        void refresh();
    }, [refresh]);

    function openNewRuleForm() {
        setEditingId('new');
        setForm(EMPTY_CATEGORY_RULE_FORM);
        setFormError(null);
    }

    function openEditForm(rule: Resolution) {
        setEditingId(rule.id);
        setForm(categoryRuleFormValuesFromResolution(rule));
        setFormError(null);
    }

    function closeForm() {
        setEditingId(null);
        setFormError(null);
    }

    async function handleToggleStatus(rule: Resolution) {
        setBusyId(rule.id);
        try {
            if (rule.status === 'active') {
                await disableResolution(ledger, rule.id);
            } else {
                await enableResolution(ledger, rule.id);
            }
            await refresh();
        } catch (err) {
            setError(`Failed to update rule: ${String(err)}`);
        } finally {
            setBusyId(null);
        }
    }

    async function handleSave() {
        const input = buildCategoryRuleInput(form);
        if (input === null) {
            setFormError(
                'Enter a target account and at least one of merchant match or description pattern.',
            );
            return;
        }
        setSaving(true);
        setFormError(null);
        try {
            // CategoryRule ids are content-addressed by (kind, subjectRefs,
            // parts, predicate) — there is no in-place update, so editing
            // disables the old resolution (if still active) and creates a new
            // one from the edited fields. See automation-utils.ts.
            if (editingId !== null && editingId !== 'new') {
                const existing = rules.find((r) => r.id === editingId);
                if (existing?.status === 'active') {
                    await disableResolution(ledger, existing.id);
                }
            }
            await createResolution(ledger, input);
            closeForm();
            await refresh();
        } catch (err) {
            setFormError(`Failed to save rule: ${String(err)}`);
        } finally {
            setSaving(false);
        }
    }

    return (
        <div className="rules-panel">
            <div className="txn-form-header header-actions">
                <div>
                    <h2>Category rules</h2>
                    <p>
                        Standing rules that auto-categorize matching
                        transactions instead of leaving them in
                        Expenses:Unknown. Editing a rule disables the old one
                        and creates a new one from the edited fields.
                    </p>
                </div>
                <button
                    type="button"
                    className="primary-button"
                    onClick={openNewRuleForm}
                    disabled={editingId !== null}
                >
                    New rule
                </button>
            </div>

            {error !== null && <p className="status status-error">{error}</p>}

            {editingId !== null && (
                <div className="rules-edit-form">
                    <h3>{editingId === 'new' ? 'New rule' : 'Edit rule'}</h3>
                    <div className="txn-grid">
                        <label className="field">
                            Merchant match (normalized payee)
                            <input
                                type="text"
                                value={form.normalizedPayee}
                                placeholder="e.g. SAFEWAY"
                                onChange={(e) => {
                                    setForm((f) => ({
                                        ...f,
                                        normalizedPayee: e.target.value,
                                    }));
                                }}
                            />
                        </label>
                        <label className="field">
                            Or description pattern (regex)
                            <input
                                type="text"
                                value={form.descriptionRegex}
                                placeholder="e.g. STARBUCKS.*"
                                onChange={(e) => {
                                    setForm((f) => ({
                                        ...f,
                                        descriptionRegex: e.target.value,
                                    }));
                                }}
                            />
                        </label>
                        <label className="field">
                            Target account
                            <AccountInput
                                value={form.account}
                                onChange={(v) => {
                                    setForm((f) => ({ ...f, account: v }));
                                }}
                                accounts={accountNames}
                                placeholder="Expenses:Groceries"
                            />
                        </label>
                        <label className="field">
                            Amount min (optional)
                            <input
                                type="text"
                                value={form.amountMin}
                                onChange={(e) => {
                                    setForm((f) => ({
                                        ...f,
                                        amountMin: e.target.value,
                                    }));
                                }}
                            />
                        </label>
                        <label className="field">
                            Amount max (optional)
                            <input
                                type="text"
                                value={form.amountMax}
                                onChange={(e) => {
                                    setForm((f) => ({
                                        ...f,
                                        amountMax: e.target.value,
                                    }));
                                }}
                            />
                        </label>
                        <label className="field">
                            Notes
                            <input
                                type="text"
                                value={form.notes}
                                onChange={(e) => {
                                    setForm((f) => ({
                                        ...f,
                                        notes: e.target.value,
                                    }));
                                }}
                            />
                        </label>
                    </div>
                    <p className="hint">
                        Scope:{' '}
                        {form.scope
                            ? `${form.scope.loginName ?? '?'}/${form.scope.label ?? '?'} only (not editable here)`
                            : 'All accounts (global)'}
                    </p>
                    {formError !== null && (
                        <p className="status status-error">{formError}</p>
                    )}
                    <div className="txn-actions">
                        <button
                            type="button"
                            className="primary-button"
                            onClick={() => {
                                void handleSave();
                            }}
                            disabled={saving}
                        >
                            {saving ? 'Saving…' : 'Save rule'}
                        </button>
                        <button
                            type="button"
                            className="ghost-button"
                            onClick={closeForm}
                            disabled={saving}
                        >
                            Cancel
                        </button>
                    </div>
                </div>
            )}

            <div className="table-wrap">
                {loading ? (
                    <p className="status">Loading rules…</p>
                ) : rules.length === 0 ? (
                    <p className="table-empty">
                        No category rules yet. Create one above, or check
                        "Create rule" when bulk-recategorizing in the
                        Transactions tab.
                    </p>
                ) : (
                    <table className="ledger-table">
                        <thead>
                            <tr>
                                <th>Match → Account</th>
                                <th>Scope</th>
                                <th>Status</th>
                                <th>Updated</th>
                                <th />
                            </tr>
                        </thead>
                        <tbody>
                            {rules.map((rule) => (
                                <tr key={rule.id}>
                                    <td>{categoryRuleSummary(rule)}</td>
                                    <td>{scopeLabel(rule)}</td>
                                    <td>{rule.status}</td>
                                    <td className="mono">
                                        {rule.updatedAt.slice(0, 10)}
                                    </td>
                                    <td>
                                        <div className="pipeline-row-actions">
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    openEditForm(rule);
                                                }}
                                                disabled={
                                                    editingId !== null ||
                                                    busyId === rule.id
                                                }
                                            >
                                                Edit
                                            </button>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    void handleToggleStatus(
                                                        rule,
                                                    );
                                                }}
                                                disabled={
                                                    editingId !== null ||
                                                    busyId === rule.id
                                                }
                                            >
                                                {rule.status === 'active'
                                                    ? 'Disable'
                                                    : 'Enable'}
                                            </button>
                                        </div>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </div>
        </div>
    );
}
