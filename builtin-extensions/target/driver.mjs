/**
 * target scraper for Refreshmint.
 */

import { inspect } from 'refreshmint:util';

const TARGET_ORIGIN = 'https://www.target.com/';
const TARGET_LOGIN_URL = 'https://www.target.com/login';
const TARGET_ORDERS_URL = 'https://www.target.com/orders';

const SOURCE_KIND_ORDER_JSON = 'order-json';
const SOURCE_KIND_RECEIPT_IMAGE = 'receipt-image';

/**
 * @typedef {object} ScrapeContext
 * @property {PageApi} mainPage
 * @property {number} currentStep
 * @property {string[]} progressNames
 * @property {Set<string>} progressNamesSet
 * @property {number} lastProgressStep
 */

async function waitMs(page, ms) {
    await page.evaluate(`new Promise((resolve) => setTimeout(resolve, ${ms}))`);
}

async function humanPace(page, minMs, maxMs) {
    const delta = maxMs - minMs;
    const ms = minMs + Math.floor(Math.random() * (delta + 1));
    await waitMs(page, ms);
}

/**
 * Capture incremental DOM changes so debug exec logs show the current state.
 *
 * @param {PageApi} page
 * @param {string} label
 */
async function logStateSnapshot(page, label) {
    const snapshot = await page.snapshot({
        incremental: true,
        track: 'state-loop',
    });
    refreshmint.log(`${label}: ${snapshot}`);
}

/**
 * @param {PageApi} page
 * @returns {Promise<boolean>}
 */
async function hasSelector(page, selector) {
    const result = await page.evaluate(`(function() {
        return document.querySelector(${JSON.stringify(selector)}) != null;
    })()`);
    return result === true;
}

function sanitizeFilenameSegment(value) {
    return String(value || '')
        .trim()
        .replace(/[^A-Za-z0-9._-]+/g, '-')
        .replace(/-+/g, '-')
        .replace(/^-|-$/g, '')
        .toLowerCase();
}

function normalizeCurrencyAmount(value) {
    const raw = String(value || '').trim();
    if (raw === '') {
        return null;
    }
    const negative = raw.includes('-') || /^\(.*\)$/.test(raw);
    const unsigned = raw
        .replace(/[()]/g, '')
        .replace(/[$,]/g, '')
        .replace(/-/g, '')
        .trim();
    if (unsigned === '') {
        return null;
    }
    return negative ? `-${unsigned}` : unsigned;
}

function monthNameToNumber(name) {
    const key = String(name || '')
        .slice(0, 3)
        .toLowerCase();
    switch (key) {
        case 'jan':
            return '01';
        case 'feb':
            return '02';
        case 'mar':
            return '03';
        case 'apr':
            return '04';
        case 'may':
            return '05';
        case 'jun':
            return '06';
        case 'jul':
            return '07';
        case 'aug':
            return '08';
        case 'sep':
            return '09';
        case 'oct':
            return '10';
        case 'nov':
            return '11';
        case 'dec':
            return '12';
        default:
            return null;
    }
}

function parseDateToIso(value) {
    const raw = String(value || '').trim();
    if (raw === '') {
        return null;
    }

    let match = raw.match(/^(\d{4})-(\d{2})-(\d{2})/);
    if (match) {
        return `${match[1]}-${match[2]}-${match[3]}`;
    }

    match = raw.match(/^(\d{2})\/(\d{2})\/(\d{4})$/);
    if (match) {
        return `${match[3]}-${match[1]}-${match[2]}`;
    }

    match = raw.match(/^([A-Za-z]+)\s+(\d{1,2}),\s*(\d{4})$/);
    if (match) {
        const month = monthNameToNumber(match[1]);
        if (month != null) {
            return `${match[3]}-${month}-${String(match[2]).padStart(2, '0')}`;
        }
    }

    return null;
}

function base64ToBytes(base64) {
    const alphabet =
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const clean = String(base64 || '').replace(/[^A-Za-z0-9+/=]/g, '');
    /** @type {number[]} */
    const out = [];
    let buffer = 0;
    let bits = 0;
    for (let i = 0; i < clean.length; i++) {
        const ch = clean[i];
        if (ch === '=') {
            break;
        }
        const val = alphabet.indexOf(ch);
        if (val < 0) {
            continue;
        }
        buffer = (buffer << 6) | val;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push((buffer >> bits) & 0xff);
        }
    }
    return out;
}

function decodeDataUrl(dataUrl) {
    const match = String(dataUrl || '').match(/^data:([^;,]+)?;base64,(.+)$/i);
    if (!match) {
        return null;
    }
    return {
        mimeType: match[1] || 'application/octet-stream',
        bytes: base64ToBytes(match[2]),
    };
}

function sourceItemKey(attachmentKey, sourceKind, attachmentPart) {
    return `${attachmentKey}|${sourceKind}|${attachmentPart || 'single'}`;
}

async function listExistingTargetDocuments() {
    const docsJson = await refreshmint.listAccountDocuments({
        extensionName: 'target',
    });
    const docs = JSON.parse(docsJson || '[]');
    return Array.isArray(docs) ? docs : [];
}

function buildExistingDocumentState(docs) {
    const savedKeys = new Set();
    for (const doc of docs) {
        if (doc == null || typeof doc !== 'object') {
            continue;
        }
        const metadata =
            doc.metadata != null && typeof doc.metadata === 'object'
                ? doc.metadata
                : null;
        if (metadata == null) {
            continue;
        }
        const attachmentKey =
            typeof metadata.attachmentKey === 'string'
                ? metadata.attachmentKey
                : null;
        const sourceKind =
            typeof metadata.sourceKind === 'string'
                ? metadata.sourceKind
                : null;
        const attachmentPart =
            typeof metadata.attachmentPart === 'string'
                ? metadata.attachmentPart
                : 'single';
        if (attachmentKey == null || sourceKind == null) {
            continue;
        }
        savedKeys.add(sourceItemKey(attachmentKey, sourceKind, attachmentPart));
    }
    return { savedKeys };
}

function makeTargetAttachmentKey(orderType, id) {
    return orderType === 'online'
        ? `target:online:${id}`
        : `target:store:${id}`;
}

function hasSavedSource(
    existingState,
    attachmentKey,
    sourceKind,
    attachmentPart,
) {
    return existingState.savedKeys.has(
        sourceItemKey(attachmentKey, sourceKind, attachmentPart),
    );
}

function markSavedSource(
    existingState,
    attachmentKey,
    sourceKind,
    attachmentPart,
) {
    existingState.savedKeys.add(
        sourceItemKey(attachmentKey, sourceKind, attachmentPart),
    );
}

async function extractOrderPageMetadata(page) {
    const raw = await page.evaluate(`(function() {
        const bodyText = (document.body && document.body.innerText) || '';
        const title = document.title || '';
        const url = window.location.href || '';

        const dataUrls = [];
        const seenUrls = new Set();
        const els = Array.from(
            document.querySelectorAll('img[src], a[href], source[src]')
        );
        for (const el of els) {
            const rawValue =
                el.getAttribute('src') ||
                el.getAttribute('href') ||
                '';
            if (!/^data:image\\/gif;base64,/i.test(rawValue)) {
                continue;
            }
            if (seenUrls.has(rawValue)) {
                continue;
            }
            seenUrls.add(rawValue);
            const contextText = [
                el.getAttribute('alt') || '',
                el.getAttribute('title') || '',
                el.getAttribute('aria-label') || '',
                el.textContent || '',
            ]
                .join(' ')
                .toLowerCase();
            let part = 'receipt';
            if (/front/.test(contextText)) {
                part = 'front';
            } else if (/back/.test(contextText)) {
                part = 'back';
            }
            dataUrls.push({ dataUrl: rawValue, part });
        }

        const findDate = function() {
            const timeEl = document.querySelector('time[datetime]');
            if (timeEl) {
                return timeEl.getAttribute('datetime') || '';
            }

            const exactPatterns = [
                /(?:order date|placed|purchased on|purchase date)[:\\s]+([A-Za-z]+\\s+\\d{1,2},\\s*\\d{4})/i,
                /(?:order date|placed|purchased on|purchase date)[:\\s]+(\\d{2}\\/\\d{2}\\/\\d{4})/i,
            ];
            for (const pattern of exactPatterns) {
                const match = bodyText.match(pattern);
                if (match) {
                    return match[1];
                }
            }

            const genericMatch = bodyText.match(/[A-Za-z]+\\s+\\d{1,2},\\s*\\d{4}/);
            if (genericMatch) {
                return genericMatch[0];
            }
            return '';
        };

        const findTotal = function() {
            const patterns = [
                /(?:grand total|total|order total)\\s*[:\\n ]+(-?\\$?[\\d,]+\\.\\d{2})/i,
                /(-?\\$?[\\d,]+\\.\\d{2})\\s*(?:grand total|total|order total)/i,
            ];
            for (const pattern of patterns) {
                const match = bodyText.match(pattern);
                if (match) {
                    return match[1];
                }
            }
            return '';
        };

        const findLast4 = function() {
            const patterns = [
                /ending in\\s*(\\d{4})/i,
                /last\\s*4\\s*digits\\s*(\\d{4})/i,
                /card\\s*(?:ending|ends)?\\s*in\\s*(\\d{4})/i,
            ];
            for (const pattern of patterns) {
                const match = bodyText.match(pattern);
                if (match) {
                    return match[1];
                }
            }
            return '';
        };

        return JSON.stringify({
            title,
            url,
            purchaseDateText: findDate(),
            grandTotalText: findTotal(),
            paymentLast4: findLast4(),
            receiptImages: dataUrls,
        });
    })()`);
    const parsed = JSON.parse(String(raw || '{}'));
    return {
        url: String(parsed.url || ''),
        title: String(parsed.title || ''),
        purchaseDate: parseDateToIso(parsed.purchaseDateText),
        grandTotal: normalizeCurrencyAmount(parsed.grandTotalText),
        paymentLast4:
            String(parsed.paymentLast4 || '').match(/^\d{4}$/) != null
                ? String(parsed.paymentLast4)
                : null,
        receiptImages: Array.isArray(parsed.receiptImages)
            ? parsed.receiptImages
            : [],
    };
}

async function extractOrderPagePayload(page, orderType, id) {
    const raw = await page.evaluate(`(function() {
        const scriptObjects = [];
        for (const script of Array.from(
            document.querySelectorAll('script[type="application/json"], script[type="application/ld+json"]'),
        )) {
            const text = (script.textContent || '').trim();
            if (text === '') {
                continue;
            }
            try {
                scriptObjects.push(JSON.parse(text));
            } catch {
                scriptObjects.push(text);
            }
        }

        const nextDataEl = document.querySelector('#__NEXT_DATA__');
        let nextData = null;
        if (nextDataEl && (nextDataEl.textContent || '').trim() !== '') {
            try {
                nextData = JSON.parse(nextDataEl.textContent || 'null');
            } catch {
                nextData = nextDataEl.textContent || null;
            }
        }

        return JSON.stringify({
            capturedAt: new Date().toISOString(),
            pageUrl: window.location.href || '',
            title: document.title || '',
            orderType: ${JSON.stringify(orderType)},
            targetOrderId: ${JSON.stringify(id)},
            metadata: {
                bodyText: (document.body && document.body.innerText) || '',
            },
            nextData,
            scriptObjects,
        });
    })()`);
    return JSON.parse(String(raw || '{}'));
}

async function downloadJsonPayload(page, filename, payload) {
    const content = JSON.stringify(payload, null, 2);
    await page.goto('about:blank', {
        waitUntil: 'load',
        timeout: 10000,
    });
    await waitMs(page, 300);
    const downloadPromise = page.waitForDownload(10000);
    await page.evaluate(`(function(filename, content) {
        const blob = new Blob([content], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = filename;
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
        setTimeout(function() {
            URL.revokeObjectURL(url);
        }, 0);
    })(${JSON.stringify(filename)}, ${JSON.stringify(content)})`);
    return await downloadPromise;
}

async function discoverOnlineOrderIds(page) {
    const raw = await page.evaluate(`(function() {
        const ids = new Set();
        const add = function(value) {
            const id = String(value || '').trim();
            if (/^\\d{6,}$/.test(id)) {
                ids.add(id);
            }
        };

        for (const anchor of Array.from(document.querySelectorAll('a[href]'))) {
            const href = anchor.getAttribute('href') || '';
            const match = href.match(/\\/orders\\/(\\d{6,})(?:[/?#]|$)/);
            if (match) {
                add(match[1]);
            }
        }

        const bodyText = (document.body && document.body.innerText) || '';
        const bodyMatches = bodyText.match(/\\b9\\d{8,}\\b/g) || [];
        for (const match of bodyMatches) {
            add(match);
        }

        return JSON.stringify(Array.from(ids).sort());
    })()`);
    const parsed = JSON.parse(String(raw || '[]'));
    return Array.isArray(parsed) ? parsed : [];
}

async function discoverInStoreReceiptIds(page) {
    const raw = await page.evaluate(`(function() {
        const ids = new Set();
        const add = function(value) {
            const id = String(value || '').trim();
            if (/^\\d{4}-\\d{4}-\\d{4}-\\d{4}$/.test(id)) {
                ids.add(id);
            }
        };

        for (const anchor of Array.from(document.querySelectorAll('a[href]'))) {
            const href = anchor.getAttribute('href') || '';
            const match = href.match(
                /\\/orders\\/stores\\/(\\d{4}-\\d{4}-\\d{4}-\\d{4})(?:[/?#]|$)/,
            );
            if (match) {
                add(match[1]);
            }
        }

        const bodyText = (document.body && document.body.innerText) || '';
        const bodyMatches =
            bodyText.match(/\\b\\d{4}-\\d{4}-\\d{4}-\\d{4}\\b/g) || [];
        for (const match of bodyMatches) {
            add(match);
        }

        return JSON.stringify(Array.from(ids).sort());
    })()`);
    const parsed = JSON.parse(String(raw || '[]'));
    return Array.isArray(parsed) ? parsed : [];
}

async function clickInStoreTab(page) {
    try {
        const tab = page.getByRole('tab', { name: 'In-store' });
        if (await tab.isVisible()) {
            await tab.click({ timeout: 5000 });
            await waitMs(page, 1500);
            await page.waitForLoadState('networkidle', undefined);
            return true;
        }
    } catch (_err) {
        // fall through
    }

    try {
        const button = page.getByRole('button', { name: 'In-store' });
        if (await button.isVisible()) {
            await button.click({ timeout: 5000 });
            await waitMs(page, 1500);
            await page.waitForLoadState('networkidle', undefined);
            return true;
        }
    } catch (_err) {
        // fall through
    }

    return false;
}

async function loadAllInStorePurchases(page) {
    let stagnantIterations = 0;
    let knownCount = 0;

    while (stagnantIterations < 2) {
        const receiptIds = await discoverInStoreReceiptIds(page);
        if (receiptIds.length > knownCount) {
            knownCount = receiptIds.length;
            stagnantIterations = 0;
        } else {
            stagnantIterations++;
        }

        let clicked = false;
        try {
            const button = page.getByRole('button', {
                name: 'Load more purchases',
            });
            if (await button.isVisible()) {
                refreshmint.log(
                    'Target in-store branch: loading more purchases',
                );
                await button.click({ timeout: 5000 });
                clicked = true;
            }
        } catch (_err) {
            clicked = false;
        }

        if (!clicked) {
            break;
        }

        await waitMs(page, 2000);
        await page.waitForLoadState('networkidle', undefined);
    }
}

async function saveOrderPageAttachment(page, orderType, id, existingState) {
    const attachmentKey = makeTargetAttachmentKey(orderType, id);
    if (
        hasSavedSource(
            existingState,
            attachmentKey,
            SOURCE_KIND_ORDER_JSON,
            'single',
        )
    ) {
        refreshmint.log(
            `Target attachment already exists for ${attachmentKey} (${SOURCE_KIND_ORDER_JSON})`,
        );
        return { saved: false, purchaseDate: null };
    }

    const metadata = await extractOrderPageMetadata(page);
    const payload = await extractOrderPagePayload(page, orderType, id);
    const purchaseDate = metadata.purchaseDate || 'unknown-date';
    const prefix = orderType === 'online' ? 'online' : 'store';
    const filename = `orders/${prefix}-${sanitizeFilenameSegment(
        purchaseDate,
    )}-${sanitizeFilenameSegment(id)}.json`;
    payload.purchaseDate = metadata.purchaseDate;
    payload.grandTotal = metadata.grandTotal;
    payload.paymentLast4 = metadata.paymentLast4;
    payload.receiptImageCount = metadata.receiptImages.length;

    refreshmint.log(
        `Saving Target ${orderType} order JSON attachment ${filename}`,
    );
    const download = await downloadJsonPayload(
        page,
        `${sanitizeFilenameSegment(prefix)}-${sanitizeFilenameSegment(
            purchaseDate,
        )}-${sanitizeFilenameSegment(id)}.json`,
        payload,
    );
    await refreshmint.saveDownloadedResource(download.path, filename, {
        coverageEndDate: metadata.purchaseDate || undefined,
        originalUrl: metadata.url,
        mimeType: 'application/json',
        attachmentKey,
        attachmentType:
            orderType === 'online'
                ? 'target-order-json'
                : 'target-store-order-json',
        targetOrderType: orderType,
        targetOrderId: id,
        purchaseDate: metadata.purchaseDate || undefined,
        grandTotal: metadata.grandTotal || undefined,
        paymentLast4: metadata.paymentLast4 || undefined,
        sourceKind: SOURCE_KIND_ORDER_JSON,
        hasReceiptImage: metadata.receiptImages.length > 0,
    });
    markSavedSource(
        existingState,
        attachmentKey,
        SOURCE_KIND_ORDER_JSON,
        'single',
    );

    if (orderType === 'store') {
        await saveReceiptImages(
            id,
            metadata.purchaseDate,
            metadata.url || (await page.url()),
            metadata.grandTotal,
            metadata.paymentLast4,
            metadata.receiptImages,
            existingState,
        );
    }

    return { saved: true, purchaseDate: metadata.purchaseDate };
}

async function saveReceiptImages(
    receiptId,
    purchaseDate,
    originalUrl,
    grandTotal,
    paymentLast4,
    receiptImages,
    existingState,
) {
    const attachmentKey = makeTargetAttachmentKey('store', receiptId);
    for (let index = 0; index < receiptImages.length; index++) {
        const image = receiptImages[index];
        if (image == null || typeof image !== 'object') {
            continue;
        }
        const attachmentPart = `receipt-${index + 1}`;
        if (
            hasSavedSource(
                existingState,
                attachmentKey,
                SOURCE_KIND_RECEIPT_IMAGE,
                attachmentPart,
            )
        ) {
            continue;
        }

        const decoded = decodeDataUrl(image.dataUrl);
        if (decoded == null) {
            refreshmint.log(
                `Skipping Target receipt image ${attachmentPart}; failed to decode data URL`,
            );
            continue;
        }

        const filename = `receipts/store-${sanitizeFilenameSegment(
            purchaseDate || 'unknown-date',
        )}-${sanitizeFilenameSegment(receiptId)}-${attachmentPart}.gif`;
        refreshmint.log(`Saving Target receipt image attachment ${filename}`);
        await refreshmint.saveResource(filename, decoded.bytes, {
            coverageEndDate: purchaseDate || undefined,
            originalUrl,
            mimeType: decoded.mimeType || 'image/gif',
            attachmentKey,
            attachmentType: 'target-receipt-image',
            targetOrderType: 'store',
            targetOrderId: receiptId,
            purchaseDate: purchaseDate || undefined,
            grandTotal: grandTotal || undefined,
            paymentLast4: paymentLast4 || undefined,
            sourceKind: SOURCE_KIND_RECEIPT_IMAGE,
            attachmentPart,
        });
        markSavedSource(
            existingState,
            attachmentKey,
            SOURCE_KIND_RECEIPT_IMAGE,
            attachmentPart,
        );
    }
}

async function scrapeOnlineOrders(page, existingState) {
    await page.goto(TARGET_ORDERS_URL, {
        waitUntil: 'networkidle',
        timeout: 30000,
    });
    await waitMs(page, 1500);
    const orderIds = await discoverOnlineOrderIds(page);
    refreshmint.log(`Target discovered ${orderIds.length} online orders`);

    for (const orderId of orderIds) {
        const detailUrl = `${TARGET_ORDERS_URL}/${orderId}`;
        refreshmint.log(`Target visiting online order ${orderId}`);
        await page.goto(detailUrl, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await waitMs(page, 2000);
        await saveOrderPageAttachment(page, 'online', orderId, existingState);
        await humanPace(page, 500, 900);
    }
}

async function scrapeInStoreOrders(page, existingState) {
    await page.goto(TARGET_ORDERS_URL, {
        waitUntil: 'networkidle',
        timeout: 30000,
    });
    await waitMs(page, 1500);

    const switched = await clickInStoreTab(page);
    if (!switched) {
        refreshmint.log(
            'Target in-store tab not found; skipping in-store receipts',
        );
        return;
    }

    await loadAllInStorePurchases(page);
    const receiptIds = await discoverInStoreReceiptIds(page);
    refreshmint.log(`Target discovered ${receiptIds.length} in-store receipts`);

    for (const receiptId of receiptIds) {
        const detailUrl = `${TARGET_ORDERS_URL}/stores/${receiptId}`;
        refreshmint.log(`Target visiting in-store receipt ${receiptId}`);
        await page.goto(detailUrl, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await waitMs(page, 2000);
        await saveOrderPageAttachment(page, 'store', receiptId, existingState);
        await humanPace(page, 500, 900);
    }
}

/**
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function navigateToLogin(context) {
    const page = context.mainPage;
    refreshmint.log(`Navigating to ${TARGET_LOGIN_URL}`);
    await page.goto(TARGET_LOGIN_URL);
    return { progressName: `navigate to ${TARGET_LOGIN_URL}` };
}

/**
 * Expected page conditions:
 * - URL is on https://www.target.com/login or a nearby signin step.
 * - The page either shows the email field, password field, or is transitioning.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string}>}
 */
async function handleLogin(context) {
    const page = context.mainPage;
    const url = await page.url();
    if (!url.startsWith(TARGET_ORIGIN)) {
        throw new Error(`expected Target origin during login, got ${url}`);
    }

    await logStateSnapshot(page, 'target login snapshot');

    if (await hasSelector(page, '#username')) {
        refreshmint.log('Target login branch: email step');
        await page.locator('#username').fill('target_username');
        await humanPace(page, 300, 700);

        // Click Continue to advance from the email step to the password step.
        await page.getByRole('button', { name: 'Continue' }).click();
        await waitMs(page, 2000);
        return { progressName: 'submitted target email' };
    }

    if (await hasSelector(page, '#password')) {
        refreshmint.log('Target login branch: password step');
        await page.locator('#password').fill('target_password');
        await humanPace(page, 500, 1000);

        // Submit credentials and let Target redirect to the signed-in area.
        await page.getByRole('button', { name: 'Sign in' }).click();
        await waitMs(page, 4000);
        return { progressName: 'submitted target password' };
    }

    refreshmint.log(
        'Target login branch: waiting for recognizable login fields',
    );
    return { progressName: 'waiting for target login fields' };
}

/**
 * Expected page conditions:
 * - User is already signed in on target.com.
 * - Orders page is reachable and order attachments can be captured.
 *
 * @param {ScrapeContext} context
 * @returns {Promise<{progressName: string, done?: boolean}>}
 */
async function handleOrdersHome(context) {
    const page = context.mainPage;
    const url = await page.url();
    if (!url.startsWith(TARGET_ORIGIN)) {
        throw new Error(`expected Target origin on orders page, got ${url}`);
    }

    await logStateSnapshot(page, 'target orders snapshot');

    const existingState = buildExistingDocumentState(
        await listExistingTargetDocuments(),
    );

    await scrapeOnlineOrders(page, existingState);
    await scrapeInStoreOrders(page, existingState);

    refreshmint.log('Target attachment-only scrape complete');
    return { progressName: 'captured target order attachments', done: true };
}

async function main() {
    refreshmint.log('target scraper starting');
    const pages = await browser.pages();
    const mainPage = pages[0];
    if (mainPage == null) {
        throw new Error('expected at least one page');
    }

    /** @type {ScrapeContext} */
    const context = {
        mainPage,
        currentStep: 0,
        progressNames: [],
        progressNamesSet: new Set(),
        lastProgressStep: 0,
    };

    while (true) {
        context.currentStep++;
        const url = await context.mainPage.url();
        refreshmint.log(`Step ${context.currentStep}: URL=${url}`);

        /** @type {{progressName: string, done?: boolean}} */
        let stepReturn;

        if (url === 'about:blank') {
            stepReturn = await navigateToLogin(context);
        } else if (
            url.startsWith(TARGET_LOGIN_URL) ||
            url.startsWith(`${TARGET_ORIGIN}login?`)
        ) {
            stepReturn = await handleLogin(context);
        } else if (url.startsWith(TARGET_ORDERS_URL)) {
            stepReturn = await handleOrdersHome(context);
        } else if (url.startsWith(TARGET_ORIGIN)) {
            refreshmint.log(
                'Target branch: signed-in site page outside /orders, navigating to orders.',
            );
            await context.mainPage.goto(TARGET_ORDERS_URL);
            stepReturn = { progressName: 'navigate to target orders' };
        } else {
            stepReturn = await navigateToLogin(context);
        }

        const progressName = stepReturn.progressName;
        context.progressNames.push(progressName);
        if (!context.progressNamesSet.has(progressName)) {
            context.progressNamesSet.add(progressName);
            context.lastProgressStep = context.currentStep;
        }

        if (context.currentStep - context.lastProgressStep > 6) {
            throw new Error('no progress in last 6 steps');
        }
        if (stepReturn.done) {
            refreshmint.log('Scraping complete');
            break;
        }

        await humanPace(context.mainPage, 800, 1400);
    }
}

main().catch((err) => {
    refreshmint.log(`Fatal error: ${err.message}`);
    refreshmint.log(inspect(err));
    if (err.stack) {
        refreshmint.log(err.stack);
    }
});
