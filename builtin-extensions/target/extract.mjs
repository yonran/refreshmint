/**
 * Target extractor for Refreshmint.
 *
 * Processes order JSON documents saved by the Target driver and emits
 * one account-journal entry per payment method on each receipt. This
 * lets the transfer detector later link each entry to the corresponding
 * CC / gift-card / EBT account journal entry from the other login.
 *
 * No gl_account is required for this login because the "balance account"
 * for each entry is derived from the payment instrument (e.g.
 * Liabilities:Payment:CC:4261), not from a pre-configured GL mapping.
 */

export async function extract(context) {
    const info = context.documentInfo?.metadata ?? {};
    if (
        info.attachmentType !== 'target-order-json' &&
        info.attachmentType !== 'target-store-order-json'
    ) {
        return [];
    }

    const purchaseDate = info.purchaseDate;
    const orderId = info.targetOrderId ?? '';
    const orderType = info.targetOrderType ?? 'online';
    if (!purchaseDate || !orderId) return [];

    const body = context.json?.metadata?.bodyText ?? '';
    const payments = parsePayments(body);
    if (payments.length === 0) return [];

    // Validate that the payment amounts sum to the grand total (warn if not).
    const grandTotal = parseFloat(info.grandTotal ?? '');
    if (isFinite(grandTotal) && payments.length > 0) {
        const sum = payments.reduce((acc, p) => acc + p.amount, 0);
        if (Math.abs(sum - grandTotal) > 0.005) {
            console.warn(
                `target extract: payment sum ${sum} != grandTotal ${grandTotal} ` +
                    `for order ${orderId}`,
            );
        }
    }

    return payments
        .map(({ label, account, amount }, i) => {
            const aquantity = `-${amount}`;
            // Guard: amount must be a finite negative number.
            if (
                !isFinite(parseFloat(aquantity)) ||
                parseFloat(aquantity) >= 0
            ) {
                return null;
            }
            return {
                tdate: purchaseDate,
                tstatus: 'Cleared',
                tdescription: `Target ${orderType} #${orderId} (${label})`,
                tcomment: '',
                ttags: [
                    ['evidence', `${context.document.name}:pmt${i}`],
                    ['amount', `${aquantity} USD`],
                    ['bankId', `target:${orderType}:${orderId}:pmt${i}`],
                ],
                tpostings: [
                    {
                        paccount: account,
                        pamount: [{ acommodity: 'USD', aquantity }],
                    },
                ],
            };
        })
        .filter(Boolean);
}

/**
 * Parse the "Payment Summary" section of a Target order page's bodyText.
 *
 * Returns an array of { label, account, amount } objects for each
 * financial payment instrument found. Discounts and coupons (negative
 * amounts or unrecognised lines) are skipped.
 *
 * Known payment types:
 *   CC ending in / *XXXX    → Liabilities:Payment:CC:XXXX
 *   Gift Card / RedCard     → Assets:Payment:GiftCard
 *   EBT                     → Assets:Payment:EBT
 *   Cash                    → Assets:Payment:Cash
 */
function parsePayments(bodyText) {
    // Isolate the Payment Summary section (everything after the heading).
    const summaryStart = bodyText.search(/payment\s+summary/i);
    const section = summaryStart >= 0 ? bodyText.slice(summaryStart) : bodyText;

    // Split into lines and walk through them two-at-a-time.
    // Target's bodyText often has the label on one line and the amount on
    // the next, e.g.:
    //   "Target Circle Credit Card *4261\n$15.11"
    // or inline: "Target Circle Credit Card *4261 $15.11"
    const lines = section
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean);

    const results = [];

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];

        // Extract a dollar amount from this line or the next line.
        const amountOnLine = extractPositiveDollarAmount(line);
        const nextLine = lines[i + 1] ?? '';
        const amountOnNext = extractPositiveDollarAmount(nextLine);

        // Determine which amount to use and whether to advance i.
        let amount;
        let advanceExtra = false;
        if (amountOnLine !== null) {
            amount = amountOnLine;
        } else if (amountOnNext !== null) {
            amount = amountOnNext;
            advanceExtra = true;
        } else {
            continue;
        }

        // Classify the payment instrument from the label text.
        const instrument = classifyPaymentLine(line);
        if (instrument === null) continue;

        results.push({
            label: instrument.label,
            account: instrument.account,
            amount,
        });
        if (advanceExtra) i++;
    }

    return results;
}

/**
 * Classify a single label line as a payment instrument.
 * Returns { label, account } or null if not a recognised instrument.
 */
function classifyPaymentLine(line) {
    // Credit / debit card: "ending in 4261", "*4261", "Card *4261"
    const ccMatch = line.match(/(?:\*|ending in\s*)(\d{4})\b/i);
    if (ccMatch) {
        const last4 = ccMatch[1];
        return {
            label: `CC *${last4}`,
            account: `Liabilities:Payment:CC:${last4}`,
        };
    }

    // Gift card / RedCard (non-CC)
    if (/gift\s*card|red\s*card\s+balance/i.test(line)) {
        return { label: 'Gift Card', account: 'Assets:Payment:GiftCard' };
    }

    // EBT / SNAP
    if (/\bebt\b|\bsnap\b/i.test(line)) {
        return { label: 'EBT', account: 'Assets:Payment:EBT' };
    }

    // Cash
    if (/^cash$/i.test(line)) {
        return { label: 'Cash', account: 'Assets:Payment:Cash' };
    }

    return null;
}

/**
 * Extract a positive dollar amount from a string like "$15.11" or "15.11".
 * Returns the numeric value, or null if none found or if the amount is <= 0.
 */
function extractPositiveDollarAmount(text) {
    const match = text.match(/\$?([\d,]+\.\d{2})\b/);
    if (!match) return null;
    const value = parseFloat(match[1].replace(/,/g, ''));
    return isFinite(value) && value > 0 ? value : null;
}
