/// Heuristic transfer detection for inter-account payments.
///
/// Analyzes transaction descriptions to flag probable transfers between accounts.
/// Check if a transaction description looks like an inter-account transfer.
pub fn is_probable_transfer(description: &str) -> bool {
    let upper = description.to_ascii_uppercase();

    for pattern in TRANSFER_PATTERNS {
        if upper.contains(pattern) {
            return true;
        }
    }

    false
}

/// Transfer patterns to look for in transaction descriptions.
const TRANSFER_PATTERNS: &[&str] = &[
    "TRANSFER TO",
    "TRANSFER FROM",
    "XFER TO",
    "XFER FROM",
    "ONLINE TRANSFER",
    "FUNDS TRANSFER",
    "WIRE TRANSFER",
    "ACH TRANSFER",
    "INTERNAL TRANSFER",
    "PAYMENT THANK YOU",
    "PAYMENT - THANK YOU",
    "AUTOPAY",
    "AUTO PAY",
    "AUTO-PAY",
    "AUTOMATIC PAYMENT",
    "VENMO",
    "ZELLE",
    "PAYPAL TRANSFER",
    "DIRECT DEBIT",
    "DIRECT DEPOSIT",
    "BALANCE TRANSFER",
    "CC PAYMENT",
    "CREDIT CARD PAYMENT",
    "CARDMEMBER SVCS",
    "INTERNET PAYMENT",
    "EPAYMENT",
    "MOBILE PAYMENT",
];

/// Detailed classification of a probable transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferType {
    /// Generic transfer (direction unknown from description alone).
    Generic,
    /// Likely an outgoing transfer.
    Outgoing,
    /// Likely an incoming transfer.
    Incoming,
    /// Credit card payment.
    CreditCardPayment,
    /// Peer-to-peer payment service (Venmo, Zelle, PayPal).
    PeerToPeer,
}

/// Classify a transfer description into a more specific type.
pub fn classify_transfer(description: &str) -> Option<TransferType> {
    let upper = description.to_ascii_uppercase();

    if !is_probable_transfer(description) {
        return None;
    }

    // Credit card payment patterns
    if upper.contains("PAYMENT THANK YOU")
        || upper.contains("PAYMENT - THANK YOU")
        || upper.contains("CC PAYMENT")
        || upper.contains("CREDIT CARD PAYMENT")
        || upper.contains("CARDMEMBER SVCS")
    {
        return Some(TransferType::CreditCardPayment);
    }

    // P2P patterns
    if upper.contains("VENMO") || upper.contains("ZELLE") || upper.contains("PAYPAL TRANSFER") {
        return Some(TransferType::PeerToPeer);
    }

    // Directional patterns
    if upper.contains("TRANSFER TO")
        || upper.contains("XFER TO")
        || upper.contains("AUTOPAY")
        || upper.contains("AUTO PAY")
        || upper.contains("AUTO-PAY")
        || upper.contains("AUTOMATIC PAYMENT")
    {
        return Some(TransferType::Outgoing);
    }

    if upper.contains("TRANSFER FROM")
        || upper.contains("XFER FROM")
        || upper.contains("DIRECT DEPOSIT")
    {
        return Some(TransferType::Incoming);
    }

    Some(TransferType::Generic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_transfers() {
        assert!(is_probable_transfer("ONLINE TRANSFER TO SAVINGS"));
        assert!(is_probable_transfer("PAYMENT THANK YOU - AUTOPAY"));
        assert!(is_probable_transfer("VENMO PAYMENT John Doe"));
        assert!(is_probable_transfer("ZELLE TO Jane Smith"));
        assert!(is_probable_transfer("Online Transfer from CHK 1234"));
        assert!(is_probable_transfer("AUTOPAY 1234"));
    }

    #[test]
    fn rejects_non_transfers() {
        assert!(!is_probable_transfer("SHELL OIL 12345"));
        assert!(!is_probable_transfer("WALMART SUPERCENTER"));
        assert!(!is_probable_transfer("AMAZON.COM"));
        assert!(!is_probable_transfer("STARBUCKS 1234"));
    }

    #[test]
    fn classifies_credit_card_payment() {
        assert_eq!(
            classify_transfer("PAYMENT THANK YOU - AUTOPAY"),
            Some(TransferType::CreditCardPayment)
        );
        assert_eq!(
            classify_transfer("CARDMEMBER SVCS - ONLINE PMT"),
            Some(TransferType::CreditCardPayment)
        );
    }

    #[test]
    fn classifies_p2p() {
        assert_eq!(
            classify_transfer("VENMO PAYMENT John Doe"),
            Some(TransferType::PeerToPeer)
        );
        assert_eq!(
            classify_transfer("ZELLE TO Jane"),
            Some(TransferType::PeerToPeer)
        );
    }

    #[test]
    fn classifies_directional() {
        assert_eq!(
            classify_transfer("ONLINE TRANSFER TO SAVINGS"),
            Some(TransferType::Outgoing)
        );
        assert_eq!(
            classify_transfer("TRANSFER FROM CHECKING"),
            Some(TransferType::Incoming)
        );
    }

    #[test]
    fn returns_none_for_non_transfer() {
        assert_eq!(classify_transfer("SHELL OIL 12345"), None);
    }
}
