GL Conversion Notes

Overview

- The GL model keeps numbers as DecimalString and dates as explicit structs to preserve ledger semantics across beancount and hledger.
- Converters report lossy/assumed behavior via ConversionReport (see `src-tauri/src/gl.rs`).

Numbers (DecimalString vs BigDecimal)

- DecimalString preserves the original lexical form: leading/trailing zeros, explicit +, and fixed scale.
- BigDecimal would normalize and lose the original formatting (eg `001.2300`, `+1.0`, or exponent forms).
- Hledger JSON uses mantissa + scale; we reconstruct DecimalString from that to preserve scale, not original lexeme.

Dates and DateTime

- Beancount is date-only; per-posting dates and time-of-day cannot be represented.
- Hledger timeclock entries include LocalTime without offset; GL DateTime keeps an optional offset, which is dropped on export to hledger.
- If a date/time string is malformed, converters default to 1970-01-01 or 00:00:00 and report an assumption.

Beancount conversion losses

- Posting amount: beancount allows a single amount; multi-amount postings are split into multiple postings and grouped via metadata, then re-merged on import when group metadata is present.
- Posting price type: beancount does not store unit vs total price in its core data, so total-vs-unit is preserved in metadata.
- Transaction comments/descriptions/status/date2 are stored in metadata and restored on import.
- Posting tags/links, posting-level balance assertions, and posting-level dates are not supported and are dropped.
- Amount display styles, amount-level cost/cost_basis are not stored and are dropped.
- Virtual/balanced-virtual postings are not supported and are dropped.

Hledger conversion losses

- Transaction payee/narration: hledger has a single description field; payee/narration are dropped if description is present.
- Transaction/posting links and metadata are not represented in hledger JSON and are dropped.
- Cost specifications (incomplete cost specs) are not representable and are dropped.
- Entry order is not preserved because hledger JSON groups directives by type.
- Missing posting amounts are represented using hledger's AUTO marker.

Other type notes

- Tags: beancount tags are sets without values; tag values and ordering are lost on export.
- Hidden tags: hledger uses underscore prefixes; GL tracks `hidden` separately and will prefix on export when needed.
- Source positions: default positions are filled when missing, which may lose original file/line context.
