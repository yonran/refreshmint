import { useEffect, useMemo, useState } from 'react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import {
    type LedgerView,
    type LoginConfig,
    createLogin,
    deleteLogin,
    deleteLoginAccount,
    listScrapeExtensions,
    repairLoginAccountLabels,
    getScrapeLog,
    runScrapeForLogin,
    setLoginAccount,
    setLoginExtension,
} from '../tauri-commands.ts';
import { type LoginAccountMapping, normalizeLoginConfig } from '../types.ts';
import { type ScrapeLogEntry } from '../scrapeLog.ts';
import { AccountInput } from '../components/AccountInput.tsx';

interface ScrapeTabProps {
    ledger: LedgerView | null;
    // Shared login config data (loaded by App for the global conflicts panel)
    loginNames: string[];
    loginConfigsByName: Record<string, LoginConfig>;
    loginAccountMappings: Record<string, LoginAccountMapping[]>;
    isLoadingLoginConfigs: boolean;
    conflictingGlAccountSet: Set<string>;
    // Login selection state lifted to App (needed by handleIgnoreLoginAccountMapping /
    // handleLoadConflictMapping which run from the global conflicts panel in App)
    selectedLoginName: string;
    onSelectedLoginNameChange: (name: string) => void;
    loginManagementTab: 'select' | 'create';
    onLoginManagementTabChange: (tab: 'select' | 'create') => void;
    editingMappingLabel: string | null;
    onEditingMappingLabelChange: (label: string | null) => void;
    editingMappingGlAccountDraft: string;
    onEditingMappingGlAccountDraftChange: (account: string) => void;
    loginConfigStatus: string | null;
    onLoginConfigStatusChange: (status: string | null) => void;
    isSavingLoginConfig: boolean;
    onIsSavingLoginConfigChange: (saving: boolean) => void;
    // Callbacks
    onLoginConfigChanged: () => void;
    onIgnoreLoginAccountMapping: (
        loginName: string,
        label: string,
        glAccount: string,
    ) => Promise<void>;
    scrapeLogVersion: number;
    onScrapeComplete: (loginName: string) => Promise<void>;
    onScrapeAll: () => void;
    autoScrapeActive: string | null;
    headlessScrape: boolean;
}

export function ScrapeTab({
    ledger,
    loginNames,
    loginConfigsByName,
    isLoadingLoginConfigs,
    conflictingGlAccountSet,
    selectedLoginName,
    onSelectedLoginNameChange,
    loginManagementTab,
    onLoginManagementTabChange,
    editingMappingLabel,
    onEditingMappingLabelChange,
    editingMappingGlAccountDraft,
    onEditingMappingGlAccountDraftChange,
    loginConfigStatus,
    onLoginConfigStatusChange,
    isSavingLoginConfig,
    onIsSavingLoginConfigChange,
    onLoginConfigChanged,
    onIgnoreLoginAccountMapping,
    scrapeLogVersion,
    onScrapeComplete,
    onScrapeAll,
    autoScrapeActive,
    headlessScrape,
}: ScrapeTabProps) {
    const [scrapeExtensions, setScrapeExtensions] = useState<string[]>([]);
    const [scrapeStatus, setScrapeStatus] = useState<string | null>(null);
    const [scrapeLogEntries, setScrapeLogEntries] = useState<ScrapeLogEntry[]>(
        [],
    );
    const [selectedLoginExtensionDraft, setSelectedLoginExtensionDraft] =
        useState('');
    const [newLoginName, setNewLoginName] = useState('');
    const [newLoginExtension, setNewLoginExtension] = useState('');
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isRunningScrape, setIsRunningScrape] = useState(false);

    const ledgerPath = ledger?.path ?? null;

    // ─── Computed values ────────────────────────────────────────────────────────

    const scrapeAccountOptions = ledger
        ? ledger.accounts
              .map((account) => account.name.trim())
              .filter(
                  (name, index, names) =>
                      name.length > 0 && names.indexOf(name) === index,
              )
        : [];
    const activeScrapeLoginName = selectedLoginName.trim() || null;
    const hasActiveScrapeLogin = activeScrapeLoginName !== null;

    const selectedLoginConfig: LoginConfig | null =
        selectedLoginName.length === 0
            ? null
            : (loginConfigsByName[selectedLoginName] ?? null);
    const selectedLoginAccounts = useMemo(
        () =>
            selectedLoginConfig === null
                ? []
                : Object.entries(
                      normalizeLoginConfig(selectedLoginConfig).accounts,
                  ).sort(([a], [b]) => a.localeCompare(b)),
        [selectedLoginConfig],
    );
    const selectedLoginConflictCount = selectedLoginAccounts.reduce(
        (count, [, config]) => {
            const glAccount = config.glAccount?.trim() ?? '';
            return glAccount.length > 0 &&
                conflictingGlAccountSet.has(glAccount)
                ? count + 1
                : count;
        },
        0,
    );

    // ─── Effects ────────────────────────────────────────────────────────────────

    // Reset all own state when the ledger path changes.
    useEffect(() => {
        setScrapeStatus(null);
        setSelectedLoginExtensionDraft('');
        setNewLoginName('');
        setNewLoginExtension('');
        setScrapeLogEntries([]);
    }, [ledgerPath]);

    // Reload scrape log when selected login or scrapeLogVersion changes.
    useEffect(() => {
        if (activeScrapeLoginName === null || !ledger) {
            setScrapeLogEntries([]);
            return;
        }
        let cancelled = false;
        const loginName = activeScrapeLoginName;
        getScrapeLog(ledger.path, loginName)
            .then((entries) => {
                if (!cancelled) setScrapeLogEntries(entries);
            })
            .catch(() => {
                if (!cancelled) setScrapeLogEntries([]);
            });
        return () => {
            cancelled = true;
        };
    }, [activeScrapeLoginName, scrapeLogVersion, ledger]);

    // List scrape extensions when ledger changes.
    useEffect(() => {
        if (ledgerPath === null) {
            setScrapeExtensions([]);
            setIsLoadingScrapeExtensions(false);
            return;
        }

        let cancelled = false;
        setIsLoadingScrapeExtensions(true);
        setScrapeStatus(null);
        void listScrapeExtensions(ledgerPath)
            .then((extensions) => {
                if (cancelled) return;
                setScrapeExtensions(extensions);
            })
            .catch((error: unknown) => {
                if (!cancelled) {
                    setScrapeExtensions([]);
                    setScrapeStatus(
                        `Failed to load scrape extensions: ${String(error)}`,
                    );
                }
            })
            .finally(() => {
                if (!cancelled) {
                    setIsLoadingScrapeExtensions(false);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [ledgerPath]);

    // Auto-populate extension draft when selected login changes.
    useEffect(() => {
        const extension = selectedLoginConfig?.extension?.trim() ?? '';
        setSelectedLoginExtensionDraft(extension);
    }, [selectedLoginConfig]);

    // ─── Handlers ───────────────────────────────────────────────────────────────

    async function handleCreateLoginConfig() {
        if (!ledger) return;
        const loginName = newLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Login name is required.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await createLogin(ledger.path, loginName, newLoginExtension.trim());
            setNewLoginName('');
            setNewLoginExtension('');
            onSelectedLoginNameChange(loginName);
            onLoginManagementTabChange('select');
            onLoginConfigStatusChange(`Created login '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to create login: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleDeleteSelectedLoginConfig() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login to delete.');
            return;
        }
        const shouldDelete = await confirmDialog(
            `Delete login '${loginName}'? This fails if it still has documents or journal data.`,
            {
                title: 'Delete login?',
                kind: 'warning',
                okLabel: 'Delete',
                cancelLabel: 'Cancel',
            },
        );
        if (!shouldDelete) {
            onLoginConfigStatusChange('Delete login canceled.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await deleteLogin(ledger.path, loginName);
            onSelectedLoginNameChange('');
            onLoginConfigStatusChange(`Deleted login '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to delete login: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleSaveSelectedLoginExtension() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            await setLoginExtension(
                ledger.path,
                loginName,
                selectedLoginExtensionDraft.trim(),
            );
            onLoginConfigStatusChange(`Saved extension for '${loginName}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to save extension: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleSetLoginAccountMapping() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }
        const label = (editingMappingLabel ?? '').trim();
        if (label.length === 0) {
            onLoginConfigStatusChange('Label is required.');
            return;
        }
        const glAccount = editingMappingGlAccountDraft.trim();

        onIsSavingLoginConfigChange(true);
        try {
            await setLoginAccount(
                ledger.path,
                loginName,
                label,
                glAccount.length === 0 ? null : glAccount,
            );
            onLoginConfigStatusChange(
                glAccount.length === 0
                    ? `Set '${loginName}/${label}' as ignored (no GL account).`
                    : `Mapped '${loginName}/${label}' to '${glAccount}'.`,
            );
            onEditingMappingLabelChange(null);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to set mapping: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleRemoveLoginAccountMapping(label: string) {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }
        const shouldRemove = await confirmDialog(
            `Remove label '${label}' from login '${loginName}'?`,
            {
                title: 'Remove mapping?',
                kind: 'warning',
                okLabel: 'Remove',
                cancelLabel: 'Cancel',
            },
        );
        if (!shouldRemove) return;

        onIsSavingLoginConfigChange(true);
        try {
            await deleteLoginAccount(ledger.path, loginName, label);
            onLoginConfigStatusChange(`Removed '${loginName}/${label}'.`);
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to remove mapping: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleRepairSelectedLoginLabels() {
        if (!ledger) return;
        const loginName = selectedLoginName.trim();
        if (loginName.length === 0) {
            onLoginConfigStatusChange('Select a login first.');
            return;
        }

        onIsSavingLoginConfigChange(true);
        try {
            const outcome = await repairLoginAccountLabels(
                ledger.path,
                loginName,
            );
            const migratedCount = outcome.migrated.length;
            const skippedCount = outcome.skipped.length;
            const warningSummary =
                outcome.warnings.length === 0
                    ? ''
                    : ` Warnings: ${outcome.warnings.join(' ')}`;
            onLoginConfigStatusChange(
                `Repaired ${migratedCount} label${migratedCount === 1 ? '' : 's'} for '${loginName}'. Skipped ${skippedCount}.${warningSummary}`,
            );
            onLoginConfigChanged();
        } catch (error) {
            onLoginConfigStatusChange(
                `Failed to repair labels: ${String(error)}`,
            );
        } finally {
            onIsSavingLoginConfigChange(false);
        }
    }

    async function handleRunScrape() {
        if (!ledger) return;
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Login is required.');
            return;
        }

        setIsRunningScrape(true);
        setScrapeStatus(`Running scrape for ${loginName}...`);
        const timestamp = new Date().toISOString();
        try {
            await runScrapeForLogin(
                ledger.path,
                loginName,
                'manual',
                headlessScrape,
            );
            localStorage.setItem(`lastScrape:${loginName}`, timestamp);
            setScrapeStatus(`Scrape completed for ${loginName}.`);
            await onScrapeComplete(loginName);
        } catch (error) {
            setScrapeStatus(`Scrape failed: ${String(error)}`);
        } finally {
            setIsRunningScrape(false);
            getScrapeLog(ledger.path, loginName)
                .then((entries) => {
                    setScrapeLogEntries(entries);
                })
                .catch(() => {});
        }
    }

    // ─── JSX ────────────────────────────────────────────────────────────────────

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <div>
                        <h2>Run scrape</h2>
                        <p>
                            Select a login in the Login Management section
                            below, then run the same scraper pipeline as the CLI
                            command.
                        </p>
                    </div>
                </div>
                <section className="pipeline-panel">
                    <div className="txn-form-header">
                        <div>
                            <h3>Login mappings</h3>
                            <p>
                                Configure login names, extension defaults, and
                                label to GL account mappings.
                            </p>
                        </div>
                        <div className="header-actions">
                            <button
                                className="ghost-button"
                                type="button"
                                onClick={() => {
                                    onLoginConfigChanged();
                                }}
                                disabled={
                                    isLoadingLoginConfigs || isSavingLoginConfig
                                }
                            >
                                {isLoadingLoginConfigs
                                    ? 'Refreshing...'
                                    : 'Refresh logins'}
                            </button>
                        </div>
                    </div>
                    <fieldset>
                        <legend>
                            <div className="tabs pipeline-subtabs">
                                <button
                                    type="button"
                                    className={
                                        loginManagementTab === 'select'
                                            ? 'tab active'
                                            : 'tab'
                                    }
                                    onClick={() => {
                                        onLoginManagementTabChange('select');
                                    }}
                                    disabled={loginNames.length === 0}
                                >
                                    Existing login
                                </button>
                                <button
                                    type="button"
                                    className={
                                        loginManagementTab === 'create'
                                            ? 'tab active'
                                            : 'tab'
                                    }
                                    onClick={() => {
                                        onLoginManagementTabChange('create');
                                    }}
                                >
                                    Create new login
                                </button>
                            </div>
                        </legend>
                        {loginManagementTab === 'create' ? (
                            <div className="login-create-body">
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Create login name</span>
                                        <input
                                            type="text"
                                            value={newLoginName}
                                            placeholder="chase-personal"
                                            onChange={(event) => {
                                                setNewLoginName(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={isSavingLoginConfig}
                                        />
                                    </label>
                                    <label className="field">
                                        <span>Initial extension</span>
                                        <input
                                            type="text"
                                            value={newLoginExtension}
                                            placeholder="optional"
                                            onChange={(event) => {
                                                setNewLoginExtension(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={isSavingLoginConfig}
                                        />
                                    </label>
                                </div>
                                <div className="pipeline-actions">
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleCreateLoginConfig();
                                        }}
                                        disabled={isSavingLoginConfig}
                                    >
                                        {isSavingLoginConfig
                                            ? 'Saving...'
                                            : 'Create login'}
                                    </button>
                                </div>
                            </div>
                        ) : (
                            <>
                                <div className="txn-grid">
                                    <label className="field">
                                        <span>Existing login</span>
                                        <select
                                            value={selectedLoginName}
                                            onChange={(event) => {
                                                onSelectedLoginNameChange(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={isSavingLoginConfig}
                                        >
                                            <option value="">
                                                {isLoadingLoginConfigs
                                                    ? 'Loading logins...'
                                                    : 'Select login'}
                                            </option>
                                            {loginNames.map((loginName) => (
                                                <option
                                                    key={loginName}
                                                    value={loginName}
                                                >
                                                    {loginName}
                                                </option>
                                            ))}
                                        </select>
                                    </label>
                                    <label className="field">
                                        <span>Login extension</span>
                                        <select
                                            value={selectedLoginExtensionDraft}
                                            onChange={(event) => {
                                                setSelectedLoginExtensionDraft(
                                                    event.target.value,
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 ||
                                                isSavingLoginConfig ||
                                                isLoadingScrapeExtensions
                                            }
                                        >
                                            <option value="">
                                                {isLoadingScrapeExtensions
                                                    ? 'Loading extensions...'
                                                    : 'Select extension'}
                                            </option>
                                            {scrapeExtensions.map((name) => (
                                                <option key={name} value={name}>
                                                    {name}
                                                </option>
                                            ))}
                                            {selectedLoginExtensionDraft.length >
                                                0 &&
                                            !scrapeExtensions.includes(
                                                selectedLoginExtensionDraft,
                                            ) ? (
                                                <option
                                                    value={
                                                        selectedLoginExtensionDraft
                                                    }
                                                >
                                                    {selectedLoginExtensionDraft.includes(
                                                        '/',
                                                    ) ||
                                                    selectedLoginExtensionDraft.includes(
                                                        '\\',
                                                    )
                                                        ? `(unpacked) ${selectedLoginExtensionDraft.split('/').pop() ?? selectedLoginExtensionDraft}`
                                                        : selectedLoginExtensionDraft}
                                                </option>
                                            ) : null}
                                        </select>
                                    </label>
                                </div>
                                <div className="pipeline-actions">
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleSaveSelectedLoginExtension();
                                        }}
                                        disabled={
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
                                        }
                                    >
                                        {isSavingLoginConfig
                                            ? 'Saving...'
                                            : 'Save login extension'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleRepairSelectedLoginLabels();
                                        }}
                                        disabled={
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
                                        }
                                    >
                                        {isSavingLoginConfig
                                            ? 'Saving...'
                                            : 'Repair labels'}
                                    </button>
                                    <button
                                        type="button"
                                        className="ghost-button"
                                        onClick={() => {
                                            void handleDeleteSelectedLoginConfig();
                                        }}
                                        disabled={
                                            selectedLoginName.length === 0 ||
                                            isSavingLoginConfig
                                        }
                                    >
                                        {isSavingLoginConfig
                                            ? 'Saving...'
                                            : 'Delete login'}
                                    </button>
                                </div>
                                {selectedLoginConfig === null ? (
                                    <p className="hint">
                                        Select a login to manage its account
                                        labels.
                                    </p>
                                ) : selectedLoginAccounts.length === 0 ? (
                                    <p className="hint">
                                        No labels configured yet. Labels are
                                        discovered automatically on the first
                                        scrape run. Use + to add a mapping
                                        manually.
                                    </p>
                                ) : (
                                    <div className="table-wrap">
                                        <table className="ledger-table">
                                            <thead>
                                                <tr>
                                                    <th>Label</th>
                                                    <th>GL Account</th>
                                                    <th>Actions</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {selectedLoginAccounts.map(
                                                    ([label, config]) => {
                                                        const glAccount =
                                                            config.glAccount?.trim() ??
                                                            '';
                                                        const hasConflict =
                                                            glAccount.length >
                                                                0 &&
                                                            conflictingGlAccountSet.has(
                                                                glAccount,
                                                            );
                                                        return (
                                                            <tr key={label}>
                                                                <td>
                                                                    <span className="mono">
                                                                        {label}
                                                                    </span>
                                                                </td>
                                                                <td>
                                                                    {config.glAccount ??
                                                                        '(ignored)'}
                                                                    {hasConflict ? (
                                                                        <span className="secret-chip">
                                                                            conflict
                                                                        </span>
                                                                    ) : null}
                                                                </td>
                                                                <td>
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        onClick={() => {
                                                                            onEditingMappingLabelChange(
                                                                                label,
                                                                            );
                                                                            onEditingMappingGlAccountDraftChange(
                                                                                config.glAccount ??
                                                                                    '',
                                                                            );
                                                                            onLoginConfigStatusChange(
                                                                                null,
                                                                            );
                                                                        }}
                                                                        disabled={
                                                                            isSavingLoginConfig
                                                                        }
                                                                    >
                                                                        Edit
                                                                    </button>
                                                                    {glAccount.length >
                                                                    0 ? (
                                                                        <button
                                                                            type="button"
                                                                            className="ghost-button"
                                                                            onClick={() => {
                                                                                void onIgnoreLoginAccountMapping(
                                                                                    selectedLoginName,
                                                                                    label,
                                                                                    glAccount,
                                                                                );
                                                                            }}
                                                                            disabled={
                                                                                isSavingLoginConfig
                                                                            }
                                                                        >
                                                                            Ignore
                                                                        </button>
                                                                    ) : null}
                                                                    <button
                                                                        type="button"
                                                                        className="ghost-button"
                                                                        onClick={() => {
                                                                            void handleRemoveLoginAccountMapping(
                                                                                label,
                                                                            );
                                                                        }}
                                                                        disabled={
                                                                            isSavingLoginConfig
                                                                        }
                                                                    >
                                                                        Remove
                                                                    </button>
                                                                </td>
                                                            </tr>
                                                        );
                                                    },
                                                )}
                                            </tbody>
                                        </table>
                                    </div>
                                )}
                                {selectedLoginConflictCount > 0 ? (
                                    <p className="status">
                                        {selectedLoginConflictCount} mapping
                                        conflict
                                        {selectedLoginConflictCount === 1
                                            ? ''
                                            : 's'}{' '}
                                        for this login. Resolve by editing or
                                        ignoring a conflicting mapping.
                                    </p>
                                ) : null}
                                {selectedLoginConfig !== null &&
                                editingMappingLabel === null ? (
                                    <div className="pipeline-actions">
                                        <button
                                            type="button"
                                            className="ghost-button"
                                            onClick={() => {
                                                onEditingMappingLabelChange('');
                                                onEditingMappingGlAccountDraftChange(
                                                    '',
                                                );
                                                onLoginConfigStatusChange(null);
                                            }}
                                            disabled={
                                                selectedLoginName.length ===
                                                    0 || isSavingLoginConfig
                                            }
                                        >
                                            + Add mapping
                                        </button>
                                    </div>
                                ) : null}
                                {editingMappingLabel !== null ? (
                                    <>
                                        <div className="txn-grid">
                                            <label className="field">
                                                <span>Label</span>
                                                <input
                                                    type="text"
                                                    value={editingMappingLabel}
                                                    placeholder="checking"
                                                    readOnly={
                                                        editingMappingLabel.length >
                                                        0
                                                    }
                                                    onChange={(event) => {
                                                        onEditingMappingLabelChange(
                                                            event.target.value,
                                                        );
                                                        onLoginConfigStatusChange(
                                                            null,
                                                        );
                                                    }}
                                                    disabled={
                                                        isSavingLoginConfig
                                                    }
                                                />
                                            </label>
                                            <label className="field">
                                                <span>GL account</span>
                                                <AccountInput
                                                    value={
                                                        editingMappingGlAccountDraft
                                                    }
                                                    onChange={(next) => {
                                                        onEditingMappingGlAccountDraftChange(
                                                            next,
                                                        );
                                                        onLoginConfigStatusChange(
                                                            null,
                                                        );
                                                    }}
                                                    accounts={
                                                        scrapeAccountOptions
                                                    }
                                                    placeholder="Assets:Bank:Checking (blank = ignored)"
                                                />
                                            </label>
                                        </div>
                                        <div className="pipeline-actions">
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    void handleSetLoginAccountMapping();
                                                }}
                                                disabled={
                                                    selectedLoginName.length ===
                                                        0 || isSavingLoginConfig
                                                }
                                            >
                                                {isSavingLoginConfig
                                                    ? 'Saving...'
                                                    : 'Save mapping'}
                                            </button>
                                            <button
                                                type="button"
                                                className="ghost-button"
                                                onClick={() => {
                                                    onEditingMappingLabelChange(
                                                        null,
                                                    );
                                                    onLoginConfigStatusChange(
                                                        null,
                                                    );
                                                }}
                                                disabled={isSavingLoginConfig}
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                    </>
                                ) : null}
                            </>
                        )}
                        {loginConfigStatus === null ? null : (
                            <p className="status">{loginConfigStatus}</p>
                        )}
                    </fieldset>
                </section>
                {hasActiveScrapeLogin ? (
                    <p className="hint mono">Login: {activeScrapeLoginName}</p>
                ) : (
                    <p className="hint">
                        Select a login in the Login Management section above to
                        run scrape or start a debug session.
                    </p>
                )}
                <div className="txn-actions">
                    <button
                        type="button"
                        className="primary-button"
                        onClick={() => {
                            void handleRunScrape();
                        }}
                        disabled={
                            isRunningScrape ||
                            !hasActiveScrapeLogin ||
                            isLoadingScrapeExtensions
                        }
                    >
                        {isRunningScrape ? 'Running scrape...' : 'Run scrape'}
                    </button>
                    <button
                        type="button"
                        className="secondary-button"
                        onClick={onScrapeAll}
                        disabled={
                            autoScrapeActive !== null ||
                            isRunningScrape ||
                            loginNames.length === 0
                        }
                    >
                        Scrape and Extract All
                    </button>
                </div>
                {scrapeStatus === null ? null : (
                    <p
                        className={
                            scrapeStatus.toLowerCase().includes('failed') ||
                            scrapeStatus.toLowerCase().includes('error')
                                ? 'status status-error'
                                : 'status'
                        }
                    >
                        {scrapeStatus}
                    </p>
                )}
                {scrapeLogEntries.length > 0 && (
                    <details className="scrape-log-disclosure">
                        <summary className="disclosure-summary">
                            Scrape log ({scrapeLogEntries.length})
                        </summary>
                        <table className="scrape-log-table">
                            <thead>
                                <tr>
                                    <th>Time</th>
                                    <th>Source</th>
                                    <th>Status</th>
                                    <th>Error</th>
                                </tr>
                            </thead>
                            <tbody>
                                {scrapeLogEntries.map((entry, i) => (
                                    <tr
                                        key={i}
                                        className={
                                            entry.success ? '' : 'status-error'
                                        }
                                    >
                                        <td>
                                            {new Date(
                                                entry.timestamp,
                                            ).toLocaleString()}
                                        </td>
                                        <td>{entry.source}</td>
                                        <td>
                                            {entry.success ? 'OK' : 'Failed'}
                                        </td>
                                        <td>{entry.error ?? ''}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </details>
                )}
                {scrapeExtensions.length === 0 && !isLoadingScrapeExtensions ? (
                    <p className="hint">
                        No runnable extensions found in extensions/*/driver.mjs.
                    </p>
                ) : null}
            </section>
        </div>
    );
}
