# Target Scraper Notes

## Current status

The Target extension is currently **attachment-only**.

What is implemented:

- Login with `target_username` / `target_password`
- Navigate to Target order history
- Discover online order IDs from the orders page
- Discover in-store receipt IDs from the in-store orders view
- Click through to individual order detail pages
- Save one JSON attachment per order detail page
- Save separate GIF receipt-image attachments for in-store purchases when the
  page exposes base64 receipt images
- Deduplicate reruns using account-document metadata and `attachmentKey`

What the scraper saves today:

- Online order attachment:
    - `orders/online-{purchaseDate}-{orderId}.json`
- In-store order attachment:
    - `orders/store-{purchaseDate}-{receiptId}.json`
- In-store receipt images:
    - `receipts/store-{purchaseDate}-{receiptId}-receipt-N.gif`

Common metadata written to saved documents:

- `attachmentKey`
- `attachmentType`
- `targetOrderType`
- `targetOrderId`
- `purchaseDate`
- `grandTotal`
- `paymentLast4`
- `sourceKind`
- `attachmentPart` for receipt images

## Important implementation detail

The saved JSON attachment is currently a **structured page payload** captured
from the rendered order page, not the raw Target API response body.

It includes:

- page URL
- page title
- order ID and order type
- extracted page text
- `__NEXT_DATA__` when present
- parsed JSON script blobs when present

This was implemented because the current Refreshmint browser API exposes
response URLs/status but not response bodies directly to extension code.

## Not implemented

The Target extension does **not** currently do any of the following:

- raw API-response capture for `post_orders` or `store_order_details`
- `extract.mjs` or `account.rules`
- transaction extraction
- linking Target attachments into journal entries
- matching receipts to bank or card transactions
- return/refund modeling
- item-level normalization
- purchase-vs-payment accounting

## TODO

Target-specific follow-up work:

- Capture raw Target API JSON if/when the scraper runtime exposes response
  bodies cleanly
- Verify selectors and flows against more account states
- Handle unexpected logged-in routes more explicitly
- Validate receipt-image discovery across more in-store purchase layouts
- Add stronger diagnostics when no online or in-store orders are found

Cross-cutting receipt/accounting decisions are tracked in
[docs/TODO.md](/Users/yonran/repos/refreshmint/docs/TODO.md).
