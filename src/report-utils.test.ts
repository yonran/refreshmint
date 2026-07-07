import { describe, it, expect } from 'vitest';
import {
    buildReportArgs,
    buildUnknownJumpSearch,
    buildUnknownRegisterArgs,
    cannedReportConfig,
    computePeriodPresetRange,
    createDefaultReportConfig,
    defaultAutoRunConfig,
    isReportConfigStale,
    shouldAutoRunDefault,
    shouldIncludeBudget,
    summarizeUnknownRegister,
    type ReportConfig,
} from './report-utils.ts';

function config(overrides: Partial<ReportConfig> = {}): ReportConfig {
    return { ...createDefaultReportConfig(), ...overrides };
}

describe('buildReportArgs', () => {
    it('default config produces no args', () => {
        expect(buildReportArgs(createDefaultReportConfig())).toEqual([]);
    });

    it('emits -b/-e for begin and end dates (trimmed)', () => {
        expect(
            buildReportArgs(
                config({ beginDate: ' 2024-01-01 ', endDate: '2024-12-31' }),
            ),
        ).toEqual(['-b', '2024-01-01', '-e', '2024-12-31']);
    });

    it('emits the interval flag', () => {
        expect(buildReportArgs(config({ interval: '-M' }))).toEqual(['-M']);
    });

    it('suppresses balance-only flags for register commands', () => {
        // sortAmount/invert/transpose are balance-family only.
        const args = buildReportArgs(
            config({
                command: 'register',
                sortAmount: true,
                invert: true,
                transpose: true,
            }),
        );
        expect(args).not.toContain('-S');
        expect(args).not.toContain('--invert');
        expect(args).not.toContain('--transpose');
    });

    it('suppresses register-only flags for balance commands', () => {
        // regInvert/regRelated are register-family only.
        const args = buildReportArgs(
            config({
                command: 'balance',
                regInvert: true,
                regRelated: true,
                regAccumulation: '-H',
            }),
        );
        expect(args).not.toContain('--invert');
        expect(args).not.toContain('-r');
        // -H here would only come from regAccumulation, which is register-only.
        expect(args).not.toContain('-H');
    });

    it('emits --invert/--transpose only for the plain balance command', () => {
        expect(
            buildReportArgs(config({ command: 'balance', invert: true })),
        ).toContain('--invert');
        expect(
            buildReportArgs(config({ command: 'balance', transpose: true })),
        ).toContain('--transpose');
        // balancesheet is balance-family but not the plain `balance` command.
        expect(
            buildReportArgs(
                config({
                    command: 'balancesheet',
                    invert: true,
                    transpose: true,
                }),
            ),
        ).not.toContain('--invert');
    });

    it('splits query input on whitespace', () => {
        expect(
            buildReportArgs(
                config({ queryInput: '  acct:^Expenses   desc:amazon ' }),
            ),
        ).toEqual(['acct:^Expenses', 'desc:amazon']);
    });

    it('emits --depth=N and --drop=N', () => {
        expect(buildReportArgs(config({ depth: '2' }))).toContain('--depth=2');
        expect(
            buildReportArgs(config({ command: 'balance', drop: '1' })),
        ).toContain('--drop=1');
    });

    it('emits --budget only for balance-family commands', () => {
        expect(
            buildReportArgs(config({ command: 'balance', budget: true })),
        ).toContain('--budget');
        expect(
            buildReportArgs(
                config({ command: 'incomestatement', budget: true }),
            ),
        ).toContain('--budget');
        // Register family ignores budget.
        expect(
            buildReportArgs(config({ command: 'register', budget: true })),
        ).not.toContain('--budget');
        // Off by default.
        expect(buildReportArgs(config({ command: 'balance' }))).not.toContain(
            '--budget',
        );
    });
});

describe('computePeriodPresetRange', () => {
    // `today` is built from local date parts (new Date(y, m, d)) so the assertions
    // are timezone-independent — matching computePeriodPresetRange's local
    // formatting. hledger's -e is exclusive, so end is always the day AFTER the
    // last day in the range.

    it('this-month uses first-of-month .. first-of-next-month', () => {
        expect(
            computePeriodPresetRange('this-month', new Date(2026, 6, 7)),
        ).toEqual({ begin: '2026-07-01', end: '2026-08-01' });
    });

    it('this-month exclusive end rolls into next year in December', () => {
        expect(
            computePeriodPresetRange('this-month', new Date(2026, 11, 5)),
        ).toEqual({ begin: '2026-12-01', end: '2027-01-01' });
    });

    it('last-month across a year boundary (Jan → prior Dec)', () => {
        expect(
            computePeriodPresetRange('last-month', new Date(2026, 0, 15)),
        ).toEqual({ begin: '2025-12-01', end: '2026-01-01' });
    });

    it('ytd in December ends tomorrow', () => {
        expect(computePeriodPresetRange('ytd', new Date(2026, 11, 15))).toEqual(
            {
                begin: '2026-01-01',
                end: '2026-12-16',
            },
        );
    });

    it('ytd tomorrow rolls over the leap day', () => {
        // 2024-02-29 + 1 day = 2024-03-01 (2024 is a leap year).
        expect(computePeriodPresetRange('ytd', new Date(2024, 1, 29))).toEqual({
            begin: '2024-01-01',
            end: '2024-03-01',
        });
    });

    it('last-12-months spans years (11 months back .. next month)', () => {
        expect(
            computePeriodPresetRange('last-12-months', new Date(2024, 1, 15)),
        ).toEqual({ begin: '2023-03-01', end: '2024-03-01' });
    });
});

describe('cannedReportConfig', () => {
    // Fixed today = 2026-07-07 (local). this-month = 2026-07-01..2026-08-01;
    // last-12-months = 2025-08-01..2026-08-01.
    const today = new Date(2026, 6, 7);

    it('spending-by-category: monthly Expenses balance, this month, sorted', () => {
        const cfg = cannedReportConfig('spending-by-category', today);
        expect(cfg.command).toBe('balance');
        expect(cfg.interval).toBe('-M');
        expect(cfg.queryInput).toBe('acct:^Expenses');
        expect(cfg.sortAmount).toBe(true);
        expect({ begin: cfg.beginDate, end: cfg.endDate }).toEqual({
            begin: '2026-07-01',
            end: '2026-08-01',
        });
        expect(buildReportArgs(cfg)).toEqual([
            '-b',
            '2026-07-01',
            '-e',
            '2026-08-01',
            '-M',
            '-S',
            'acct:^Expenses',
        ]);
    });

    it('income-vs-expense: monthly income statement over last 12 months', () => {
        const cfg = cannedReportConfig('income-vs-expense', today);
        expect(cfg.command).toBe('incomestatement');
        expect(cfg.interval).toBe('-M');
        expect(buildReportArgs(cfg)).toEqual([
            '-b',
            '2025-08-01',
            '-e',
            '2026-08-01',
            '-M',
        ]);
    });

    it('net-worth-trend: monthly historical balance sheet, depth 1', () => {
        const cfg = cannedReportConfig('net-worth-trend', today);
        expect(cfg.command).toBe('balancesheet');
        expect(cfg.interval).toBe('-M');
        expect(cfg.accumulation).toBe('-H');
        expect(cfg.depth).toBe('1');
        expect(buildReportArgs(cfg)).toEqual([
            '-b',
            '2025-08-01',
            '-e',
            '2026-08-01',
            '-M',
            '--depth=1',
            '-H',
        ]);
    });

    it('budget: monthly Expenses balance vs. budget, this month', () => {
        const cfg = cannedReportConfig('budget', today);
        expect(cfg.command).toBe('balance');
        expect(cfg.budget).toBe(true);
        expect(cfg.interval).toBe('-M');
        expect(cfg.queryInput).toBe('acct:^Expenses');
        expect(buildReportArgs(cfg)).toEqual([
            '-b',
            '2026-07-01',
            '-e',
            '2026-08-01',
            '-M',
            '--budget',
            'acct:^Expenses',
        ]);
    });
});

describe('defaultAutoRunConfig', () => {
    // Fixed today = 2026-07-07 (local). last-12-months = 2025-08-01..2026-08-01.
    const today = new Date(2026, 6, 7);

    it('is the spending-by-category report widened to the last 12 months', () => {
        const cfg = defaultAutoRunConfig(today);
        const canned = cannedReportConfig('spending-by-category', today);
        const range = computePeriodPresetRange('last-12-months', today);
        // Same command/query/interval/sort as the Quick-report button, but the
        // wider auto-run window so a ledger whose data ended months ago is not
        // greeted with an empty table.
        expect(cfg.command).toBe('balance');
        expect(cfg.interval).toBe('-M');
        expect(cfg.queryInput).toBe('acct:^Expenses');
        expect(cfg.sortAmount).toBe(true);
        expect({ begin: cfg.beginDate, end: cfg.endDate }).toEqual(range);
        // The only difference from the canned spending-by-category config is the
        // date range.
        expect({ ...cfg, beginDate: '', endDate: '' }).toEqual({
            ...canned,
            beginDate: '',
            endDate: '',
        });
    });
});

describe('shouldAutoRunDefault', () => {
    const someResult = { rows: [['account', 'balance']], text: null };

    it('is true when nothing has run yet', () => {
        expect(
            shouldAutoRunDefault({
                result: null,
                error: null,
                hasAutoRun: false,
            }),
        ).toBe(true);
    });

    it('is false once the default has auto-run', () => {
        expect(
            shouldAutoRunDefault({
                result: null,
                error: null,
                hasAutoRun: true,
            }),
        ).toBe(false);
    });

    it('is false when a result already exists', () => {
        expect(
            shouldAutoRunDefault({
                result: someResult,
                error: null,
                hasAutoRun: false,
            }),
        ).toBe(false);
    });

    it('is false when an error already exists', () => {
        expect(
            shouldAutoRunDefault({
                result: null,
                error: 'boom',
                hasAutoRun: false,
            }),
        ).toBe(false);
    });
});

describe('shouldIncludeBudget', () => {
    it('is true only for a balance-family command with budget enabled', () => {
        expect(
            shouldIncludeBudget(config({ command: 'balance', budget: true })),
        ).toBe(true);
    });

    it('is false for a register command even with budget enabled', () => {
        // A persisted budget checkbox must not gate a Transaction Register run
        // with "No budget.journal found".
        expect(
            shouldIncludeBudget(config({ command: 'register', budget: true })),
        ).toBe(false);
    });

    it('is false for a balance command with budget disabled', () => {
        expect(
            shouldIncludeBudget(config({ command: 'balance', budget: false })),
        ).toBe(false);
    });
});

describe('isReportConfigStale', () => {
    it('is false when nothing has run yet (null lastRun)', () => {
        expect(isReportConfigStale(config(), null)).toBe(false);
    });

    it('is false when the config is unchanged since the last run', () => {
        const cfg = config({ command: 'balance', interval: '-M' });
        expect(isReportConfigStale(cfg, { ...cfg })).toBe(false);
    });

    it('is true when the command changed', () => {
        expect(
            isReportConfigStale(
                config({ command: 'register' }),
                config({ command: 'balance' }),
            ),
        ).toBe(true);
    });

    it('is true when a single option field changed', () => {
        expect(
            isReportConfigStale(
                config({ interval: '-M' }),
                config({ interval: '' }),
            ),
        ).toBe(true);
    });

    it('ignores query whitespace-only differences (matches buildReportArgs trim)', () => {
        expect(
            isReportConfigStale(
                config({ queryInput: '  acct:^Expenses  ' }),
                config({ queryInput: 'acct:^Expenses' }),
            ),
        ).toBe(false);
    });
});

describe('buildUnknownRegisterArgs', () => {
    // hledger 1.52 register CSV header (verified): txnidx,date,code,description,
    // account,amount,total.
    it('includes dates when present and the Expenses:Unknown account filter', () => {
        expect(buildUnknownRegisterArgs('2026-07-01', '2026-08-01')).toEqual([
            '-b',
            '2026-07-01',
            '-e',
            '2026-08-01',
            'acct:^Expenses:Unknown(:|$)',
        ]);
    });

    it('omits blank dates', () => {
        expect(buildUnknownRegisterArgs('', '')).toEqual([
            'acct:^Expenses:Unknown(:|$)',
        ]);
    });
});

describe('buildUnknownJumpSearch', () => {
    it('builds an acct + date-range transactions search', () => {
        expect(buildUnknownJumpSearch('2026-07-01', '2026-08-01')).toBe(
            'acct:Expenses:Unknown date:2026-07-01..2026-08-01',
        );
    });

    it('omits the date clause when both dates are blank', () => {
        expect(buildUnknownJumpSearch('', '')).toBe('acct:Expenses:Unknown');
    });
});

describe('summarizeUnknownRegister', () => {
    const header = [
        'txnidx',
        'date',
        'code',
        'description',
        'account',
        'amount',
        'total',
    ];

    it('returns null when there are no data rows', () => {
        expect(summarizeUnknownRegister([])).toBeNull();
        expect(summarizeUnknownRegister([header])).toBeNull();
    });

    it('counts a multi-posting transaction once', () => {
        const rows = [
            header,
            ['2', '2024-01-20', '', 'Split', 'Expenses:Unknown', '$5.00', ''],
            ['2', '2024-01-20', '', 'Split', 'Expenses:Unknown', '$3.00', ''],
        ];
        expect(summarizeUnknownRegister(rows)).toEqual({
            txnCount: 1,
            total: '8.00',
        });
    });

    it('sums amounts across transactions, stripping $ and commas', () => {
        const rows = [
            header,
            ['1', '2024-01-15', '', 'A', 'Expenses:Unknown', '$1,234.56', ''],
            ['2', '2024-01-20', '', 'B', 'Expenses:Unknown', '$-5.00', ''],
        ];
        expect(summarizeUnknownRegister(rows)).toEqual({
            txnCount: 2,
            total: '1229.56',
        });
    });

    it('falls back to count-only when an amount is unparseable', () => {
        const rows = [
            header,
            ['1', '2024-01-15', '', 'A', 'Expenses:Unknown', '5 AAPL', ''],
        ];
        expect(summarizeUnknownRegister(rows)).toEqual({
            txnCount: 1,
            total: '',
        });
    });
});
