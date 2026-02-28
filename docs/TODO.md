# TODO

## Receipts And Retail Attachments

### Product model

We currently treat retail receipts as attachment-only evidence. Future work needs
to decide whether receipts should remain evidence-only or evolve into one of
these richer models:

- Receipt-enriched bank transactions
- Receipt-created expense transactions
- Explicit purchase/payment/fulfillment/refund event modeling

### Date semantics

If receipts become more than attachments, define and preserve separate meanings
for:

- Purchase date
- Fulfillment date
- Payment or capture date
- Refund date
- Statement or posting date

Future implementations should decide which of these dates drives ledger entries
under each product mode.

### Matching policy

We need a clear matching policy between receipt artifacts and imported bank/card
transactions, including:

- allowed date drift
- allowed amount drift
- split shipments and split captures
- refunds and partial refunds
- double-count prevention when both receipts and bank imports exist

### Extraction shape

Future receipt extraction needs decisions on:

- whether receipts should ever create transactions directly
- whether extraction should operate at purchase level or item level
- whether taxes, discounts, shipping, and fees should become explicit fields or
  postings
- whether receipt images remain evidence-only attachments

### Stable metadata contract

The phase-1 attachment metadata contract should stay additive. Current stable
keys for receipt-style attachments are:

- `attachmentKey`
- `attachmentType`
- `purchaseDate`
- `sourceKind`
- `attachmentPart`
- provider-specific IDs such as `targetOrderId`

Future changes should add metadata keys rather than renaming or repurposing
these.

### Returns and refunds

We still need a product decision for:

- whether return receipts are separate attachment groups
- whether refunds should be modeled as separate events
- how returned items should link back to original evidence

### Online vs in-store normalization

Retail providers often expose different levels of detail for online and
in-store purchases. We need to decide whether all retail receipt sources should
normalize to one shared schema or preserve source-specific differences where the
data is materially different.
