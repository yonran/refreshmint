import { useCallback, useEffect, useRef, useState } from 'react';
import { ReportChart } from './ReportChart.tsx';
import { getCurrentToken, getSearchSuggestions } from '../search-utils.ts';
import {
    type AccountRow,
    type HledgerReportResult,
    runHledgerReport,
} from '../tauri-commands.ts';
import {
    BALANCE_FAMILY,
    COMMAND_LABELS,
    REGISTER_FAMILY,
    buildReportArgs,
    buildUnknownJumpSearch,
    buildUnknownRegisterArgs,
    cannedReportConfig,
    computePeriodPresetRange,
    isReportConfigStale,
    shouldAutoRunDefault,
    summarizeUnknownRegister,
    type Accumulation,
    type CannedReportId,
    type BalanceMode,
    type BalanceView,
    type Interval,
    type PeriodPreset,
    type ReportConfig,
    type RegisterAccumulation,
} from '../report-utils.ts';
import type { ReportsTabSession } from '../types.ts';

interface Props {
    ledger: string;
    accounts: AccountRow[];
    session: ReportsTabSession;
    onSessionChange: (
        updater: (current: ReportsTabSession) => ReportsTabSession,
    ) => void;
    // Cross-tab navigation: open the Transactions tab pre-filtered to `search`.
    onNavigateToTransactions: (search: string) => void;
}

export function ReportsTab({
    ledger,
    accounts,
    session,
    onSessionChange,
    onNavigateToTransactions,
}: Props) {
    const [config, setConfig] = useState<ReportConfig>(session.config);
    // Shallow-merge a partial update into the single config atom.
    const patch = useCallback((partial: Partial<ReportConfig>) => {
        setConfig((current) => ({ ...current, ...partial }));
    }, []);

    // Autocomplete state (view-only, not part of the report request).
    const [acSuggestions, setAcSuggestions] = useState<string[]>([]);
    const [acActiveIndex, setAcActiveIndex] = useState(-1);
    const queryInputRef = useRef<HTMLInputElement>(null);

    // Results
    const [result, setResult] = useState<HledgerReportResult | null>(
        session.result,
    );
    const [error, setError] = useState<string | null>(session.error);
    const [running, setRunning] = useState(false);
    // Data-quality banner: Expenses:Unknown activity in the last run's date
    // range. Recomputed after every successful run; not persisted in the session.
    const [unknownSummary, setUnknownSummary] = useState<{
        txnCount: number;
        total: string;
    } | null>(null);
    // lastRunConfig/hasAutoRun are persisted across tab switches but are only
    // read/written by later features (stale hint, default auto-run); mirror them
    // through refs so they survive the App-held session round-trip.
    const lastRunConfigRef = useRef<ReportConfig | null>(session.lastRunConfig);
    const hasAutoRunRef = useRef<boolean>(session.hasAutoRun);

    // Keep a live snapshot of everything the App-held session tracks so it can be
    // flushed back on unmount (mirrors the TransactionsTab persistence pattern).
    const sessionRef = useRef<ReportsTabSession>(session);
    sessionRef.current = {
        config,
        result,
        error,
        lastRunConfig: lastRunConfigRef.current,
        hasAutoRun: hasAutoRunRef.current,
    };

    // Adopt the incoming session when it changes (e.g. after a ledger reset).
    useEffect(() => {
        setConfig(session.config);
        setResult(session.result);
        setError(session.error);
        lastRunConfigRef.current = session.lastRunConfig;
        hasAutoRunRef.current = session.hasAutoRun;
    }, [session]);

    // Flush the latest local state back to the App when the tab unmounts.
    useEffect(() => {
        return () => {
            onSessionChange(() => sessionRef.current);
        };
    }, [onSessionChange]);

    const { command, interval, queryInput } = config;

    const applyQueryCompletion = useCallback(
        (suggestion: string) => {
            const input = queryInputRef.current;
            if (!input) return;
            const currentQuery = input.value;
            const cursorPos = input.selectionStart ?? currentQuery.length;
            const { start, end } = getCurrentToken(currentQuery, cursorPos);
            const newValue =
                currentQuery.substring(0, start) +
                suggestion +
                ' ' +
                currentQuery.substring(end);
            patch({ queryInput: newValue });
            setAcSuggestions([]);
            setAcActiveIndex(-1);
            // Move cursor after the inserted suggestion
            const newCursorPos = start + suggestion.length + 1;
            requestAnimationFrame(() => {
                input.setSelectionRange(newCursorPos, newCursorPos);
            });
        },
        [patch],
    );

    // Run an explicit config. Taking the config as an argument (rather than
    // reading component state) lets canned reports set the whole config and run
    // it in the same click without waiting for a setState to flush.
    const runReport = useCallback(
        async (cfg: ReportConfig) => {
            setRunning(true);
            setError(null);
            setResult(null);
            setUnknownSummary(null);
            let ok = false;
            try {
                const args = buildReportArgs(cfg);
                const res = await runHledgerReport(
                    ledger,
                    cfg.command,
                    args,
                    cfg.budget,
                );
                setResult(res);
                ok = true;
            } catch (e) {
                setError(String(e));
            } finally {
                // Record the config that produced the current output (on both
                // success and error) so the stale-result hint can compare against
                // subsequent edits.
                lastRunConfigRef.current = cfg;
                setRunning(false);
            }
            // Data-quality banner: a secondary register query over the run's date
            // range. It runs after the main report is already displayed and its
            // failures are non-fatal (logged, never surfaced as a report error).
            if (ok) {
                try {
                    const unkArgs = buildUnknownRegisterArgs(
                        cfg.beginDate,
                        cfg.endDate,
                    );
                    const unkRes = await runHledgerReport(
                        ledger,
                        'register',
                        unkArgs,
                    );
                    setUnknownSummary(summarizeUnknownRegister(unkRes.rows));
                } catch (e) {
                    console.warn('Expenses:Unknown banner query failed', e);
                }
            }
        },
        [ledger],
    );

    const handleRun = useCallback(() => {
        void runReport(config);
    }, [runReport, config]);

    const runCannedReport = useCallback(
        (id: CannedReportId) => {
            const cfg = cannedReportConfig(id, new Date());
            setConfig(cfg);
            void runReport(cfg);
        },
        [runReport],
    );

    // On first open for a ledger, auto-run a useful default so the tab isn't a
    // blank form. Session persistence (result/hasAutoRun) makes this a no-op when
    // returning to an already-populated tab; a ledger open resets the session, so
    // the next mount auto-runs again. Mount-only — deps intentionally empty.
    useEffect(() => {
        if (shouldAutoRunDefault(sessionRef.current)) {
            hasAutoRunRef.current = true;
            runCannedReport('spending-by-category');
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    const isBalanceFamily = BALANCE_FAMILY.includes(command);
    const isRegisterFamily = REGISTER_FAMILY.includes(command);

    const showChart =
        result !== null &&
        result.rows.length > 1 &&
        result.text === null &&
        (command === 'register' ||
            (BALANCE_FAMILY.includes(command) && interval !== ''));

    const chartKind: 'register' | 'balance-interval' =
        command === 'register' ? 'register' : 'balance-interval';

    // Show a hint when the options have been edited since the displayed output
    // was produced. Suppressed while a run is in flight.
    const isStale =
        !running && isReportConfigStale(config, lastRunConfigRef.current);

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <h2>Reports</h2>
                </div>

                {/* Canned one-click reports */}
                <div className="field-group">
                    <label className="field-label">Quick reports</label>
                    <div className="field-row">
                        {(
                            [
                                [
                                    'spending-by-category',
                                    'Spending by category',
                                ],
                                ['income-vs-expense', 'Income vs. expense'],
                                ['net-worth-trend', 'Net worth trend'],
                                ['budget', 'Budget vs. actual'],
                            ] as [CannedReportId, string][]
                        ).map(([id, label]) => (
                            <button
                                key={id}
                                type="button"
                                className="tab"
                                disabled={running}
                                onClick={() => {
                                    runCannedReport(id);
                                }}
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                </div>

                {/* Command selector */}
                <div className="field-group">
                    <label className="field-label">Report</label>
                    <div className="tabs report-command-tabs">
                        {COMMAND_LABELS.map(
                            ({ command: cmd, label, tooltip }) => (
                                <button
                                    key={cmd}
                                    type="button"
                                    title={tooltip}
                                    className={
                                        command === cmd ? 'tab active' : 'tab'
                                    }
                                    onClick={() => {
                                        patch({ command: cmd });
                                    }}
                                >
                                    {label}
                                </button>
                            ),
                        )}
                    </div>
                </div>

                {/* Period */}
                <div className="field-group">
                    <label className="field-label">Period</label>
                    <div className="field-row">
                        {(
                            [
                                ['this-month', 'This month'],
                                ['last-month', 'Last month'],
                                ['ytd', 'Year to date'],
                                ['last-12-months', 'Last 12 months'],
                            ] as [PeriodPreset, string][]
                        ).map(([preset, label]) => (
                            <button
                                key={preset}
                                type="button"
                                className="tab"
                                onClick={() => {
                                    const { begin, end } =
                                        computePeriodPresetRange(
                                            preset,
                                            new Date(),
                                        );
                                    patch({ beginDate: begin, endDate: end });
                                }}
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                    <div className="field-row">
                        <label className="field-label-sm">Begin</label>
                        <input
                            type="date"
                            className="date-input"
                            value={config.beginDate}
                            onChange={(e) => {
                                patch({ beginDate: e.target.value });
                            }}
                        />
                        <label className="field-label-sm">End</label>
                        <input
                            type="date"
                            className="date-input"
                            value={config.endDate}
                            onChange={(e) => {
                                patch({ endDate: e.target.value });
                            }}
                        />
                    </div>
                    <div className="field-row">
                        <label className="field-label-sm">Interval</label>
                        <div className="tabs">
                            {(
                                [
                                    ['', 'None'],
                                    ['-D', 'Daily'],
                                    ['-W', 'Weekly'],
                                    ['-M', 'Monthly'],
                                    ['-Q', 'Quarterly'],
                                    ['-Y', 'Yearly'],
                                ] as [Interval, string][]
                            ).map(([flag, label]) => (
                                <button
                                    key={flag || 'none'}
                                    type="button"
                                    className={
                                        interval === flag ? 'tab active' : 'tab'
                                    }
                                    onClick={() => {
                                        patch({ interval: flag });
                                    }}
                                >
                                    {label}
                                </button>
                            ))}
                        </div>
                    </div>
                </div>

                {/* Query input */}
                <div className="field-group">
                    <label className="field-label">Query</label>
                    <div className="search-bar-wrapper">
                        <input
                            ref={queryInputRef}
                            type="search"
                            placeholder="hledger query: desc:amazon acct:^Expenses date:thismonth"
                            value={queryInput}
                            onChange={(e) => {
                                const val = e.target.value;
                                patch({ queryInput: val });
                                const cursorPos =
                                    e.target.selectionStart ?? val.length;
                                const { token, start } = getCurrentToken(
                                    val,
                                    cursorPos,
                                );
                                const cursorOffsetInToken = cursorPos - start;
                                const sugs = getSearchSuggestions(
                                    token,
                                    cursorOffsetInToken,
                                    accounts,
                                );
                                setAcSuggestions(sugs);
                                setAcActiveIndex(-1);
                            }}
                            onKeyDown={(e) => {
                                if (acSuggestions.length === 0) return;
                                if (e.key === 'ArrowDown') {
                                    e.preventDefault();
                                    setAcActiveIndex((i) =>
                                        Math.min(
                                            i + 1,
                                            acSuggestions.length - 1,
                                        ),
                                    );
                                } else if (e.key === 'ArrowUp') {
                                    e.preventDefault();
                                    setAcActiveIndex((i) => Math.max(i - 1, 0));
                                } else if (
                                    (e.key === 'Enter' || e.key === 'Tab') &&
                                    acActiveIndex >= 0
                                ) {
                                    e.preventDefault();
                                    applyQueryCompletion(
                                        acSuggestions[acActiveIndex] ?? '',
                                    );
                                } else if (e.key === 'Escape') {
                                    setAcSuggestions([]);
                                    setAcActiveIndex(-1);
                                }
                            }}
                            onBlur={() => {
                                setTimeout(() => {
                                    setAcSuggestions([]);
                                    setAcActiveIndex(-1);
                                }, 150);
                            }}
                        />
                        {acSuggestions.length > 0 && (
                            <div className="search-autocomplete" role="listbox">
                                {acSuggestions.map((sug, i) => (
                                    <div
                                        key={sug}
                                        className={`ac-item${i === acActiveIndex ? ' active' : ''}`}
                                        role="option"
                                        aria-selected={i === acActiveIndex}
                                        onMouseDown={(e) => {
                                            e.preventDefault();
                                            applyQueryCompletion(sug);
                                        }}
                                    >
                                        {sug}
                                    </div>
                                ))}
                            </div>
                        )}
                    </div>
                </div>

                {/* Run button */}
                <div className="field-group">
                    <button
                        type="button"
                        className="primary-button"
                        disabled={running}
                        onClick={handleRun}
                    >
                        {running ? 'Running…' : 'Run'}
                    </button>
                </div>

                {/* Options (collapsible) */}
                <details className="field-group">
                    <summary className="field-label">Options</summary>

                    {/* Filters */}
                    <div className="field-group">
                        <label className="field-label">Filters</label>
                        <div className="field-row checkbox-row">
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.statusCleared}
                                    onChange={(e) => {
                                        patch({
                                            statusCleared: e.target.checked,
                                        });
                                    }}
                                />
                                <span>Cleared (-C, hledger status)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.statusPending}
                                    onChange={(e) => {
                                        patch({
                                            statusPending: e.target.checked,
                                        });
                                    }}
                                />
                                <span>Pending (-P, hledger status)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.statusUnmarked}
                                    onChange={(e) => {
                                        patch({
                                            statusUnmarked: e.target.checked,
                                        });
                                    }}
                                />
                                <span>Unmarked (-U, hledger status)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.realOnly}
                                    onChange={(e) => {
                                        patch({ realOnly: e.target.checked });
                                    }}
                                />
                                <span>Real only (-R)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.showEmpty}
                                    onChange={(e) => {
                                        patch({ showEmpty: e.target.checked });
                                    }}
                                />
                                <span>Show empty (-E)</span>
                            </label>
                            <label className="checkbox-field">
                                <span>Depth</span>
                                <input
                                    type="number"
                                    className="small-number-input"
                                    min="1"
                                    value={config.depth}
                                    onChange={(e) => {
                                        patch({ depth: e.target.value });
                                    }}
                                    placeholder="N"
                                />
                            </label>
                        </div>
                    </div>

                    {/* Valuation */}
                    <div className="field-group">
                        <label className="field-label">Valuation</label>
                        <div className="field-row checkbox-row">
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.valueCost}
                                    onChange={(e) => {
                                        patch({ valueCost: e.target.checked });
                                    }}
                                />
                                <span>Cost basis (-B)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={config.valueMarket}
                                    onChange={(e) => {
                                        patch({
                                            valueMarket: e.target.checked,
                                        });
                                    }}
                                />
                                <span>Market value (-V)</span>
                            </label>
                            <label className="checkbox-field">
                                <span>Exchange</span>
                                <input
                                    type="text"
                                    className="small-text-input"
                                    placeholder="COMM"
                                    value={config.exchangeCommodity}
                                    onChange={(e) => {
                                        patch({
                                            exchangeCommodity: e.target.value,
                                        });
                                    }}
                                />
                            </label>
                        </div>
                    </div>

                    {/* Balance-family options */}
                    {isBalanceFamily && (
                        <>
                            <div className="field-group">
                                <label className="field-label">
                                    Calculation
                                </label>
                                <div className="field-row">
                                    <div className="tabs">
                                        {(
                                            [
                                                ['', 'Sum'],
                                                [
                                                    '--valuechange',
                                                    'Value Change',
                                                ],
                                                ['--gain', 'Gain'],
                                                ['--count', 'Count'],
                                            ] as [BalanceMode, string][]
                                        ).map(([flag, label]) => (
                                            <button
                                                key={flag || 'sum'}
                                                type="button"
                                                className={
                                                    config.balanceMode === flag
                                                        ? 'tab active'
                                                        : 'tab'
                                                }
                                                onClick={() => {
                                                    patch({
                                                        balanceMode: flag,
                                                    });
                                                }}
                                            >
                                                {label}
                                            </button>
                                        ))}
                                    </div>
                                </div>
                            </div>
                            <div className="field-group">
                                <label className="field-label">
                                    Accumulation
                                </label>
                                <div className="field-row">
                                    <div className="tabs">
                                        {(
                                            [
                                                ['', 'Change'],
                                                ['--cumulative', 'Cumulative'],
                                                ['-H', 'Historical'],
                                            ] as [Accumulation, string][]
                                        ).map(([flag, label]) => (
                                            <button
                                                key={flag || 'change'}
                                                type="button"
                                                className={
                                                    config.accumulation === flag
                                                        ? 'tab active'
                                                        : 'tab'
                                                }
                                                onClick={() => {
                                                    patch({
                                                        accumulation: flag,
                                                    });
                                                }}
                                            >
                                                {label}
                                            </button>
                                        ))}
                                    </div>
                                </div>
                            </div>
                            <div className="field-group">
                                <label className="field-label">View</label>
                                <div className="field-row">
                                    <div className="tabs">
                                        {(
                                            [
                                                ['', 'Default'],
                                                ['-l', 'Flat'],
                                                ['-t', 'Tree'],
                                            ] as [BalanceView, string][]
                                        ).map(([flag, label]) => (
                                            <button
                                                key={flag || 'default'}
                                                type="button"
                                                className={
                                                    config.balanceView === flag
                                                        ? 'tab active'
                                                        : 'tab'
                                                }
                                                onClick={() => {
                                                    patch({
                                                        balanceView: flag,
                                                    });
                                                }}
                                            >
                                                {label}
                                            </button>
                                        ))}
                                    </div>
                                </div>
                            </div>
                            <div className="field-group">
                                <label className="field-label">
                                    Columns &amp; Display
                                </label>
                                <div className="field-row checkbox-row">
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.showAverage}
                                            onChange={(e) => {
                                                patch({
                                                    showAverage:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Average (-A)</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.showRowTotal}
                                            onChange={(e) => {
                                                patch({
                                                    showRowTotal:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Row total (-T)</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.summaryOnly}
                                            onChange={(e) => {
                                                patch({
                                                    summaryOnly:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Summary only</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.noTotal}
                                            onChange={(e) => {
                                                patch({
                                                    noTotal: e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>No total (-N)</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.sortAmount}
                                            onChange={(e) => {
                                                patch({
                                                    sortAmount:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Sort by amount (-S)</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.percent}
                                            onChange={(e) => {
                                                patch({
                                                    percent: e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Percent (-%)</span>
                                    </label>
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.budget}
                                            onChange={(e) => {
                                                patch({
                                                    budget: e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Budget (--budget)</span>
                                    </label>
                                    {command === 'balance' && (
                                        <label className="checkbox-field">
                                            <input
                                                type="checkbox"
                                                checked={config.invert}
                                                onChange={(e) => {
                                                    patch({
                                                        invert: e.target
                                                            .checked,
                                                    });
                                                }}
                                            />
                                            <span>Invert</span>
                                        </label>
                                    )}
                                    {command === 'balance' && (
                                        <label className="checkbox-field">
                                            <input
                                                type="checkbox"
                                                checked={config.transpose}
                                                onChange={(e) => {
                                                    patch({
                                                        transpose:
                                                            e.target.checked,
                                                    });
                                                }}
                                            />
                                            <span>Transpose</span>
                                        </label>
                                    )}
                                    <label className="checkbox-field">
                                        <span>Drop</span>
                                        <input
                                            type="number"
                                            className="small-number-input"
                                            min="0"
                                            value={config.drop}
                                            onChange={(e) => {
                                                patch({ drop: e.target.value });
                                            }}
                                            placeholder="N"
                                        />
                                    </label>
                                </div>
                            </div>
                        </>
                    )}

                    {/* Register-family options */}
                    {isRegisterFamily && (
                        <div className="field-group">
                            <label className="field-label">
                                Register Options
                            </label>
                            <div className="field-row">
                                <div className="tabs">
                                    {(
                                        [
                                            ['', 'Change'],
                                            ['--cumulative', 'Cumulative'],
                                            ['-H', 'Historical'],
                                        ] as [RegisterAccumulation, string][]
                                    ).map(([flag, label]) => (
                                        <button
                                            key={flag || 'change'}
                                            type="button"
                                            className={
                                                config.regAccumulation === flag
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                patch({
                                                    regAccumulation: flag,
                                                });
                                            }}
                                        >
                                            {label}
                                        </button>
                                    ))}
                                </div>
                            </div>
                            <div className="field-row checkbox-row">
                                {command !== 'aregister' && (
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.regAverage}
                                            onChange={(e) => {
                                                patch({
                                                    regAverage:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Average (-A)</span>
                                    </label>
                                )}
                                {command !== 'aregister' && (
                                    <label className="checkbox-field">
                                        <input
                                            type="checkbox"
                                            checked={config.regRelated}
                                            onChange={(e) => {
                                                patch({
                                                    regRelated:
                                                        e.target.checked,
                                                });
                                            }}
                                        />
                                        <span>Related (-r)</span>
                                    </label>
                                )}
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={config.regInvert}
                                        onChange={(e) => {
                                            patch({
                                                regInvert: e.target.checked,
                                            });
                                        }}
                                    />
                                    <span>Invert</span>
                                </label>
                            </div>
                        </div>
                    )}
                </details>

                {/* Error */}
                {error !== null && (
                    <div className="error-message">
                        <pre>{error}</pre>
                    </div>
                )}

                {/* Stale-result hint */}
                {isStale && (
                    <div className="report-stale-hint">
                        Options changed — results may be stale. Re-run to
                        refresh.
                    </div>
                )}

                {/* Data-quality banner: uncategorized activity in this period */}
                {unknownSummary !== null && unknownSummary.txnCount > 0 && (
                    <div className="report-unknown-banner">
                        <span>
                            {unknownSummary.txnCount}{' '}
                            {unknownSummary.txnCount === 1
                                ? 'transaction'
                                : 'transactions'}
                            {unknownSummary.total
                                ? ` ($${unknownSummary.total})`
                                : ''}{' '}
                            still in Expenses:Unknown in this period.
                        </span>
                        <button
                            type="button"
                            className="tab"
                            onClick={() => {
                                const last = lastRunConfigRef.current;
                                onNavigateToTransactions(
                                    buildUnknownJumpSearch(
                                        last?.beginDate ?? '',
                                        last?.endDate ?? '',
                                    ),
                                );
                            }}
                        >
                            Categorize them
                        </button>
                    </div>
                )}
            </section>

            {/* Results */}
            {result !== null && (
                <div className="table-wrap">
                    {showChart && (
                        <ReportChart rows={result.rows} kind={chartKind} />
                    )}
                    {result.text !== null ? (
                        <pre className="report-text">{result.text}</pre>
                    ) : result.rows.length === 0 ? (
                        <p className="no-results">No results.</p>
                    ) : (
                        <table className="report-table">
                            <thead>
                                <tr>
                                    {result.rows[0]?.map((cell, i) => (
                                        <th key={i}>{cell}</th>
                                    ))}
                                </tr>
                            </thead>
                            <tbody>
                                {result.rows.slice(1).map((row, ri) => {
                                    // Section header rows: all cells after the first are empty
                                    const isSectionHeader =
                                        row.length > 1 &&
                                        row.slice(1).every((c) => c === '');
                                    return (
                                        <tr
                                            key={ri}
                                            className={
                                                isSectionHeader
                                                    ? 'section-header'
                                                    : undefined
                                            }
                                        >
                                            {row.map((cell, ci) => {
                                                const isNum =
                                                    cell !== '' &&
                                                    !isNaN(parseFloat(cell));
                                                return (
                                                    <td
                                                        key={ci}
                                                        style={
                                                            isNum
                                                                ? {
                                                                      textAlign:
                                                                          'right',
                                                                  }
                                                                : undefined
                                                        }
                                                    >
                                                        {isSectionHeader &&
                                                        ci === 0 ? (
                                                            <strong>
                                                                {cell}
                                                            </strong>
                                                        ) : (
                                                            cell
                                                        )}
                                                    </td>
                                                );
                                            })}
                                        </tr>
                                    );
                                })}
                            </tbody>
                        </table>
                    )}
                </div>
            )}
        </div>
    );
}
