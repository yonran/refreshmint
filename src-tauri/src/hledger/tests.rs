use super::*;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn journal_roundtrip() {
    let amount = Amount {
        acommodity: "USD".to_string(),
        aquantity: DecimalRaw {
            decimal_places: 2,
            decimal_mantissa: Number::from(1234),
            floating_point: 12.34,
        },
        astyle: Some(AmountStyle {
            ascommodityside: Side::L,
            ascommodityspaced: false,
            asdigitgroups: None,
            asdecimalmark: Some('.'),
            asprecision: AmountPrecision::Precision(2),
            asrounding: Rounding::NoRounding,
        }),
        acost: None,
        acostbasis: None,
    };

    let posting = Posting {
        pdate: None,
        pdate2: None,
        pstatus: Status::Unmarked,
        paccount: "Assets:Cash".to_string(),
        pamount: vec![amount],
        pcomment: String::new(),
        ptype: PostingType::RegularPosting,
        ptags: Vec::new(),
        pbalanceassertion: None,
        ptransaction_index: None,
        poriginal: None,
    };

    let source_start = SourcePos {
        source_name: "journal.journal".to_string(),
        source_line: 10,
        source_column: 1,
    };
    let source_end = SourcePos {
        source_name: "journal.journal".to_string(),
        source_line: 12,
        source_column: 1,
    };

    let txn = Transaction {
        tindex: 1,
        tprecedingcomment: String::new(),
        tsourcepos: SourceSpan(source_start, source_end),
        tdate: "2024-01-02".to_string(),
        tdate2: None,
        tstatus: Status::Unmarked,
        tcode: String::new(),
        tdescription: "Coffee".to_string(),
        tcomment: String::new(),
        ttags: Vec::new(),
        tpostings: vec![posting],
    };

    let mut extra = BTreeMap::new();
    extra.insert("custom".to_string(), json!({"enabled": true}));

    let journal = Journal {
        jtxns: vec![txn],
        jpricedirectives: Vec::new(),
        jperiodictxns: Vec::new(),
        jtxnmodifiers: Vec::new(),
        jtimeclockentries: Vec::new(),
        jdeclaredpayees: Vec::new(),
        jdeclaredtags: Vec::new(),
        jdeclaredaccounts: Vec::new(),
        jdeclaredcommodities: BTreeMap::new(),
        extra,
    };

    let encoded = match serde_json::to_string(&journal) {
        Ok(value) => value,
        Err(err) => panic!("serialize journal failed: {err}"),
    };
    let decoded: Journal = match serde_json::from_str(&encoded) {
        Ok(value) => value,
        Err(err) => panic!("deserialize journal failed: {err}"),
    };

    assert_eq!(decoded, journal);
}

#[test]
fn amount_precision_natural_roundtrip() {
    let value = AmountStyle {
        ascommodityside: Side::L,
        ascommodityspaced: false,
        asdigitgroups: None,
        asdecimalmark: None,
        asprecision: AmountPrecision::Natural,
        asrounding: Rounding::NoRounding,
    };

    let encoded = match serde_json::to_value(&value) {
        Ok(value) => value,
        Err(err) => panic!("serialize amount style failed: {err}"),
    };
    assert_eq!(encoded.get("asprecision"), Some(&json!(null)));

    let decoded: AmountStyle = match serde_json::from_value(encoded) {
        Ok(value) => value,
        Err(err) => panic!("deserialize amount style failed: {err}"),
    };
    assert_eq!(decoded.asprecision, AmountPrecision::Natural);
}
