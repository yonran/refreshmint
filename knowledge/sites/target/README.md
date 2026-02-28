# Target.com Order Scraping

## Note

This file is research and implementation guidance, not a description of the
current shipped Target extension behavior.

Current implementation status lives in
`builtin-extensions/target/README.md`.

## Regulatory Status

**Regulation:** None — retail chain, not a financial institution. Online and in-store order history are voluntary services, not regulated documents.

## Login Flow

1. Navigate to `https://www.target.com/login`
2. Enter email in `#username` field
3. Click "Continue" button
4. Enter password in `#password` field
5. Click "Sign in" button
6. No MFA required (as of Feb 2026)

Selectors used:

- Email: `#username`
- Password: `#password`
- Continue/Sign in buttons: role-based (`button[name="Continue"]`, `button[name="Sign in"]`)

## Order Types

Target has two separate order history views:

- **Online orders** (`/orders` tab "Online"): Shipped/delivered orders placed on target.com
- **In-store purchases** (`/orders` tab "In-store"): Physical store purchases linked to Target Circle account

## API Endpoints

### Online Order History (list)

```
GET https://api.target.com/guest_order_aggregations/v1/order_history
  ?page_number=1
  &page_size=10
  &order_purchase_type=ONLINE
  &pending_order=true
  &shipt_status=true
```

- Paginated: `total_orders`, `total_pages` in response
- Returns order summaries with `order_number`, `placed_date`, `summary.grand_total`, `order_lines[]`

### Online Order Detail

```
GET https://api.target.com/post_orders/v1/{order_number}
```

- Web page URL: `https://www.target.com/orders/{order_number}`
- Response keys: `order_number`, `order_date`, `summary` (pricing breakdown), `packages[].order_lines[]` (items), `payments`, `addresses`, `guest_profile`

### In-Store Order Detail

```
GET https://api.target.com/guest_order_aggregations/v1/{receipt_id}/store_order_details?subscription=false
```

- Web page URL: `https://www.target.com/orders/stores/{receipt_id}`
- Response keys: `store_receipt_id`, `store_id`, `order_purchase_date`, `grand_total`, `order_lines[]`, `summary` (with adjustments/discounts), `payment_transactions[]`, `address[]`, `receipts` (base64 GIF receipt images)

### In-Store Order List

- No direct API for listing in-store orders
- Must navigate to `https://www.target.com/orders`, click "In-store" tab, then click "Load more purchases" repeatedly (about 6 times for 65 orders)
- Receipt IDs are extracted from the page DOM

## Authentication / API Access

**Critical: Direct `fetch()` from `page.evaluate()` fails with 401 Unauthorized** because the API requires an `x-api-key` header that only the browser's built-in request pipeline provides.

The working approach is **response interception**: navigate to each order page and capture the API response as it flows through.

## Recommended Scrape Pattern: Save-As-You-Go

**Do not** collect all data first and then save in a second phase. If the download phase fails, you lose everything and must re-fetch. Instead, **save each order immediately after capturing it**.

Pattern for each order:

1. Attach response handler
2. Navigate to order page → capture JSON via response handler
3. Navigate to `about:blank`
4. Download JSON file via blob pattern (10s timeout, retry once on timeout)
5. Remove response handler
6. Repeat for next order

This way each order is persisted before moving to the next — if something fails mid-batch, you only need to retry the failed IDs.

### In-Store Orders

```javascript
async (page) => {
    const receiptIds = ['6032-2290-0172-8918' /* ... */];
    const outputDir = '.playwright-mcp/target-instore-orders';
    const results = { saved: [], errors: [] };

    for (const id of receiptIds) {
        let orderData = null;

        const handler = async (response) => {
            const url = response.url();
            if (url.includes('store_order_details') && url.includes(id)) {
                try {
                    const contentType =
                        response.headers()['content-type'] || '';
                    if (contentType.includes('json')) {
                        orderData = await response.json();
                    }
                } catch (e) {}
            }
        };

        page.on('response', handler);

        try {
            // 1. Navigate to order page and capture response
            await page.goto(`https://www.target.com/orders/stores/${id}`, {
                waitUntil: 'networkidle',
                timeout: 30000,
            });
            await page.waitForTimeout(1500);

            if (!orderData) {
                results.errors.push({ id, error: 'No response captured' });
                page.removeListener('response', handler);
                continue;
            }

            // 2. Navigate to about:blank for reliable blob downloads
            page.removeListener('response', handler);
            await page.goto('about:blank');

            // 3. Save immediately via blob download
            const date =
                orderData.order_purchase_date?.slice(0, 10) || 'unknown';
            const filename = `store_order_details-${date}-${id}.json`;
            const content = JSON.stringify(orderData, null, 2);

            let saved = false;
            for (let attempt = 0; attempt < 2 && !saved; attempt++) {
                try {
                    const [download] = await Promise.all([
                        page.waitForEvent('download', { timeout: 10000 }),
                        page.evaluate(
                            ({ filename, content }) => {
                                const blob = new Blob([content], {
                                    type: 'application/json',
                                });
                                const url = URL.createObjectURL(blob);
                                const a = document.createElement('a');
                                a.href = url;
                                a.download = filename;
                                document.body.appendChild(a);
                                a.click();
                                a.remove();
                                URL.revokeObjectURL(url);
                            },
                            { filename, content },
                        ),
                    ]);
                    await download.saveAs(`${outputDir}/${filename}`);
                    saved = true;
                    console.log(
                        `Saved ${filename} (${orderData.order_lines?.length || 0} items, $${orderData.grand_total})`,
                    );
                } catch (e) {
                    if (attempt === 0)
                        console.log(`Retry download for ${id}: ${e.message}`);
                    else
                        results.errors.push({
                            id,
                            error: `Download failed: ${e.message}`,
                        });
                }
            }

            if (saved) results.saved.push(id);
        } catch (e) {
            page.removeListener('response', handler);
            results.errors.push({ id, error: e.message });
            console.log(`ERR: ${id} - ${e.message}`);
        }
    }

    return results;
};
```

### Online Orders

For online orders, use `post_orders` in the URL match. Use a **broader match** pattern — the API path may vary slightly (e.g. `post_orders/v1/` vs `post_orders/v2/`):

```javascript
// Response handler for online orders — broader match
const handler = async (response) => {
    const url = response.url();
    if (url.includes('post_orders') && url.includes(orderNumber)) {
        try {
            const contentType = response.headers()['content-type'] || '';
            if (contentType.includes('json')) {
                orderData = await response.json();
            }
        } catch (e) {}
    }
};

// Navigate and wait longer — online order API responses can be slow
await page.goto(`https://www.target.com/orders/${orderNumber}`, {
    waitUntil: 'networkidle',
    timeout: 30000,
});
await page.waitForTimeout(3000); // 3s, not 1.5s — API response may arrive late
```

## Blob Download Notes

Since `browser_run_code` has no `fs` access, use the blob download pattern. Key reliability tips:

- **Always download from `about:blank`**: Target.com page scripts can interfere with blob downloads (CSP, event handlers). Navigate to `about:blank` before triggering the download.
- **Use 10s timeout**: `page.waitForEvent('download', { timeout: 10000 })` — 5s caused ~8% timeout rate.
- **Retry once on timeout**: A single retry resolves most transient failures.
- **Log what was saved**: Since you can't verify file contents from `browser_run_code`, log details so failures are diagnosable.

See the save-as-you-go code above for the complete pattern.

## Data Format Details

### In-Store Order (`store_order_details`)

- `store_receipt_id`: e.g. `"6032-2290-0172-8918"` (4 groups of digits)
- `order_purchase_date`: ISO 8601 with timezone, e.g. `"2026-02-01T19:31:36-06:00"`
- `grand_total`: string dollar amount, e.g. `"2.65"`
- `order_lines[].item.description`: may contain HTML entities (`&#38;` for &, `&#39;` for ', `&#8482;` for TM)
- `order_lines[].item.unit_price` / `list_price`: string dollar amounts
- `order_lines[].item.product_classification`: `product_type_name` (e.g. GROCERY, APPAREL, HOME), `product_subtype_name`, `merchandise_type_name`
- `summary.adjustments[]`: discount details including Target Circle Card 5%, promotions
- `payment_transactions[]`: payment method details (card number last 4, type)
- `receipts`: base64-encoded GIF receipt images (large, ~2-10KB each)
- `address[]`: store location details

### Online Order (`post_orders`)

- `order_number`: numeric string, e.g. `"912002749053897"`
- `order_date`: ISO 8601
- `summary`: pricing breakdown with `grand_total`, `total_product_price`, `total_taxes`, etc.
- `packages[].order_lines[]`: items grouped by package/shipment
- `payments[]`: payment method details

## Gotchas

1. **x-api-key required**: Can't directly `fetch()` the API — must use response interception via `page.on('response', handler)`
2. **Download timeouts on target.com pages**: The blob download pattern sometimes fails with timeout on target.com pages. Navigate to `about:blank` first, then do downloads there. Use 10s timeout and retry once.
3. **Window data lost on every `page.goto()`**: Each navigation creates a new page context — anything stored on `window` is wiped. **Do not** collect data on `window` across navigations. Use the save-as-you-go pattern instead: capture → navigate to `about:blank` → save → next order.
4. **Two-phase collect-then-save is fragile**: Collecting all data first and then saving in bulk means any failure in the save phase requires re-fetching everything. The save-as-you-go pattern above avoids this.
5. **In-store orders require "Load more"**: The in-store tab initially shows ~10 orders. Must click "Load more purchases" button repeatedly to load all.
6. **HTML entities in descriptions**: Item descriptions may contain `&#38;`, `&#39;`, `&#8482;` etc.
7. **Order ID format varies**: Most online orders start with `912...` but at least one starts with `902...`
8. **Receipt images are large base64**: The `receipts` field contains full GIF receipt images encoded as base64 data URIs. This makes individual JSON files ~20-50KB.
9. **Online order response interception can miss**: Use a broad URL match (`url.includes('post_orders')` not `url.includes('post_orders/v1/')`) and wait 3s after navigation (not 1.5s). The API response may arrive late.

## File Output

### In-store orders

- Directory: `.playwright-mcp/target-instore-orders/`
- Naming: `store_order_details-{yyyy-mm-dd}-{receipt_id}.json`
