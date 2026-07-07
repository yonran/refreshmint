import { describe, it, expect } from 'vitest';
import {
    buildReportArgs,
    createDefaultReportConfig,
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
