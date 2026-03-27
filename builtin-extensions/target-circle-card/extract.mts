/**
 * Target Circle Card extractor.
 *
 * Bronze-step extraction is intentionally per-document.
 * The later medallion ETL is responsible for combining CSV, OFX, PDF, and other
 * artifacts into richer unified rows.
 *
 * Current extractor support:
 * - CSV activity exports: extract transaction rows with strong evidence and tags
 * - PDF statements: intentionally skipped for now
 * - OFX/QFX/QBO: parse the raw file to JSON and emit per-transaction bronze rows
 */

/* eslint-disable
    @typescript-eslint/no-base-to-string,
    @typescript-eslint/no-unnecessary-condition,
    @typescript-eslint/no-unnecessary-type-conversion,
    @typescript-eslint/no-unsafe-type-assertion,
    @typescript-eslint/strict-boolean-expressions
*/

import { Ofx } from 'ofx-data-extractor';

type ExtractContext = {
    accountName?: string;
    document?: { name?: string };
    documentInfo?: {
        coverageEndDate?: string;
        dateRangeStart?: string;
        dateRangeEnd?: string;
    };
    csv?: string[][];
    file?: File;
    pdf?: unknown;
};

type ParsedQfx = {
    sourceFormat?: string | null;
    accountId?: string | null;
    currency?: string | null;
    dateRangeStart?: string | null;
    dateRangeEnd?: string | null;
    ledgerBalance?: string | null;
    availableBalance?: string | null;
    transactions?: ParsedQfxTransaction[];
};

type ParsedQfxTransaction = {
    id?: string | null;
    transactionType?: string | null;
    postedAt?: string | null;
    userAt?: string | null;
    amount?: string | null;
    name?: string | null;
    memo?: string | null;
    sic?: string | null;
};

type ExtractedRow = {
    tdate: string;
    tstatus: 'Cleared';
    tdescription: string;
    tcomment: string;
    ttags: [string, string][];
};

export async function extract(
    context: ExtractContext,
): Promise<ExtractedRow[]> {
    if (Array.isArray(context.csv) && context.csv.length >= 2) {
        return extractCsv(context);
    }

    if (
        isOfxFamilyDocument(context.document?.name) &&
        context.file instanceof File
    ) {
        return extractOfxFamily(
            context,
            await parseOfxFile(context.file, context.document?.name),
        );
    }

    if (context.pdf) {
        return [];
    }

    return [];
}

async function parseOfxFile(
    file: File,
    documentName?: string,
): Promise<ParsedQfx> {
    const text = decodeOfxBytes(new Uint8Array(await file.arrayBuffer()));
    const ofx = new Ofx(text);
    const parsed = asRecord(ofx.toJson());
    const ofxRoot = getRecord(parsed, 'OFX');
    const creditCardStatement = asRecord(
        getRecord(
            getRecord(getRecord(ofxRoot, 'CREDITCARDMSGSRSV1'), 'CCSTMTTRNRS'),
            'CCSTMTRS',
        ),
    );
    const bankStatement = asRecord(
        getRecord(
            getRecord(getRecord(ofxRoot, 'BANKMSGSRSV1'), 'STMTTRNRS'),
            'STMTRS',
        ),
    );
    const statement = creditCardStatement ?? bankStatement;
    const bankTransactions = getRecord(statement, 'BANKTRANLIST');
    const transactions =
        getRecordArray(bankTransactions, 'STRTTRN') ??
        getRecordArray(bankTransactions, 'STMTTRN') ??
        [];

    return {
        sourceFormat: inferOfxSourceFormat(documentName),
        accountId:
            getString(getRecord(creditCardStatement, 'CCACCTFROM'), 'ACCTID') ??
            getString(getRecord(bankStatement, 'BANKACCTFROM'), 'ACCTID'),
        currency: getString(statement, 'CURDEF'),
        dateRangeStart: getString(bankTransactions, 'DTSTART'),
        dateRangeEnd: getString(bankTransactions, 'DTEND'),
        ledgerBalance: getString(getRecord(statement, 'LEDGERBAL'), 'BALAMT'),
        availableBalance: getString(getRecord(statement, 'AVAILBAL'), 'BALAMT'),
        transactions: transactions.map((transaction) => ({
            id: getString(transaction, 'FITID'),
            transactionType: getString(transaction, 'TRNTYPE'),
            postedAt: getString(transaction, 'DTPOSTED'),
            userAt: getString(transaction, 'DTUSER'),
            amount: getString(transaction, 'TRNAMT'),
            name: getString(transaction, 'NAME'),
            memo: getString(transaction, 'MEMO'),
            sic: getString(transaction, 'SIC'),
        })),
    };
}

function extractOfxFamily(
    context: ExtractContext,
    parsed: ParsedQfx,
): ExtractedRow[] {
    if (!Array.isArray(parsed?.transactions)) {
        return [];
    }

    return parsed.transactions
        .map((transaction, index) =>
            extractOfxTransaction(transaction, index, context, parsed),
        )
        .filter((txn): txn is ExtractedRow => txn !== null);
}

function decodeOfxBytes(bytes: Uint8Array): string {
    const header = new TextDecoder('ascii').decode(
        bytes.subarray(0, Math.min(bytes.length, 256)),
    );
    const charset = header
        .match(/CHARSET:([^\r\n]+)/i)?.[1]
        ?.trim()
        .toLowerCase();
    const encoding = charset === '1252' ? 'windows-1252' : 'utf-8';
    return new TextDecoder(encoding).decode(bytes);
}

function inferOfxSourceFormat(documentName?: string): string {
    const lowerName = String(documentName || '').toLowerCase();
    if (lowerName.endsWith('.qfx')) {
        return 'qfx';
    }
    if (lowerName.endsWith('.qbo')) {
        return 'qbo';
    }
    return 'ofx';
}

function asRecord(value: unknown): Record<string, unknown> | null {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
        ? (value as Record<string, unknown>)
        : null;
}

function getRecord(
    record: Record<string, unknown> | null,
    key: string,
): Record<string, unknown> | null {
    return asRecord(record?.[key]);
}

function getRecordArray(
    record: Record<string, unknown> | null,
    key: string,
): Record<string, unknown>[] | null {
    const value = record?.[key];
    if (!Array.isArray(value)) {
        return null;
    }

    return value.flatMap((item) => {
        const entry = asRecord(item);
        return entry == null ? [] : [entry];
    });
}

function getString(
    record: Record<string, unknown> | null,
    key: string,
): string | null {
    const value = record?.[key];
    return value == null ? null : String(value);
}

function extractCsv(context: ExtractContext): ExtractedRow[] {
    const rows = context.csv;
    if (!Array.isArray(rows) || rows.length < 2) {
        return [];
    }

    const header = (rows[0] || []).map(cleanHeaderCell);
    if (
        header.length < 7 ||
        header[0] !== 'Transaction Date' ||
        header[1] !== 'Posting Date' ||
        header[2] !== 'Ref#' ||
        header[3] !== 'Amount' ||
        header[4] !== 'Description'
    ) {
        return [];
    }

    return rows
        .slice(1)
        .map((row, index) => extractCsvRow(row, index, context))
        .filter((txn): txn is ExtractedRow => txn !== null);
}

function extractCsvRow(
    row: string[],
    index: number,
    context: ExtractContext,
): ExtractedRow | null {
    if (!Array.isArray(row) || row.length < 7) {
        return null;
    }

    const transactionDateRaw = String(row[0] || '').trim();
    const postingDateRaw = String(row[1] || '').trim();
    const reference = String(row[2] || '').trim();
    const amountRaw = String(row[3] || '').trim();
    const descriptionRaw = String(row[4] || '').trim();
    const last4 = String(row[5] || '').trim();
    const transactionType = String(row[6] || '').trim();

    if (!postingDateRaw || !amountRaw || !descriptionRaw) {
        return null;
    }

    const postingDate = parseIsoDate(
        transactionDateRaw === '' ? postingDateRaw : postingDateRaw,
    );
    const transactionDate = parseIsoDate(transactionDateRaw);
    if (postingDate == null) {
        return null;
    }

    const normalizedAmount = normalizeCurrencyAmount(amountRaw);
    if (normalizedAmount === '') {
        return null;
    }
    const finalAmount = shouldInvertAmounts(context)
        ? invertAmount(normalizedAmount)
        : normalizedAmount;

    const description = normalizeWhitespace(descriptionRaw);
    const commentParts: string[] = [];
    if (transactionType !== '') {
        commentParts.push(`type=${transactionType}`);
    }
    if (last4 !== '') {
        commentParts.push(`last4=${last4}`);
    }
    if (transactionDate != null && transactionDate !== postingDate) {
        commentParts.push(`transactionDate=${transactionDate}`);
    }

    const documentName = String(context.document?.name || '');
    const tags: [string, string][] = [
        ['evidence', `${documentName}:${index + 2}:1`],
        ['amount', `${finalAmount} USD`],
        ['sourceFormat', 'csv'],
    ];

    if (reference !== '') {
        tags.push(['bankId', reference]);
        tags.push(['reference', reference]);
    }
    if (transactionDate != null) {
        tags.push(['transactionDate', transactionDate]);
    }
    tags.push(['postingDate', postingDate]);
    if (transactionType !== '') {
        tags.push(['transactionType', transactionType]);
    }
    if (last4 !== '') {
        tags.push(['cardLast4', last4]);
    }
    if (context.documentInfo?.coverageEndDate) {
        tags.push([
            'coverageEndDate',
            String(context.documentInfo.coverageEndDate),
        ]);
    }
    if (context.documentInfo?.dateRangeStart) {
        tags.push([
            'dateRangeStart',
            String(context.documentInfo.dateRangeStart),
        ]);
    }
    if (context.documentInfo?.dateRangeEnd) {
        tags.push(['dateRangeEnd', String(context.documentInfo.dateRangeEnd)]);
    }

    return {
        tdate: postingDate,
        tstatus: 'Cleared',
        tdescription: description,
        tcomment: commentParts.join(' '),
        ttags: tags,
    };
}

function extractOfxTransaction(
    transaction: ParsedQfxTransaction,
    index: number,
    context: ExtractContext,
    parsed: ParsedQfx,
): ExtractedRow | null {
    const postingDate = parseIsoDate(transaction?.postedAt);
    if (postingDate == null) {
        return null;
    }

    const normalizedAmount = normalizeCurrencyAmount(transaction?.amount);
    if (normalizedAmount === '') {
        return null;
    }
    const finalAmount = shouldInvertAmounts(context)
        ? invertAmount(normalizedAmount)
        : normalizedAmount;

    const transactionDate = parseIsoDate(transaction?.userAt);
    const description = normalizeWhitespace(
        [transaction?.name, transaction?.memo].filter(Boolean).join(' '),
    );
    if (description === '') {
        return null;
    }

    const transactionType = String(transaction?.transactionType || '').trim();
    const fitId = String(transaction?.id || '').trim();
    const sic = String(transaction?.sic || '').trim();

    const commentParts: string[] = [];
    if (transactionType !== '') {
        commentParts.push(`type=${transactionType}`);
    }
    if (sic !== '') {
        commentParts.push(`sic=${sic}`);
    }
    if (transactionDate != null && transactionDate !== postingDate) {
        commentParts.push(`transactionDate=${transactionDate}`);
    }

    const documentName = String(context.document?.name || '');
    const tags: [string, string][] = [
        ['evidence', `${documentName}:${index + 1}:1`],
        ['amount', `${finalAmount} USD`],
        ['sourceFormat', String(parsed?.sourceFormat || 'ofx')],
        ['postingDate', postingDate],
    ];

    if (fitId !== '') {
        tags.push(['bankId', fitId]);
        tags.push(['fitId', fitId]);
    }
    if (transactionDate != null) {
        tags.push(['transactionDate', transactionDate]);
    }
    if (transactionType !== '') {
        tags.push(['transactionType', transactionType]);
    }
    if (sic !== '') {
        tags.push(['sic', sic]);
    }
    if (parsed?.accountId) {
        tags.push(['accountId', String(parsed.accountId)]);
    }
    if (parsed?.currency) {
        tags.push(['currency', String(parsed.currency)]);
    }
    if (parsed?.dateRangeStart) {
        tags.push(['dateRangeStart', String(parsed.dateRangeStart)]);
    }
    if (parsed?.dateRangeEnd) {
        tags.push(['dateRangeEnd', String(parsed.dateRangeEnd)]);
    }
    if (parsed?.ledgerBalance) {
        tags.push(['ledgerBalance', `${parsed.ledgerBalance} USD`]);
    }
    if (parsed?.availableBalance) {
        tags.push(['availableBalance', `${parsed.availableBalance} USD`]);
    }
    if (context.documentInfo?.coverageEndDate) {
        tags.push([
            'coverageEndDate',
            String(context.documentInfo.coverageEndDate),
        ]);
    }
    if (context.documentInfo?.dateRangeStart) {
        tags.push([
            'sidecarDateRangeStart',
            String(context.documentInfo.dateRangeStart),
        ]);
    }
    if (context.documentInfo?.dateRangeEnd) {
        tags.push([
            'sidecarDateRangeEnd',
            String(context.documentInfo.dateRangeEnd),
        ]);
    }

    return {
        tdate: postingDate,
        tstatus: 'Cleared',
        tdescription: description,
        tcomment: commentParts.join(' '),
        ttags: tags,
    };
}

function isOfxFamilyDocument(documentName?: string): boolean {
    const lower = String(documentName || '')
        .trim()
        .toLowerCase();
    return (
        lower.endsWith('.ofx') ||
        lower.endsWith('.qfx') ||
        lower.endsWith('.qbo')
    );
}

function cleanHeaderCell(value: string | undefined): string {
    return String(value || '')
        .replace(/^\uFEFF/, '')
        .trim();
}

function parseIsoDate(value: string | null | undefined): string | null {
    const raw = String(value || '').trim();
    const match = raw.match(/^(\d{4})-(\d{2})-(\d{2})$/);
    if (!match) {
        return null;
    }
    return `${match[1]}-${match[2]}-${match[3]}`;
}

function normalizeWhitespace(value: string | null | undefined): string {
    return String(value || '')
        .replace(/\s+/g, ' ')
        .trim();
}

function normalizeCurrencyAmount(amount: string | null | undefined): string {
    const raw = String(amount || '').trim();
    if (raw === '') {
        return '';
    }
    const negative = raw.includes('-') || /^\(.*\)$/.test(raw);
    const unsigned = raw
        .replace(/[()]/g, '')
        .replace(/[$,]/g, '')
        .replace(/-/g, '')
        .trim();
    if (unsigned === '') {
        return '';
    }
    return negative ? `-${unsigned}` : unsigned;
}

function invertAmount(amount: string): string {
    const raw = String(amount || '').trim();
    if (raw === '') {
        return '';
    }
    if (raw.startsWith('-')) {
        return raw.slice(1);
    }
    return `-${raw}`;
}

function shouldInvertAmounts(context: ExtractContext): boolean {
    const accountName = String(context?.accountName || '').trim();
    return accountName.startsWith('Liabilities:');
}
