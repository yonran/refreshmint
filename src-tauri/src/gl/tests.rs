use super::{beancount, hledger, *};

fn date(year: i32, month: u8, day: u8) -> Date {
    Date { year, month, day }
}

fn amount(commodity: &str, quantity: &str) -> Amount {
    Amount {
        commodity: commodity.to_string(),
        quantity: DecimalString(quantity.to_string()),
        style: None,
        cost: None,
        cost_basis: None,
    }
}

#[test]
fn beancount_split_merge_postings_roundtrip() {
    let posting = Posting {
        account: "Assets:Cash".to_string(),
        amount: Some(MixedAmount {
            amounts: vec![amount("USD", "10"), amount("EUR", "5")],
        }),
        lot_cost: None,
        cost_spec: None,
        price: Some(PriceSpec {
            amount: amount("USD", "1.5"),
            price_type: PriceType::Total,
        }),
        status: Some(Status::Pending),
        flag: Some("!".to_string()),
        tags: vec![Tag {
            name: "tag".to_string(),
            value: Some("value".to_string()),
            hidden: false,
        }],
        links: vec!["link-1".to_string()],
        comment: Some("posting comment".to_string()),
        posting_type: PostingType::Virtual,
        balance_assertion: Some(BalanceAssertion {
            amount: amount("USD", "100"),
            total: true,
            inclusive: false,
            source: None,
        }),
        date: Some(date(2024, 1, 2)),
        date2: None,
        meta: Metadata::new(),
        source: None,
    };

    let txn = Transaction {
        date: date(2024, 1, 1),
        date2: None,
        status: None,
        flag: None,
        code: None,
        payee: None,
        narration: Some("Test".to_string()),
        description: None,
        comment: None,
        preceding_comment: None,
        tags: Vec::new(),
        links: Vec::new(),
        postings: vec![posting.clone()],
        index: None,
        meta: Metadata::new(),
        source: None,
    };

    let ledger = Ledger {
        entries: vec![Entry::Transaction(txn)],
        meta: Metadata::new(),
    };

    let (b_ledger, _) = beancount::to_beancount(&ledger);
    assert_eq!(b_ledger.entries.len(), 1);
    let beancount::Directive::Transaction(b_txn) = &b_ledger.entries[0] else {
        panic!("expected beancount transaction");
    };
    assert_eq!(b_txn.postings.len(), 2);
    assert!(b_txn.postings.iter().all(|p| p.meta.is_some()));

    let (roundtrip, _) = beancount::from_beancount(&b_ledger);
    let Entry::Transaction(round_txn) = &roundtrip.entries[0] else {
        panic!("expected roundtripped transaction");
    };
    assert_eq!(round_txn.postings.len(), 1);
    assert_eq!(round_txn.postings[0], posting);
}

#[test]
fn beancount_transaction_metadata_roundtrip() {
    let posting = Posting {
        account: "Assets:Cash".to_string(),
        amount: Some(MixedAmount {
            amounts: vec![amount("USD", "10")],
        }),
        lot_cost: None,
        cost_spec: None,
        price: None,
        status: None,
        flag: None,
        tags: Vec::new(),
        links: Vec::new(),
        comment: None,
        posting_type: PostingType::Regular,
        balance_assertion: None,
        date: None,
        date2: None,
        meta: Metadata::new(),
        source: None,
    };

    let txn = Transaction {
        date: date(2024, 2, 1),
        date2: Some(date(2024, 2, 2)),
        status: Some(Status::Cleared),
        flag: Some("*".to_string()),
        code: Some("CODE".to_string()),
        payee: Some("Payee".to_string()),
        narration: Some("Narration".to_string()),
        description: Some("Description".to_string()),
        comment: Some("Comment".to_string()),
        preceding_comment: Some("Leading comment".to_string()),
        tags: vec![Tag {
            name: "tag".to_string(),
            value: Some("v".to_string()),
            hidden: true,
        }],
        links: vec!["link".to_string()],
        postings: vec![posting],
        index: Some(42),
        meta: Metadata::new(),
        source: None,
    };

    let ledger = Ledger {
        entries: vec![Entry::Transaction(txn.clone())],
        meta: Metadata::new(),
    };

    let (b_ledger, _) = beancount::to_beancount(&ledger);
    let (roundtrip, _) = beancount::from_beancount(&b_ledger);
    let Entry::Transaction(round_txn) = &roundtrip.entries[0] else {
        panic!("expected roundtripped transaction");
    };

    assert_eq!(round_txn.date2, txn.date2);
    assert_eq!(round_txn.status, txn.status);
    assert_eq!(round_txn.code, txn.code);
    assert_eq!(round_txn.description, txn.description);
    assert_eq!(round_txn.comment, txn.comment);
    assert_eq!(round_txn.preceding_comment, txn.preceding_comment);
    assert_eq!(round_txn.index, txn.index);
    assert_eq!(round_txn.tags, txn.tags);
    assert_eq!(round_txn.links, txn.links);
}

#[test]
fn hledger_missing_amount_roundtrip() {
    let posting = Posting {
        account: "Assets:Cash".to_string(),
        amount: None,
        lot_cost: None,
        cost_spec: None,
        price: None,
        status: None,
        flag: None,
        tags: Vec::new(),
        links: Vec::new(),
        comment: None,
        posting_type: PostingType::Regular,
        balance_assertion: None,
        date: None,
        date2: None,
        meta: Metadata::new(),
        source: None,
    };

    let txn = Transaction {
        date: date(2024, 3, 1),
        date2: None,
        status: None,
        flag: None,
        code: None,
        payee: None,
        narration: None,
        description: Some("Description".to_string()),
        comment: None,
        preceding_comment: None,
        tags: Vec::new(),
        links: Vec::new(),
        postings: vec![posting],
        index: None,
        meta: Metadata::new(),
        source: None,
    };

    let ledger = Ledger {
        entries: vec![Entry::Transaction(txn)],
        meta: Metadata::new(),
    };

    let (journal, _) = hledger::to_hledger_journal(&ledger);
    let posting_amounts = &journal.jtxns[0].tpostings[0].pamount;
    assert_eq!(posting_amounts.len(), 1);
    assert_eq!(posting_amounts[0].acommodity, "AUTO");

    let (roundtrip, _) = hledger::from_hledger_journal(&journal);
    let Entry::Transaction(round_txn) = &roundtrip.entries[0] else {
        panic!("expected roundtripped transaction");
    };
    assert!(round_txn.postings[0].amount.is_none());
}
