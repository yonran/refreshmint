import type { AmountStyleHint, AmountTotal } from './tauri-commands.ts';

// Shared amount-formatting helpers. Previously duplicated verbatim in App.tsx,
// TransactionsTab.tsx, and TransactionsTable.tsx; the TransactionsTable copy
// (which factored out formatScaled) is the canonical version kept here.

export function formatScaled(mantissa: string, scale: number): string {
    let negative = false;
    let digits = mantissa;
    if (digits.startsWith('-')) {
        negative = true;
        digits = digits.slice(1);
    }
    if (scale > 0) {
        const scaleInt = Math.max(scale, 0);
        if (digits.length <= scaleInt) {
            const needed = scaleInt + 1 - digits.length;
            digits = `${'0'.repeat(needed)}${digits}`;
        }
        const split = digits.length - scaleInt;
        const value = `${digits.slice(0, split)}.${digits.slice(split)}`;
        return negative ? `-${value}` : value;
    }
    return negative ? `-${digits}` : digits;
}

export function normalizeStyle(style: AmountStyleHint | null) {
    if (style === null) {
        return { side: 'R' as const, spaced: true };
    }
    return style;
}

export function formatTotal(total: AmountTotal): string {
    const value = formatScaled(total.mantissa, total.scale);
    const { side, spaced } = normalizeStyle(total.style);
    const separator = spaced ? ' ' : '';
    return side === 'L'
        ? `${total.commodity}${separator}${value}`
        : `${value}${separator}${total.commodity}`;
}

export function formatTotals(totals: AmountTotal[] | null): string {
    if (!totals || totals.length === 0) {
        return 'N/A';
    }
    return totals.map(formatTotal).join(', ');
}
