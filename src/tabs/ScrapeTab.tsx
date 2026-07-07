import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
    type LedgerView,
    listScrapeExtensions,
    getScrapeLog,
    runScrapeForLogin,
} from '../tauri-commands.ts';
import { type ScrapeLogEntry } from '../scrapeLog.ts';
import {
    appendLogLine,
    type ScrapeOutputLine,
} from '../scrape-console-utils.ts';

interface ScrapeTabProps {
    ledger: LedgerView | null;
    loginNames: string[];
    isLoadingLoginConfigs: boolean;
    // Login selection is lifted to App and shared with the Settings tab so
    // both tabs act on the same login.
    selectedLoginName: string;
    onSelectedLoginNameChange: (name: string) => void;
    scrapeLogVersion: number;
    onScrapeComplete: (loginName: string) => Promise<void>;
    onScrapeAll: () => void;
    autoScrapeActive: string | null;
    headlessScrape: boolean;
}

export function ScrapeTab({
    ledger,
    loginNames,
    isLoadingLoginConfigs,
    selectedLoginName,
    onSelectedLoginNameChange,
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
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const [isRunningScrape, setIsRunningScrape] = useState(false);
    // Live driver output for the currently-selected login, streamed from the
    // backend via `refreshmint://scrape-output`.
    const [consoleLines, setConsoleLines] = useState<string[]>([]);
    const consoleRef = useRef<HTMLPreElement | null>(null);

    const ledgerPath = ledger?.path ?? null;

    // ─── Computed values ────────────────────────────────────────────────────────

    const activeScrapeLoginName = selectedLoginName.trim() || null;
    const hasActiveScrapeLogin = activeScrapeLoginName !== null;

    // ─── Effects ────────────────────────────────────────────────────────────────

    // Reset all own state when the ledger path changes.
    useEffect(() => {
        setScrapeStatus(null);
        setScrapeLogEntries([]);
        setConsoleLines([]);
    }, [ledgerPath]);

    // Stream live driver output for the selected login into the console pane.
    // Filtering by login keeps a running scrape's output out of other logins'
    // panes (an auto-scrape may run a different login concurrently).
    useEffect(() => {
        if (activeScrapeLoginName === null) return;
        const loginName = activeScrapeLoginName;
        const unlisten = listen<ScrapeOutputLine & { loginName: string }>(
            'refreshmint://scrape-output',
            (event) => {
                if (event.payload.loginName !== loginName) return;
                setConsoleLines((current) =>
                    appendLogLine(current, {
                        stream: event.payload.stream,
                        line: event.payload.line,
                    }),
                );
            },
        );
        return () => {
            void unlisten.then((fn) => {
                fn();
            });
        };
    }, [activeScrapeLoginName]);

    // Clear the console when switching logins so it only shows the selected
    // login's output.
    useEffect(() => {
        setConsoleLines([]);
    }, [activeScrapeLoginName]);

    // Auto-scroll the console pane to the newest line.
    useEffect(() => {
        const pane = consoleRef.current;
        if (pane) pane.scrollTop = pane.scrollHeight;
    }, [consoleLines]);

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

    // ─── Handlers ───────────────────────────────────────────────────────────────

    async function handleRunScrape() {
        if (!ledger) return;
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Login is required.');
            return;
        }

        setIsRunningScrape(true);
        setConsoleLines([]);
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
                            Select a login, then run the same scraper pipeline
                            as the CLI command.
                        </p>
                    </div>
                </div>
                <label className="field">
                    <span>Login</span>
                    <select
                        value={selectedLoginName}
                        onChange={(event) => {
                            onSelectedLoginNameChange(event.target.value);
                        }}
                    >
                        <option value="">
                            {isLoadingLoginConfigs
                                ? 'Loading logins...'
                                : 'Select login'}
                        </option>
                        {loginNames.map((loginName) => (
                            <option key={loginName} value={loginName}>
                                {loginName}
                            </option>
                        ))}
                    </select>
                </label>
                {hasActiveScrapeLogin ? (
                    <p className="hint mono">Login: {activeScrapeLoginName}</p>
                ) : (
                    <p className="hint">Select a login above to run scrape.</p>
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
                {(isRunningScrape || consoleLines.length > 0) && (
                    <div className="scrape-console">
                        <div className="scrape-console-header">
                            Live output
                            {activeScrapeLoginName !== null
                                ? ` — ${activeScrapeLoginName}`
                                : ''}
                        </div>
                        <pre className="scrape-console-pane" ref={consoleRef}>
                            {consoleLines.length > 0
                                ? consoleLines.join('\n')
                                : 'Waiting for driver output...'}
                        </pre>
                    </div>
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
