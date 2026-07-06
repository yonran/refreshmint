import { describe, expect, it } from 'vitest';
import type { AmountTotal } from './tauri-commands.ts';
import { formatScaled, formatTotal, formatTotals } from './amount-utils.ts';

function total(partial: Partial<AmountTotal>): AmountTotal {
    return {
        commodity: 'USD',
        mantissa: '12345',
        scale: 2,
        style: null,
        ...partial,
    };
}

describe('formatScaled', () => {
    it('inserts a decimal point at the scale position', () => {
        expect(formatScaled('12345', 2)).toBe('123.45');
    });

    it('handles negative mantissas', () => {
        expect(formatScaled('-12345', 2)).toBe('-123.45');
    });

    it('zero-pads mantissas shorter than the scale', () => {
        expect(formatScaled('5', 2)).toBe('0.05');
    });

    it('returns the mantissa unchanged for scale 0', () => {
        expect(formatScaled('12345', 0)).toBe('12345');
    });
});

describe('formatTotal', () => {
    it('formats a scale-2 total with a right-side spaced commodity by default', () => {
        expect(formatTotal(total({ mantissa: '12345', scale: 2 }))).toBe(
            '123.45 USD',
        );
    });

    it('formats a negative total', () => {
        expect(formatTotal(total({ mantissa: '-12345', scale: 2 }))).toBe(
            '-123.45 USD',
        );
    });

    it('honors a left-side unspaced style', () => {
        expect(
            formatTotal(
                total({
                    commodity: '$',
                    mantissa: '12345',
                    scale: 2,
                    style: { side: 'L', spaced: false },
                }),
            ),
        ).toBe('$123.45');
    });

    it('treats a null style as right-side spaced', () => {
        expect(
            formatTotal(total({ commodity: 'USD', mantissa: '5', scale: 2 })),
        ).toBe('0.05 USD');
    });
});

describe('formatTotals', () => {
    it('returns N/A for null', () => {
        expect(formatTotals(null)).toBe('N/A');
    });

    it('returns N/A for an empty array', () => {
        expect(formatTotals([])).toBe('N/A');
    });

    it('joins multiple totals with a comma and space', () => {
        expect(
            formatTotals([
                total({ commodity: 'USD', mantissa: '100', scale: 0 }),
                total({ commodity: 'EUR', mantissa: '250', scale: 0 }),
            ]),
        ).toBe('100 USD, 250 EUR');
    });
});
