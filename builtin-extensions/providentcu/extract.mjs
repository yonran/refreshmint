/**
 * Provident Credit Union extractor for Refreshmint.
 * Handles CSV exports (activity) and PDF statements (credit cards).
 */

export async function extract(context) {
    if (context.csv) {
        return extractCsv(context);
    }
    if (context.pdf) {
        return extractPdf(context);
    }
    return [];
}

async function extractCsv(context) {
    if (!context.csv || context.csv.length < 2) {
        return [];
    }

    // Header: "Date","Description","Comments","Check Number","Amount","Balance"
    // Example: "02/18/2026","UI BENEFIT WA ST EMPLOY SEC ID2911762161","","","$1,037.00","$54,265.70"

    return context.csv
        .slice(1)
        .map((row, i) => {
            if (row.length < 5) return null;

            const [date, description, comments, checkNumber, amount, _balance] =
                row;

            if (!date || !amount) return null;

            const tdate = parseDate(date);
            if (!tdate) return null;

            // Clean up amount: "$1,037.00" -> "1037.00", "-$50.00" -> "-50.00"
            const tamount = amount.replace(/[$,]/g, '').trim();

            // Build description
            let tdescription = description.trim();
            // Remove "(Pending)" suffix if present
            tdescription = tdescription.replace(/\s*\(Pending\)$/i, '');

            if (checkNumber && checkNumber.trim()) {
                tdescription += ` (Check #${checkNumber.trim()})`;
            }

            const ttags = [
                ['evidence', `${context.document.name}:${i + 2}:1`],
                ['amount', `${tamount} USD`],
            ];

            const tcomment = comments ? comments.trim() : '';

            return {
                tdate,
                tstatus: 'Cleared',
                tdescription,
                tcomment,
                ttags,
            };
        })
        .filter((txn) => txn !== null);
}

async function extractPdf(context) {
    const transactions = [];

    // Credit card statement logic
    let year = null;
    let closingDate = null;

    // Try to find the statement year from "Statement Closing Date"
    // Statement Closing Date                              02/06/2026
    for (const page of context.pdf.pages) {
        const match = page.text.match(
            /Statement Closing Date\s+(\d{2}\/\d{2}\/(\d{4}))/,
        );
        if (match) {
            closingDate = parseDate(match[1]);
            year = match[2];
            break;
        }
    }

    if (!year) {
        // Fallback to year from filename if possible
        const fileYearMatch =
            context.document.name.match(/(\d{4})-\d{2}-\d{2}/);
        if (fileYearMatch) {
            year = fileYearMatch[1];
        } else {
            year = new Date().getFullYear().toString();
        }
    }

    for (const page of context.pdf.pages) {
        let inTransactionsSection = false;

        // In the PDF text context, we get items or page.text split by lines
        const lines = page.text.split('\n');

        for (let i = 0; i < lines.length; i++) {
            const line = lines[i];

            if (
                line.match(/Transactions\b/i) ||
                line.match(/Transactions \(continued\)/i)
            ) {
                inTransactionsSection = true;
                continue;
            }
            if (
                line.match(/Fees\b/i) ||
                line.match(/Interest Charged\b/i) ||
                line.match(/Interest Charge Calculation\b/i) ||
                line.match(/REBATE REWARDS ACTIVITY/i)
            ) {
                inTransactionsSection = false;
                continue;
            }

            if (!inTransactionsSection) continue;

            // Row format: Post Date  Trans Date  Reference  Description  Amount
            // Example: 01/11  01/09  2423168QSHRNYPXD1  SAFEWAY #1965 SEATTLE WA  $27.53
            // Note: Amount can be negative -$921.07

            // Regex to match: MM/DD MM/DD REFERENCE DESCRIPTION AMOUNT
            // Reference is usually 17-23 chars alphanumeric
            // Description can have spaces
            // Amount starts with $ or -$ or just digits
            const txnMatch = line.match(
                /^\s*(\d{2}\/\d{2})\s+(\d{2}\/\d{2})\s+([A-Z0-9]{10,})\s+(.+?)\s+(-?\$?[\d,]+\.\d{2})\s*$/,
            );

            if (txnMatch) {
                const [
                    _,
                    _postDate,
                    transDate,
                    reference,
                    description,
                    amount,
                ] = txnMatch;

                // Use transDate if available, otherwise postDate
                const [m, d] = transDate.split('/');

                // Handle year wrap-around
                let txnYear = year;
                if (closingDate) {
                    const [stmtYear, stmtMonth] = closingDate
                        .split('-')
                        .map(Number);
                    if (stmtMonth < 3 && Number(m) > 10) {
                        txnYear = (stmtYear - 1).toString();
                    }
                }

                const tdate = `${txnYear}-${m}-${d}`;
                const tamount = amount.replace(/[$,]/g, '').trim();

                transactions.push({
                    tdate,
                    tstatus: 'Cleared',
                    tdescription: description.trim(),
                    tcomment: `Ref: ${reference}`,
                    ttags: [
                        [
                            'evidence',
                            `${context.document.name}#page=${page.pageNumber}`,
                        ],
                        ['amount', `${tamount} USD`],
                        ['bankId', reference],
                    ],
                });
            }
        }
    }

    return transactions;
}

function parseDate(dateStr) {
    if (!dateStr) return null;
    const parts = dateStr.split('/');
    if (parts.length === 3) {
        const [m, d, y] = parts;
        return `${y}-${m.padStart(2, '0')}-${d.padStart(2, '0')}`;
    }
    return null;
}
