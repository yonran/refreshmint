import { describe, it, expect } from 'vitest';
import {
    buildReportArgs,
    cannedReportConfig,
    computePeriodPresetRange,
    createDefaultReportConfig,
    shouldAutoRunDefault,
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
