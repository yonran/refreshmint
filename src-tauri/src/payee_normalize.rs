//! Payee normalization for transaction descriptions.
//!
//! Collapses the many raw forms a single merchant appears in on bank statements
//! (`SQ *BLUE BOTTLE`, `COSTCO WHSE #0123`, `PURCHASE AUTHORIZED ON 06/12 SAFEWAY
//! #1234 OAKLAND CA`) down to a stable merchant key so that category rules and the
//! tokenizer can treat them as the same payee.
//!
//! Sibling of `transfer_detector.rs`: pure string heuristics, no I/O.
//!
//! Design bias (see the batch plan): stay conservative. A false merge (two
//! different merchants normalizing to the same key) is worse than a false split
//! (one merchant keeping two keys), so when a token is ambiguous we keep it.

use std::sync::OnceLock;

use regex::Regex;

/// Boilerplate prefixes that banks staple onto the front of the real merchant
/// name. Each is stripped once, longest/most-specific first. Patterns ending in a
/// date placeholder consume the date too (see `PREFIX_DATE_RE`).
const BOILERPLATE_PREFIXES: &[&str] = &[
    "PURCHASE AUTHORIZED ON",
    "RECURRING PAYMENT AUTHORIZED ON",
    "RECURRING PAYMENT",
    "DEBIT CARD PURCHASE",
    "DEBIT PURCHASE",
    "POS PURCHASE",
    "POS DEBIT",
    "CHECKCARD",
];

/// Processor prefixes glued directly to the merchant (often with `*`).
const PROCESSOR_PREFIXES: &[&str] = &["SQ *", "TST* ", "TST*", "PAYPAL *", "PY *", "SP *", "IC* "];

/// Canonical US state (+ DC) abbreviations. A trailing `<CITY> <STATE>` suffix is
/// stripped only when the last token is one of these, to bound false merges.
const STATE_ABBREVS: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS",
    "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY",
    "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC",
];

// Each regex is compiled once via OnceLock (LazyLock would exceed the crate's
// 1.77.2 MSRV). unwrap is safe: the patterns are compile-time literals.

/// A leading date consumed by prefixes like `PURCHASE AUTHORIZED ON 06/12` or the
/// bare `MMDD`/`MMDDYY` form that follows `CHECKCARD`.
#[allow(clippy::unwrap_used)]
fn prefix_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?|\d{4,6})\b").unwrap())
}

/// Trailing phone number, e.g. `800-555-1212`, `8005551212`, `415-555-1212`.
#[allow(clippy::unwrap_used)]
fn trailing_phone_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:\b1[-\s]?)?(?:\d{3}[-\s]?\d{3}[-\s]?\d{4}|\d{3}[-\s]?\d{7}|\d{10})\s*$")
            .unwrap()
    })
}

/// Trailing store number: `#0123`, `# 12`, or a bare long digit run (≥4 digits).
#[allow(clippy::unwrap_used)]
fn trailing_store_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:#\s*\w+|\b\d{4,})\s*$").unwrap())
}

/// Normalize a raw transaction description to a stable merchant key.
///
/// Uppercases, strips bank/processor boilerplate prefixes, and peels trailing
/// noise (store numbers, phone numbers, `<CITY> <STATE>` location suffixes),
/// collapsing whitespace. Returns the merchant portion; if stripping would empty
/// the string, the uppercased-collapsed original is returned instead (conservative
/// fallback — never return empty).
pub fn normalize_payee(description: &str) -> String {
    let upper = description.to_ascii_uppercase();
    let mut s = collapse_ws(&upper);

    s = strip_processor_prefix(&s);
    s = strip_boilerplate_prefix(&s);
    s = collapse_ws(&s);

    let stripped = strip_trailing_noise(&s);
    if stripped.is_empty() {
        // Everything looked like noise; keep the pre-strip merchant string rather
        // than collapse distinct merchants into "".
        s
    } else {
        stripped
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_processor_prefix(s: &str) -> String {
    for p in PROCESSOR_PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            return rest.trim_start().to_string();
        }
    }
    s.to_string()
}

fn strip_boilerplate_prefix(s: &str) -> String {
    for p in BOILERPLATE_PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            // Only strip when the prefix ends on a word boundary: either it is the
            // whole string, or a space follows. Otherwise `CHECKCARD` would bite
            // into `CHECKCARDIO GYM` -> `IO GYM`; keep trying later prefixes.
            let rest = if rest.is_empty() {
                rest
            } else if let Some(after_space) = rest.strip_prefix(' ') {
                after_space
            } else {
                continue;
            };
            // Consume an immediately-following date (e.g. `... ON 06/12`).
            let rest = prefix_date_re().replace(rest, "");
            return rest.trim_start().to_string();
        }
    }
    s.to_string()
}

fn strip_trailing_noise(s: &str) -> String {
    let mut cur = s.trim().to_string();
    loop {
        let before = cur.clone();

        // Phone numbers first (most specific).
        cur = trailing_phone_re().replace(&cur, "").trim_end().to_string();
        // Trailing `<CITY> <STATE>` location suffix.
        cur = strip_trailing_city_state(&cur);
        // Trailing store number / long digit run.
        cur = trailing_store_re().replace(&cur, "").trim_end().to_string();

        cur = cur.trim_end_matches(['*', '-', ',']).trim_end().to_string();

        if cur == before || cur.is_empty() {
            break;
        }
    }
    cur
}

/// Strip a trailing `<CITY> <STATE>` pair when the final token is a canonical US
/// state abbreviation and at least one merchant token would remain. Only one token
/// of city is removed; multi-word cities keep the leading words (a false split,
/// which is the safe direction).
fn strip_trailing_city_state(s: &str) -> String {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.len() < 3 {
        // Need merchant + city + state to safely drop city+state.
        return s.to_string();
    }
    let last = tokens[tokens.len() - 1];
    if STATE_ABBREVS.contains(&last) {
        // Drop state + one preceding city token.
        return tokens[..tokens.len() - 2].join(" ");
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_processor_star_prefix() {
        assert_eq!(normalize_payee("SQ *BLUE BOTTLE"), "BLUE BOTTLE");
        assert_eq!(normalize_payee("TST* THE COFFEE BAR"), "THE COFFEE BAR");
        assert_eq!(normalize_payee("PAYPAL *STEAM GAMES"), "STEAM GAMES");
    }

    #[test]
    fn strips_trailing_store_number() {
        assert_eq!(normalize_payee("COSTCO WHSE #0123"), "COSTCO WHSE");
        assert_eq!(normalize_payee("TARGET 00012345"), "TARGET");
    }

    #[test]
    fn strips_prefix_date_store_and_city_state() {
        assert_eq!(
            normalize_payee("PURCHASE AUTHORIZED ON 06/12 SAFEWAY #1234 OAKLAND CA"),
            "SAFEWAY"
        );
    }

    #[test]
    fn strips_pos_and_checkcard_prefixes() {
        assert_eq!(normalize_payee("POS DEBIT WALGREENS"), "WALGREENS");
        assert_eq!(normalize_payee("POS PURCHASE TRADER JOES"), "TRADER JOES");
        assert_eq!(normalize_payee("CHECKCARD 0612 WHOLE FOODS"), "WHOLE FOODS");
        assert_eq!(
            normalize_payee("DEBIT CARD PURCHASE SHELL OIL"),
            "SHELL OIL"
        );
    }

    #[test]
    fn strips_recurring_payment_prefix() {
        assert_eq!(normalize_payee("RECURRING PAYMENT NETFLIX"), "NETFLIX");
    }

    #[test]
    fn does_not_strip_boilerplate_prefix_glued_to_merchant() {
        // Boilerplate prefixes only strip on a word boundary (space or end of
        // string). A merchant whose name merely starts with those letters must
        // survive intact — `CHECKCARD` must not bite into `CHECKCARDIO GYM`, and
        // `POS DEBIT` must not bite into `POS DEBITX`.
        assert_eq!(normalize_payee("CHECKCARDIO GYM"), "CHECKCARDIO GYM");
        assert_eq!(normalize_payee("POS DEBITX"), "POS DEBITX");
    }

    #[test]
    fn strips_trailing_phone_number() {
        assert_eq!(normalize_payee("COMCAST 800-266-2278"), "COMCAST");
        assert_eq!(normalize_payee("AMZN MKTP 866-216-1072"), "AMZN MKTP");
    }

    #[test]
    fn collapses_whitespace_and_uppercases() {
        assert_eq!(normalize_payee("  blue   bottle  "), "BLUE BOTTLE");
    }

    #[test]
    fn keeps_merchant_when_everything_looks_like_noise() {
        // A bare store number should not normalize to empty.
        assert_eq!(normalize_payee("#0123"), "#0123");
    }

    #[test]
    fn does_not_overstrip_two_token_state_lookalike() {
        // Only two tokens: never treat the second as a state to drop.
        assert_eq!(normalize_payee("IN N"), "IN N");
    }

    #[test]
    fn conservative_multiword_city_keeps_leading_words() {
        // Multi-word city "SAN FRANCISCO": only "FRANCISCO CA" dropped, "SAN"
        // stays (a false split — the safe direction).
        assert_eq!(
            normalize_payee("PEETS COFFEE SAN FRANCISCO CA"),
            "PEETS COFFEE SAN"
        );
    }

    #[test]
    fn idempotent() {
        let once = normalize_payee("PURCHASE AUTHORIZED ON 06/12 SAFEWAY #1234 OAKLAND CA");
        assert_eq!(normalize_payee(&once), once);
    }
}
