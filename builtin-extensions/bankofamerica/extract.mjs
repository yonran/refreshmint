/**
 * Bank of America extractor for transaction CSV exports.
 */

export async function extract(context) {
    if (!context.csv || context.csv.length < 2) {
        return [];
    }
    const header = context.csv[0] ?? [];
    if (
        header.length < 5 ||
        header[0] !== 'Posted Date' ||
        header[1] !== 'Reference Number' ||
        header[2] !== 'Payee' ||
        header[4] !== 'Amount'
    ) {
        return [];
    }

    return context.csv
        .slice(1)
        .map((row, i) => extractCsvRow(row, i, context))
        .filter((txn) => txn !== null);
}

function extractCsvRow(row, index, context) {
    if (row.length < 5) return null;
    const [date, reference, payee, address, amount] = row;
    if (!date || !reference || !payee || !amount) return null;

    const tdate = parseDate(date);
    if (!tdate) return null;

    const normalizedAmount = normalizeCurrencyAmount(amount);
    if (!normalizedAmount) return null;

    return {
        tdate,
        tstatus: 'Cleared',
        tdescription: String(payee).trim(),
        tcomment: String(address || '').trim(),
        ttags: [
            ['evidence', `${context.document.name}:${index + 2}:1`],
            ['amount', `${normalizedAmount} USD`],
            ['bankId', String(reference).trim()],
        ],
    };
}

function parseDate(value) {
    const raw = String(value || '').trim();
    const match = raw.match(/^(\d{2})\/(\d{2})\/(\d{4})$/);
    if (!match) return null;
    return `${match[3]}-${match[1]}-${match[2]}`;
}

function normalizeCurrencyAmount(amount) {
    const raw = String(amount || '').trim();
    if (!raw) return '';
    const negative = raw.includes('-') || /^\(.*\)$/.test(raw);
    const unsigned = raw
        .replace(/[()]/g, '')
        .replace(/[$,]/g, '')
        .replace(/-/g, '')
        .trim();
    if (!unsigned) return '';
    return negative ? `-${unsigned}` : unsigned;
}
