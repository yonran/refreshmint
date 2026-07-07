import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
    type LedgerView,
    type ScrapeLogEntry,
    type LastScrapeSummary,
    listScrapeExtensions,
    getScrapeLog,
    getLastScrapeSummaries,
    runScrapeForLogin,
    cancelScrape,
    listScrapeFailureArtifacts,
    readScrapeFailureArtifact,
    getLockStatusSnapshot,
    startLockMetadataWatch,
    stopLockMetadataWatch,
    type LockStatusSnapshot,
} from '../tauri-commands.ts';
import {
    appendLogLine,
    partitionArtifacts,
    type ScrapeOutputLine,
} from '../scrape-console-utils.ts';
import type { ScrapeTabSession } from '../types.ts';

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
    promptTimeoutSecs: number;
    // Running/status/console state is lifted to App so it survives the ScrapeTab
    // unmount that a tab switch causes mid-scrape (mirrors ReportsTab).
    session: ScrapeTabSession;
    onSessionChange: (
        updater: (current: ScrapeTabSession) => ScrapeTabSession,
    ) => void;
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
    promptTimeoutSecs,
    session,
    onSessionChange,
}: ScrapeTabProps) {
    const [scrapeExtensions, setScrapeExtensions] = useState<string[]>([]);
    // Running/status/console live in local state mirrored back into the App-held
    // session so a mid-scrape tab switch (which unmounts this tab) keeps the
    // Running/Cancel affordance, the console, and the completion status.
    const [scrapeStatus, setScrapeStatus] = useState<string | null>(
        session.scrapeStatus,
    );
    // The login whose scrape this tab started and is still running, or null.
    const [runningLoginName, setRunningLoginName] = useState<string | null>(
        session.runningLoginName,
    );
    // Live driver output for the currently-selected login, streamed from the
    // backend via `refreshmint://scrape-output`.
    const [consoleLines, setConsoleLines] = useState<string[]>(
        session.consoleLines,
    );
    const [scrapeLogEntries, setScrapeLogEntries] = useState<ScrapeLogEntry[]>(
        [],
    );
    const [isLoadingScrapeExtensions, setIsLoadingScrapeExtensions] =
        useState(false);
    const isRunningScrape = runningLoginName !== null;
    const consoleRef = useRef<HTMLPreElement | null>(null);

    // Live snapshot of the App-tracked session, flushed on unmount so the running
    // state survives a tab switch. Assigned during render (like ReportsTab) so it
    // always reflects the latest local state even under StrictMode.
    const sessionRef = useRef<ScrapeTabSession>(session);
    sessionRef.current = { runningLoginName, scrapeStatus, consoleLines };

    // Commit a partial session update to BOTH the ref and the App immediately.
    // Used at the async run boundary (start/finish) so the transition reaches the
    // App even if this tab has already unmounted (a setState there would no-op,
    // so the completion status would otherwise be lost). Eager assignment before
    // the onSessionChange call keeps it StrictMode-safe (see ReportsTab).
    const commitSession = (partial: Partial<ScrapeTabSession>) => {
        sessionRef.current = { ...sessionRef.current, ...partial };
        onSessionChange(() => sessionRef.current);
    };

    // Adopt the incoming session when it changes (e.g. ledger reset, or a run
    // that completed while this tab was unmounted).
    useEffect(() => {
        setRunningLoginName(session.runningLoginName);
        setScrapeStatus(session.scrapeStatus);
        setConsoleLines(session.consoleLines);
    }, [session]);

    // Flush the latest local state back to the App when the tab unmounts.
    useEffect(() => {
        return () => {
            onSessionChange(() => sessionRef.current);
        };
    }, [onSessionChange]);
    // Loaded failure artifacts for the entry whose "Artifacts" link was clicked.
    const [artifactView, setArtifactView] = useState<{
        dir: string;
        image: string | null;
        texts: { name: string; content: string }[];
    } | null>(null);
    const [artifactError, setArtifactError] = useState<string | null>(null);
    // Per-login scrape summaries and lock status for the console table.
    const [summaries, setSummaries] = useState<
        Record<string, LastScrapeSummary>
    >({});
    const [lockStatus, setLockStatus] = useState<LockStatusSnapshot | null>(
        null,
    );

    const ledgerPath = ledger?.path ?? null;

    // ─── Computed values ────────────────────────────────────────────────────────

    const activeScrapeLoginName = selectedLoginName.trim() || null;
    const hasActiveScrapeLogin = activeScrapeLoginName !== null;
    // The selected login's scrape (started from this tab) is in flight.
    const selectedRunning =
        activeScrapeLoginName !== null &&
        runningLoginName === activeScrapeLoginName;

    // ─── Effects ────────────────────────────────────────────────────────────────

    // Reset local-only state when the ledger path changes. The session-backed
    // running/status/console state is reset by the App (it swaps in a fresh
    // ScrapeTabSession on ledger open) and adopted via the [session] effect above.
    useEffect(() => {
        setScrapeLogEntries([]);
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

    // Clear the console when the user actually switches logins so it only shows
    // the selected login's output. Guarded by a ref so it does NOT fire on
    // (re)mount, which would wipe the console lines just adopted from the
    // App-held session after a tab switch.
    const prevConsoleLoginRef = useRef<string | null>(activeScrapeLoginName);
    useEffect(() => {
        if (prevConsoleLoginRef.current !== activeScrapeLoginName) {
            prevConsoleLoginRef.current = activeScrapeLoginName;
            setConsoleLines([]);
        }
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

    // Load per-login summaries and lock status for the console, and keep the
    // lock status live via the lock-status watcher (mirrors PipelineTab).
    useEffect(() => {
        if (ledgerPath === null || loginNames.length === 0) {
            setSummaries({});
            setLockStatus(null);
            return;
        }
        let cancelled = false;
        let unlisten: (() => void) | null = null;
        const refreshSummaries = () =>
            getLastScrapeSummaries(ledgerPath, loginNames)
                .then((s) => {
                    if (!cancelled) setSummaries(s);
                })
                .catch(() => {});
        const loadLocks = () =>
            getLockStatusSnapshot(ledgerPath, loginNames)
                .then((s) => {
                    if (!cancelled) setLockStatus(s);
                })
                .catch(() => {});
        void refreshSummaries();
        void loadLocks();
        void startLockMetadataWatch(ledgerPath)
            .then(() =>
                listen('refreshmint://lock-status-changed', () => {
                    void loadLocks();
                }),
            )
            .then((listener) => {
                if (cancelled) listener();
                else unlisten = listener;
            })
            .catch(() => {});
        return () => {
            cancelled = true;
            if (unlisten !== null) unlisten();
            void stopLockMetadataWatch();
        };
    }, [ledgerPath, loginNames, scrapeLogVersion]);

    // ─── Handlers ───────────────────────────────────────────────────────────────

    // Reload the per-login console summaries (called after a run completes).
    function loadSummaries() {
        if (ledgerPath === null || loginNames.length === 0) return;
        getLastScrapeSummaries(ledgerPath, loginNames)
            .then(setSummaries)
            .catch(() => {});
    }

    async function runScrapeFor(loginName: string) {
        if (!ledger) return;
        onSelectedLoginNameChange(loginName);
        const startStatus = `Running scrape for ${loginName}...`;
        setRunningLoginName(loginName);
        setConsoleLines([]);
        setScrapeStatus(startStatus);
        // Eagerly push the running transition to the App so a tab switch during
        // the scrape (which unmounts this tab) keeps the Running/Cancel state.
        commitSession({
            runningLoginName: loginName,
            scrapeStatus: startStatus,
            consoleLines: [],
        });
        let finalStatus = startStatus;
        try {
            await runScrapeForLogin(
                ledger.path,
                loginName,
                'manual',
                headlessScrape,
                promptTimeoutSecs,
            );
            finalStatus = `Scrape completed for ${loginName}.`;
            setScrapeStatus(finalStatus);
            await onScrapeComplete(loginName);
        } catch (error) {
            finalStatus = `Scrape failed: ${String(error)}`;
            setScrapeStatus(finalStatus);
        } finally {
            setRunningLoginName(null);
            // Push the terminal state even if this tab unmounted mid-scrape, so
            // the completion status isn't dropped on the next remount.
            commitSession({
                runningLoginName: null,
                scrapeStatus: finalStatus,
            });
            getScrapeLog(ledger.path, loginName)
                .then((entries) => {
                    setScrapeLogEntries(entries);
                })
                .catch(() => {});
            loadSummaries();
        }
    }

    async function handleRunScrape() {
        const loginName = activeScrapeLoginName;
        if (loginName === null) {
            setScrapeStatus('Login is required.');
            return;
        }
        await runScrapeFor(loginName);
    }

    async function handleCancelScrape() {
        const loginName = activeScrapeLoginName;
        if (loginName === null) return;
        try {
            await cancelScrape(loginName);
            setScrapeStatus(`Canceling scrape for ${loginName}...`);
        } catch (error) {
            setScrapeStatus(`Cancel failed: ${String(error)}`);
        }
    }

    async function handleViewArtifacts(dir: string) {
        if (!ledger) return;
        setArtifactError(null);
        try {
            const names = await listScrapeFailureArtifacts(ledger.path, dir);
            const { imageName, textNames } = partitionArtifacts(names);
            const image =
                imageName !== null
                    ? await readScrapeFailureArtifact(
                          ledger.path,
                          dir,
                          imageName,
                      )
                    : null;
            const texts = await Promise.all(
                textNames.map(async (name) => ({
                    name,
                    content: await readScrapeFailureArtifact(
                        ledger.path,
                        dir,
                        name,
                    ),
                })),
            );
            setArtifactView({ dir, image, texts });
        } catch (error) {
            setArtifactView(null);
            setArtifactError(`Failed to load artifacts: ${String(error)}`);
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
                            autoScrapeActive !== null ||
                            !hasActiveScrapeLogin ||
                            isLoadingScrapeExtensions
                        }
                    >
                        {selectedRunning ? 'Running scrape...' : 'Run scrape'}
                    </button>
                    {selectedRunning && (
                        <button
                            type="button"
                            className="secondary-button"
                            onClick={() => {
                                void handleCancelScrape();
                            }}
                        >
                            Cancel scrape
                        </button>
                    )}
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
                {loginNames.length > 0 && (
                    <div className="scrape-console-table-wrap">
                        <table className="scrape-log-table">
                            <thead>
                                <tr>
                                    <th>Login</th>
                                    <th>Last success</th>
                                    <th>Last result</th>
                                    <th>Lock</th>
                                    <th>Actions</th>
                                </tr>
                            </thead>
                            <tbody>
                                {loginNames.map((login) => {
                                    const summary:
                                        | LastScrapeSummary
                                        | undefined = summaries[login];
                                    const lock = lockStatus?.logins[login];
                                    const running = runningLoginName === login;
                                    return (
                                        <tr key={login}>
                                            <td>{login}</td>
                                            <td>
                                                {summary?.lastSuccess != null
                                                    ? new Date(
                                                          summary.lastSuccess,
                                                      ).toLocaleString()
                                                    : '—'}
                                            </td>
                                            <td>
                                                {summary?.lastRun != null
                                                    ? summary.lastRun.success
                                                        ? 'OK'
                                                        : 'Failed'
                                                    : '—'}
                                            </td>
                                            <td>
                                                {lock?.locked === true
                                                    ? 'Locked'
                                                    : ''}
                                            </td>
                                            <td>
                                                {running ? (
                                                    <button
                                                        type="button"
                                                        className="link-button"
                                                        onClick={() => {
                                                            void cancelScrape(
                                                                login,
                                                            );
                                                        }}
                                                    >
                                                        Cancel
                                                    </button>
                                                ) : (
                                                    <button
                                                        type="button"
                                                        className="link-button"
                                                        disabled={
                                                            isRunningScrape ||
                                                            autoScrapeActive !==
                                                                null
                                                        }
                                                        onClick={() => {
                                                            void runScrapeFor(
                                                                login,
                                                            );
                                                        }}
                                                    >
                                                        Run
                                                    </button>
                                                )}
                                            </td>
                                        </tr>
                                    );
                                })}
                            </tbody>
                        </table>
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
                                    <th>Artifacts</th>
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
                                        <td>
                                            {entry.artifactsDir !==
                                            undefined ? (
                                                <button
                                                    type="button"
                                                    className="link-button"
                                                    onClick={() => {
                                                        void handleViewArtifacts(
                                                            // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
                                                            entry.artifactsDir!,
                                                        );
                                                    }}
                                                >
                                                    View
                                                </button>
                                            ) : (
                                                ''
                                            )}
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                        {artifactError !== null && (
                            <p className="status status-error">
                                {artifactError}
                            </p>
                        )}
                        {artifactView !== null && (
                            <div className="scrape-artifacts">
                                <div className="scrape-artifacts-header">
                                    <span>
                                        Failure artifacts — {artifactView.dir}
                                    </span>
                                    <button
                                        type="button"
                                        className="link-button"
                                        onClick={() => {
                                            setArtifactView(null);
                                        }}
                                    >
                                        Close
                                    </button>
                                </div>
                                {artifactView.image !== null && (
                                    <img
                                        className="scrape-artifacts-image"
                                        src={artifactView.image}
                                        alt="Scrape failure screenshot"
                                    />
                                )}
                                {artifactView.texts.map((file) => (
                                    <div key={file.name}>
                                        <div className="scrape-console-header">
                                            {file.name}
                                        </div>
                                        <pre className="scrape-console-pane">
                                            {file.content}
                                        </pre>
                                    </div>
                                ))}
                            </div>
                        )}
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
