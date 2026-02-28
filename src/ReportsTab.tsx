import { useCallback, useRef, useState } from 'react';
import { getCurrentToken, getSearchSuggestions } from './search-utils.ts';
import {
    type AccountRow,
    type HledgerReportResult,
    runHledgerReport,
} from './tauri-commands.ts';

type ReportCommand =
    | 'balance'
    | 'balancesheet'
    | 'balancesheetequity'
    | 'cashflow'
    | 'incomestatement'
    | 'register'
    | 'aregister'
    | 'activity'
    | 'stats';

type Interval = '' | '-D' | '-W' | '-M' | '-Q' | '-Y';

type BalanceMode = '' | '--valuechange' | '--gain' | '--count';
type Accumulation = '' | '--cumulative' | '-H';
type BalanceView = '' | '-l' | '-t';

type RegisterAccumulation = '' | '--cumulative' | '-H';

const BALANCE_FAMILY: ReportCommand[] = [
    'balance',
    'balancesheet',
    'balancesheetequity',
    'cashflow',
    'incomestatement',
];

const REGISTER_FAMILY: ReportCommand[] = ['register', 'aregister'];

interface Props {
    ledger: string;
    accounts: AccountRow[];
}

export function ReportsTab({ ledger, accounts }: Props) {
    const [command, setCommand] = useState<ReportCommand>('balance');

    // Period
    const [beginDate, setBeginDate] = useState('');
    const [endDate, setEndDate] = useState('');
    const [interval, setInterval] = useState<Interval>('');

    // Filter
    const [statusCleared, setStatusCleared] = useState(false);
    const [statusPending, setStatusPending] = useState(false);
    const [statusUnmarked, setStatusUnmarked] = useState(false);
    const [realOnly, setRealOnly] = useState(false);
    const [showEmpty, setShowEmpty] = useState(false);
    const [depth, setDepth] = useState('');

    // Valuation
    const [valueCost, setValueCost] = useState(false);
    const [valueMarket, setValueMarket] = useState(false);
    const [exchangeCommodity, setExchangeCommodity] = useState('');

    // Balance-family options
    const [balanceMode, setBalanceMode] = useState<BalanceMode>('');
    const [accumulation, setAccumulation] = useState<Accumulation>('');
    const [balanceView, setBalanceView] = useState<BalanceView>('');
    const [showAverage, setShowAverage] = useState(false);
    const [showRowTotal, setShowRowTotal] = useState(false);
    const [summaryOnly, setSummaryOnly] = useState(false);
    const [noTotal, setNoTotal] = useState(false);
    const [sortAmount, setSortAmount] = useState(false);
    const [percent, setPercent] = useState(false);
    const [invert, setInvert] = useState(false);
    const [transpose, setTranspose] = useState(false);
    const [drop, setDrop] = useState('');

    // Register-family options
    const [regAccumulation, setRegAccumulation] =
        useState<RegisterAccumulation>('');
    const [regAverage, setRegAverage] = useState(false);
    const [regRelated, setRegRelated] = useState(false);
    const [regInvert, setRegInvert] = useState(false);

    // Query input
    const [queryInput, setQueryInput] = useState('');
    const [acSuggestions, setAcSuggestions] = useState<string[]>([]);
    const [acActiveIndex, setAcActiveIndex] = useState(-1);
    const queryInputRef = useRef<HTMLInputElement>(null);

    // Results
    const [result, setResult] = useState<HledgerReportResult | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [running, setRunning] = useState(false);

    const applyQueryCompletion = useCallback(
        (suggestion: string) => {
            const input = queryInputRef.current;
            if (!input) return;
            const cursorPos = input.selectionStart ?? queryInput.length;
            const { start, end } = getCurrentToken(queryInput, cursorPos);
            const newValue =
                queryInput.substring(0, start) +
                suggestion +
                ' ' +
                queryInput.substring(end);
            setQueryInput(newValue);
            setAcSuggestions([]);
            setAcActiveIndex(-1);
            // Move cursor after the inserted suggestion
            const newCursorPos = start + suggestion.length + 1;
            requestAnimationFrame(() => {
                input.setSelectionRange(newCursorPos, newCursorPos);
            });
        },
        [queryInput],
    );

    const buildArgs = useCallback((): string[] => {
        const args: string[] = [];

        if (beginDate.trim()) {
            args.push('-b', beginDate.trim());
        }
        if (endDate.trim()) {
            args.push('-e', endDate.trim());
        }
        if (interval) {
            args.push(interval);
        }

        // Status filters
        if (statusCleared) args.push('-C');
        if (statusPending) args.push('-P');
        if (statusUnmarked) args.push('-U');
        if (realOnly) args.push('-R');
        if (showEmpty) args.push('-E');
        if (depth.trim()) args.push(`--depth=${depth.trim()}`);

        // Valuation
        if (valueCost) args.push('-B');
        if (valueMarket) args.push('-V');
        if (exchangeCommodity.trim()) {
            args.push('-X', exchangeCommodity.trim());
        }

        const isBalanceFamily = BALANCE_FAMILY.includes(command);
        const isRegisterFamily = REGISTER_FAMILY.includes(command);

        if (isBalanceFamily) {
            if (balanceMode) args.push(balanceMode);
            if (accumulation) args.push(accumulation);
            if (balanceView) args.push(balanceView);
            if (showAverage) args.push('-A');
            if (showRowTotal) args.push('-T');
            if (summaryOnly) args.push('--summary-only');
            if (noTotal) args.push('-N');
            if (sortAmount) args.push('-S');
            if (percent) args.push('-%');
            if (invert) args.push('--invert');
            if (transpose) args.push('--transpose');
            if (drop.trim()) args.push(`--drop=${drop.trim()}`);
        }

        if (isRegisterFamily) {
            if (regAccumulation) args.push(regAccumulation);
            if (regAverage) args.push('-A');
            if (regRelated) args.push('-r');
            if (regInvert) args.push('--invert');
        }

        // Query tokens
        const trimmed = queryInput.trim();
        if (trimmed) {
            args.push(...trimmed.split(/\s+/));
        }

        return args;
    }, [
        beginDate,
        endDate,
        interval,
        statusCleared,
        statusPending,
        statusUnmarked,
        realOnly,
        showEmpty,
        depth,
        valueCost,
        valueMarket,
        exchangeCommodity,
        command,
        balanceMode,
        accumulation,
        balanceView,
        showAverage,
        showRowTotal,
        summaryOnly,
        noTotal,
        sortAmount,
        percent,
        invert,
        transpose,
        drop,
        regAccumulation,
        regAverage,
        regRelated,
        regInvert,
        queryInput,
    ]);

    const handleRun = useCallback(async () => {
        setRunning(true);
        setError(null);
        setResult(null);
        try {
            const args = buildArgs();
            const res = await runHledgerReport(ledger, command, args);
            setResult(res);
        } catch (e) {
            setError(String(e));
        } finally {
            setRunning(false);
        }
    }, [ledger, command, buildArgs]);

    const isBalanceFamily = BALANCE_FAMILY.includes(command);
    const isRegisterFamily = REGISTER_FAMILY.includes(command);

    return (
        <div className="transactions-panel">
            <section className="txn-form">
                <div className="txn-form-header">
                    <h2>Reports</h2>
                </div>

                {/* Command selector */}
                <div className="field-group">
                    <label className="field-label">Command</label>
                    <div className="tabs">
                        {(
                            [
                                ['balance', 'Balance'],
                                ['balancesheet', 'Balance Sheet'],
                                ['balancesheetequity', 'BS+Equity'],
                                ['cashflow', 'Cash Flow'],
                                ['incomestatement', 'Income Stmt'],
                                ['register', 'Register'],
                                ['aregister', 'Account Reg'],
                                ['activity', 'Activity'],
                                ['stats', 'Stats'],
                            ] as [ReportCommand, string][]
                        ).map(([cmd, label]) => (
                            <button
                                key={cmd}
                                type="button"
                                className={
                                    command === cmd ? 'tab active' : 'tab'
                                }
                                onClick={() => {
                                    setCommand(cmd);
                                }}
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                </div>

                {/* Period */}
                <div className="field-group">
                    <label className="field-label">Period</label>
                    <div className="field-row">
                        <label className="field-label-sm">Begin</label>
                        <input
                            type="text"
                            className="date-input"
                            placeholder="YYYY-MM-DD"
                            value={beginDate}
                            onChange={(e) => {
                                setBeginDate(e.target.value);
                            }}
                        />
                        <label className="field-label-sm">End</label>
                        <input
                            type="text"
                            className="date-input"
                            placeholder="YYYY-MM-DD"
                            value={endDate}
                            onChange={(e) => {
                                setEndDate(e.target.value);
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
                                        setInterval(flag);
                                    }}
                                >
                                    {label}
                                </button>
                            ))}
                        </div>
                    </div>
                </div>

                {/* Filters */}
                <div className="field-group">
                    <label className="field-label">Filters</label>
                    <div className="field-row checkbox-row">
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={statusCleared}
                                onChange={(e) => {
                                    setStatusCleared(e.target.checked);
                                }}
                            />
                            <span>Cleared (-C)</span>
                        </label>
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={statusPending}
                                onChange={(e) => {
                                    setStatusPending(e.target.checked);
                                }}
                            />
                            <span>Pending (-P)</span>
                        </label>
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={statusUnmarked}
                                onChange={(e) => {
                                    setStatusUnmarked(e.target.checked);
                                }}
                            />
                            <span>Unmarked (-U)</span>
                        </label>
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={realOnly}
                                onChange={(e) => {
                                    setRealOnly(e.target.checked);
                                }}
                            />
                            <span>Real only (-R)</span>
                        </label>
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={showEmpty}
                                onChange={(e) => {
                                    setShowEmpty(e.target.checked);
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
                                value={depth}
                                onChange={(e) => {
                                    setDepth(e.target.value);
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
                                checked={valueCost}
                                onChange={(e) => {
                                    setValueCost(e.target.checked);
                                }}
                            />
                            <span>Cost basis (-B)</span>
                        </label>
                        <label className="checkbox-field">
                            <input
                                type="checkbox"
                                checked={valueMarket}
                                onChange={(e) => {
                                    setValueMarket(e.target.checked);
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
                                value={exchangeCommodity}
                                onChange={(e) => {
                                    setExchangeCommodity(e.target.value);
                                }}
                            />
                        </label>
                    </div>
                </div>

                {/* Balance-family options */}
                {isBalanceFamily && (
                    <>
                        <div className="field-group">
                            <label className="field-label">Calculation</label>
                            <div className="field-row">
                                <div className="tabs">
                                    {(
                                        [
                                            ['', 'Sum'],
                                            ['--valuechange', 'Value Change'],
                                            ['--gain', 'Gain'],
                                            ['--count', 'Count'],
                                        ] as [BalanceMode, string][]
                                    ).map(([flag, label]) => (
                                        <button
                                            key={flag || 'sum'}
                                            type="button"
                                            className={
                                                balanceMode === flag
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                setBalanceMode(flag);
                                            }}
                                        >
                                            {label}
                                        </button>
                                    ))}
                                </div>
                            </div>
                        </div>
                        <div className="field-group">
                            <label className="field-label">Accumulation</label>
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
                                                accumulation === flag
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                setAccumulation(flag);
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
                                                balanceView === flag
                                                    ? 'tab active'
                                                    : 'tab'
                                            }
                                            onClick={() => {
                                                setBalanceView(flag);
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
                                        checked={showAverage}
                                        onChange={(e) => {
                                            setShowAverage(e.target.checked);
                                        }}
                                    />
                                    <span>Average (-A)</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={showRowTotal}
                                        onChange={(e) => {
                                            setShowRowTotal(e.target.checked);
                                        }}
                                    />
                                    <span>Row total (-T)</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={summaryOnly}
                                        onChange={(e) => {
                                            setSummaryOnly(e.target.checked);
                                        }}
                                    />
                                    <span>Summary only</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={noTotal}
                                        onChange={(e) => {
                                            setNoTotal(e.target.checked);
                                        }}
                                    />
                                    <span>No total (-N)</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={sortAmount}
                                        onChange={(e) => {
                                            setSortAmount(e.target.checked);
                                        }}
                                    />
                                    <span>Sort by amount (-S)</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={percent}
                                        onChange={(e) => {
                                            setPercent(e.target.checked);
                                        }}
                                    />
                                    <span>Percent (-%)</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={invert}
                                        onChange={(e) => {
                                            setInvert(e.target.checked);
                                        }}
                                    />
                                    <span>Invert</span>
                                </label>
                                <label className="checkbox-field">
                                    <input
                                        type="checkbox"
                                        checked={transpose}
                                        onChange={(e) => {
                                            setTranspose(e.target.checked);
                                        }}
                                    />
                                    <span>Transpose</span>
                                </label>
                                <label className="checkbox-field">
                                    <span>Drop</span>
                                    <input
                                        type="number"
                                        className="small-number-input"
                                        min="0"
                                        value={drop}
                                        onChange={(e) => {
                                            setDrop(e.target.value);
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
                        <label className="field-label">Register Options</label>
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
                                            regAccumulation === flag
                                                ? 'tab active'
                                                : 'tab'
                                        }
                                        onClick={() => {
                                            setRegAccumulation(flag);
                                        }}
                                    >
                                        {label}
                                    </button>
                                ))}
                            </div>
                        </div>
                        <div className="field-row checkbox-row">
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={regAverage}
                                    onChange={(e) => {
                                        setRegAverage(e.target.checked);
                                    }}
                                />
                                <span>Average (-A)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={regRelated}
                                    onChange={(e) => {
                                        setRegRelated(e.target.checked);
                                    }}
                                />
                                <span>Related (-r)</span>
                            </label>
                            <label className="checkbox-field">
                                <input
                                    type="checkbox"
                                    checked={regInvert}
                                    onChange={(e) => {
                                        setRegInvert(e.target.checked);
                                    }}
                                />
                                <span>Invert</span>
                            </label>
                        </div>
                    </div>
                )}

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
                                setQueryInput(val);
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
                        onClick={() => void handleRun()}
                    >
                        {running ? 'Running…' : 'Run'}
                    </button>
                </div>

                {/* Error */}
                {error !== null && (
                    <div className="error-message">
                        <pre>{error}</pre>
                    </div>
                )}
            </section>

            {/* Results */}
            {result !== null && (
                <div className="table-wrap">
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
