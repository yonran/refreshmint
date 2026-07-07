// Pure helpers for the Reports tab. Types and buildReportArgs are extracted from
// ReportsTab.tsx so they can be unit-tested and reused (canned reports, presets,
// stale-result detection). Keep buildReportArgs behavior identical to the
// original ReportsTab.buildArgs — the report-utils.test.ts suite pins it.

export type ReportCommand =
    | 'balance'
    | 'balancesheet'
    | 'balancesheetequity'
    | 'cashflow'
    | 'incomestatement'
    | 'register'
    | 'aregister'
    | 'activity'
    | 'stats';

export type Interval = '' | '-D' | '-W' | '-M' | '-Q' | '-Y';

export type BalanceMode = '' | '--valuechange' | '--gain' | '--count';
export type Accumulation = '' | '--cumulative' | '-H';
export type BalanceView = '' | '-l' | '-t';

export type RegisterAccumulation = '' | '--cumulative' | '-H';

export const BALANCE_FAMILY: ReportCommand[] = [
    'balance',
    'balancesheet',
    'balancesheetequity',
    'cashflow',
    'incomestatement',
];

export const REGISTER_FAMILY: ReportCommand[] = ['register', 'aregister'];

/**
 * Plain-language labels for the report command selector. `tooltip` is the
 * underlying hledger command, surfaced via a title= attribute so power users can
 * still see what runs. Order here is the display order in the selector.
 */
export const COMMAND_LABELS: {
    command: ReportCommand;
    label: string;
    tooltip: string;
}[] = [
    {
        command: 'balance',
        label: 'Account Balances',
        tooltip: 'hledger balance',
    },
    {
        command: 'balancesheet',
        label: 'Balance Sheet',
        tooltip: 'hledger balancesheet',
    },
    {
        command: 'balancesheetequity',
        label: 'Balance Sheet + Equity',
        tooltip: 'hledger balancesheetequity',
    },
    {
        command: 'incomestatement',
        label: 'Income Statement',
        tooltip: 'hledger incomestatement',
    },
    { command: 'cashflow', label: 'Cash Flow', tooltip: 'hledger cashflow' },
    {
        command: 'register',
        label: 'Transaction Register',
        tooltip: 'hledger register',
    },
    {
        command: 'aregister',
        label: 'Account Register',
        tooltip: 'hledger aregister',
    },
    {
        command: 'activity',
        label: 'Activity Chart',
        tooltip: 'hledger activity',
    },
    { command: 'stats', label: 'Statistics', tooltip: 'hledger stats' },
];

/**
 * The full set of report options, one field per useState atom that previously
 * lived in ReportsTab. Autocomplete/results/running state stay local to the
 * component; only the request-shaping fields live here.
 */
export interface ReportConfig {
    command: ReportCommand;

    // Period
    beginDate: string;
    endDate: string;
    interval: Interval;

    // Filter
    statusCleared: boolean;
    statusPending: boolean;
    statusUnmarked: boolean;
    realOnly: boolean;
    showEmpty: boolean;
    depth: string;

    // Valuation
    valueCost: boolean;
    valueMarket: boolean;
    exchangeCommodity: string;

    // Balance-family options
    balanceMode: BalanceMode;
    accumulation: Accumulation;
    balanceView: BalanceView;
    showAverage: boolean;
    showRowTotal: boolean;
    summaryOnly: boolean;
    noTotal: boolean;
    sortAmount: boolean;
    percent: boolean;
    invert: boolean;
    transpose: boolean;
    drop: string;

    // Register-family options
    regAccumulation: RegisterAccumulation;
    regAverage: boolean;
    regRelated: boolean;
    regInvert: boolean;

    // Query input
    queryInput: string;
}

export function createDefaultReportConfig(): ReportConfig {
    return {
        command: 'balance',
        beginDate: '',
        endDate: '',
        interval: '',
        statusCleared: false,
        statusPending: false,
        statusUnmarked: false,
        realOnly: false,
        showEmpty: false,
        depth: '',
        valueCost: false,
        valueMarket: false,
        exchangeCommodity: '',
        balanceMode: '',
        accumulation: '',
        balanceView: '',
        showAverage: false,
        showRowTotal: false,
        summaryOnly: false,
        noTotal: false,
        sortAmount: false,
        percent: false,
        invert: false,
        transpose: false,
        drop: '',
        regAccumulation: '',
        regAverage: false,
        regRelated: false,
        regInvert: false,
        queryInput: '',
    };
}

export type PeriodPreset =
    | 'this-month'
    | 'last-month'
    | 'ytd'
    | 'last-12-months';

/** Format a Date as YYYY-MM-DD using LOCAL date parts. Never use toISOString(),
 * which converts to UTC and can shift the day across a timezone boundary. */
function formatLocalDate(d: Date): string {
    const year = d.getFullYear();
    const month = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${year}-${month}-${day}`;
}

/**
 * Compute an hledger begin/end date range for a period preset relative to
 * `today`. hledger's -e (end) is EXCLUSIVE, so `end` is always the day after the
 * last day the range should include. The Date constructor normalizes out-of-range
 * month/day components, which handles year boundaries and leap days for free.
 */
export function computePeriodPresetRange(
    preset: PeriodPreset,
    today: Date,
): { begin: string; end: string } {
    const y = today.getFullYear();
    const m = today.getMonth(); // 0-based
    const d = today.getDate();
    switch (preset) {
        case 'this-month':
            return {
                begin: formatLocalDate(new Date(y, m, 1)),
                end: formatLocalDate(new Date(y, m + 1, 1)),
            };
        case 'last-month':
            return {
                begin: formatLocalDate(new Date(y, m - 1, 1)),
                end: formatLocalDate(new Date(y, m, 1)),
            };
        case 'ytd':
            return {
                begin: formatLocalDate(new Date(y, 0, 1)),
                // Exclusive end = tomorrow, so today's activity is included.
                end: formatLocalDate(new Date(y, m, d + 1)),
            };
        case 'last-12-months':
            return {
                begin: formatLocalDate(new Date(y, m - 11, 1)),
                end: formatLocalDate(new Date(y, m + 1, 1)),
            };
    }
}

/**
 * Whether the Reports tab should auto-run its default report. True only when the
 * session is pristine: nothing has run (no result, no error) and the default has
 * not already auto-run. Session persistence keeps this false on tab re-entry; a
 * ledger open resets the session so the next open auto-runs again.
 */
export function shouldAutoRunDefault(state: {
    result: object | null;
    error: string | null;
    hasAutoRun: boolean;
}): boolean {
    return state.result === null && state.error === null && !state.hasAutoRun;
}

/**
 * Whether `current` would produce a different report than the one that produced
 * the displayed results (`lastRun`). Used to show a "results may be stale" hint.
 * Compares every ReportConfig field structurally. queryInput is compared by its
 * trimmed value because buildReportArgs trims it, so whitespace-only edits do not
 * change the report. Returns false when nothing has run yet.
 */
export function isReportConfigStale(
    current: ReportConfig,
    lastRun: ReportConfig | null,
): boolean {
    if (lastRun === null) return false;
    // Normalize queryInput (buildReportArgs trims it) then compare structurally.
    // Both objects share the ReportConfig key order, so a serialized compare is
    // order-stable.
    const normalize = (c: ReportConfig): ReportConfig => ({
        ...c,
        queryInput: c.queryInput.trim(),
    });
    return (
        JSON.stringify(normalize(current)) !==
        JSON.stringify(normalize(lastRun))
    );
}

export type CannedReportId =
    | 'spending-by-category'
    | 'income-vs-expense'
    | 'net-worth-trend';

/**
 * Build a fully-formed ReportConfig for a one-click canned report, relative to
 * `today`. Each starts from createDefaultReportConfig() so unrelated options are
 * cleared. The report-utils.test.ts suite pins the resulting buildReportArgs.
 */
export function cannedReportConfig(
    id: CannedReportId,
    today: Date,
): ReportConfig {
    const base = createDefaultReportConfig();
    switch (id) {
        case 'spending-by-category': {
            const { begin, end } = computePeriodPresetRange(
                'this-month',
                today,
            );
            return {
                ...base,
                command: 'balance',
                interval: '-M',
                queryInput: 'acct:^Expenses',
                sortAmount: true,
                beginDate: begin,
                endDate: end,
            };
        }
        case 'income-vs-expense': {
            const { begin, end } = computePeriodPresetRange(
                'last-12-months',
                today,
            );
            return {
                ...base,
                command: 'incomestatement',
                interval: '-M',
                beginDate: begin,
                endDate: end,
            };
        }
        case 'net-worth-trend': {
            const { begin, end } = computePeriodPresetRange(
                'last-12-months',
                today,
            );
            return {
                ...base,
                command: 'balancesheet',
                interval: '-M',
                accumulation: '-H',
                depth: '1',
                beginDate: begin,
                endDate: end,
            };
        }
    }
}

/**
 * Verbatim port of the original ReportsTab.buildArgs(). Assembles the hledger
 * CLI arguments (excluding the command itself and the -f journal path, which the
 * backend supplies) from a ReportConfig.
 */
export function buildReportArgs(config: ReportConfig): string[] {
    const args: string[] = [];

    if (config.beginDate.trim()) {
        args.push('-b', config.beginDate.trim());
    }
    if (config.endDate.trim()) {
        args.push('-e', config.endDate.trim());
    }
    if (config.interval) {
        args.push(config.interval);
    }

    // Status filters
    if (config.statusCleared) args.push('-C');
    if (config.statusPending) args.push('-P');
    if (config.statusUnmarked) args.push('-U');
    if (config.realOnly) args.push('-R');
    if (config.showEmpty) args.push('-E');
    if (config.depth.trim()) args.push(`--depth=${config.depth.trim()}`);

    // Valuation
    if (config.valueCost) args.push('-B');
    if (config.valueMarket) args.push('-V');
    if (config.exchangeCommodity.trim()) {
        args.push('-X', config.exchangeCommodity.trim());
    }

    const isBalanceFamily = BALANCE_FAMILY.includes(config.command);
    const isRegisterFamily = REGISTER_FAMILY.includes(config.command);

    if (isBalanceFamily) {
        if (config.balanceMode) args.push(config.balanceMode);
        if (config.accumulation) args.push(config.accumulation);
        if (config.balanceView) args.push(config.balanceView);
        if (config.showAverage) args.push('-A');
        if (config.showRowTotal) args.push('-T');
        if (config.summaryOnly) args.push('--summary-only');
        if (config.noTotal) args.push('-N');
        if (config.sortAmount) args.push('-S');
        if (config.percent) args.push('-%');
        if (config.command === 'balance' && config.invert)
            args.push('--invert');
        if (config.command === 'balance' && config.transpose)
            args.push('--transpose');
        if (config.drop.trim()) args.push(`--drop=${config.drop.trim()}`);
    }

    if (isRegisterFamily) {
        if (config.regAccumulation) args.push(config.regAccumulation);
        if (config.command !== 'aregister' && config.regAverage)
            args.push('-A');
        if (config.command !== 'aregister' && config.regRelated)
            args.push('-r');
        if (config.regInvert) args.push('--invert');
    }

    // Query tokens
    const trimmed = config.queryInput.trim();
    if (trimmed) {
        args.push(...trimmed.split(/\s+/));
    }

    return args;
}
