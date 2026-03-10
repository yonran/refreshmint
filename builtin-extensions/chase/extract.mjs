/**
 * Chase extractor for transaction CSV exports.
 */

export async function extract(context) {
    if (!context.csv || context.csv.length < 2) {
        return [];
    }
    const header = context.csv[0] || [];

    // Credit Card CSV: Transaction Date,Post Date,Description,Category,Type,Amount,Memo
    if (header[0] === 'Transaction Date' && header[1] === 'Post Date') {
        return context.csv
            .slice(1)
            .map((row, i) => extractCreditCardRow(row, i, context))
            .filter((txn) => txn !== null);
    }

    // Savings/Checking (DDA) CSV: Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #
    if (header[0] === 'Details' && header[1] === 'Posting Date') {
        return context.csv
            .slice(1)
            .map((row, i) => extractDdaRow(row, i, context))
            .filter((txn) => txn !== null);
    }

    return [];
}

function extractCreditCardRow(row, index, context) {
    if (row.length < 6) return null;
    const [tDate, pDate, description, category, _type, amount, memo] = row;

    const date = parseDate(pDate); // Use Post Date as the primary hledger date
    if (!date) return null;

    const normalizedAmount = normalizeCurrencyAmount(amount);
    if (!normalizedAmount) return null;

    const tags = [
        ['evidence', `${context.document.name}:${index + 2}:1`],
        ['amount', `${normalizedAmount} USD`],
    ];

    if (tDate && tDate !== pDate) {
        const transDate = parseDate(tDate);
        if (transDate) {
            tags.push(['transaction_date', transDate]);
        }
    }

    if (category && category.trim()) {
        tags.push(['category', category.trim()]);
    }

    return {
        tdate: date,
        tstatus: 'Cleared',
        tdescription: description.trim(),
        tcomment: memo ? memo.trim() : '',
        ttags: tags,
    };
}

function extractDdaRow(row, index, context) {
    if (row.length < 6) return null;
    const [details, pDate, description, amount, type, _balance, checkNum] = row;

    const date = parseDate(pDate);
    if (!date) return null;

    const normalizedAmount = normalizeCurrencyAmount(amount);
    if (!normalizedAmount) return null;

    const tags = [
        ['evidence', `${context.document.name}:${index + 2}:1`],
        ['amount', `${normalizedAmount} USD`],
    ];

    if (details && details.trim()) {
        tags.push(['details', details.trim()]);
    }

    if (type && type.trim()) {
        tags.push(['type', type.trim()]);
    }

    return {
        tdate: date,
        tstatus: 'Cleared',
        tdescription: description.trim(),
        tcomment: checkNum ? `Check/Slip #${checkNum.trim()}` : '',
        ttags: tags,
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
