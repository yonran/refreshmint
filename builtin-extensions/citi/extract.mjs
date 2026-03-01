/**
 * Citi extractor for activity CSVs saved by the Citi scraper.
 */

const LIABILITY_LABELS = new Set(['costco_anywhere_visa_card_by_citi_3743']);

export async function extract(context) {
    if (!context.csv || context.csv.length < 2) {
        return [];
    }
    const header = context.csv[0] ?? [];
    if (
        header.length < 5 ||
        header[0] !== 'date' ||
        header[1] !== 'description' ||
        header[2] !== 'amount'
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
    const [date, description, amount, note] = row;
    if (!date || !description || !amount) return null;

    const tdate = parseDate(date);
    if (!tdate) return null;

    const normalizedAmount = normalizeCurrencyAmount(amount);
    if (!normalizedAmount) return null;
    const finalAmount = LIABILITY_LABELS.has(context.label || '')
        ? invertAmount(normalizedAmount)
        : normalizedAmount;

    return {
        tdate,
        tstatus: 'Cleared',
        tdescription: String(description).trim(),
        tcomment: String(note || '').trim(),
        ttags: [
            ['evidence', `${context.document.name}:${index + 2}:1`],
            ['amount', `${finalAmount} USD`],
        ],
    };
}

function parseDate(value) {
    const raw = String(value || '').trim();
    const match = raw.match(/^([A-Z][a-z]{2}) (\d{1,2}), (\d{4})$/);
    if (!match) return null;
    const month = MONTHS[match[1]];
    if (!month) return null;
    return `${match[3]}-${month}-${match[2].padStart(2, '0')}`;
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

function invertAmount(amount) {
    const raw = String(amount || '').trim();
    if (!raw) return '';
    if (raw.startsWith('-')) {
        return raw.slice(1);
    }
    return `-${raw}`;
}

const MONTHS = {
    Jan: '01',
    Feb: '02',
    Mar: '03',
    Apr: '04',
    May: '05',
    Jun: '06',
    Jul: '07',
    Aug: '08',
    Sep: '09',
    Oct: '10',
    Nov: '11',
    Dec: '12',
};
