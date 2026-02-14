use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub jtxns: Vec<Transaction>,
    #[serde(default)]
    pub jpricedirectives: Vec<PriceDirective>,
    #[serde(default)]
    pub jperiodictxns: Vec<PeriodicTransaction>,
    #[serde(default)]
    pub jtxnmodifiers: Vec<TransactionModifier>,
    #[serde(default)]
    pub jtimeclockentries: Vec<TimeclockEntry>,
    #[serde(default)]
    pub jdeclaredpayees: Vec<(String, PayeeDeclarationInfo)>,
    #[serde(default)]
    pub jdeclaredtags: Vec<(String, TagDeclarationInfo)>,
    #[serde(default)]
    pub jdeclaredaccounts: Vec<(String, AccountDeclarationInfo)>,
    #[serde(default)]
    pub jdeclaredcommodities: BTreeMap<String, Commodity>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecimalRaw {
    #[serde(rename = "decimalPlaces")]
    pub decimal_places: u32,
    #[serde(rename = "decimalMantissa")]
    pub decimal_mantissa: Number,
    #[serde(rename = "floatingPoint")]
    pub floating_point: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePos {
    #[serde(rename = "sourceName")]
    pub source_name: String,
    #[serde(rename = "sourceLine")]
    pub source_line: u32,
    #[serde(rename = "sourceColumn")]
    pub source_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan(pub SourcePos, pub SourcePos);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BalanceAssertion {
    pub baamount: Amount,
    pub batotal: bool,
    pub bainclusive: bool,
    pub baposition: SourcePos,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Posting {
    pub pdate: Option<String>,
    pub pdate2: Option<String>,
    pub pstatus: Status,
    pub paccount: String,
    pub pamount: MixedAmount,
    pub pcomment: String,
    pub ptype: PostingType,
    pub ptags: Vec<HledgerTag>,
    pub pbalanceassertion: Option<BalanceAssertion>,
    #[serde(rename = "ptransaction_")]
    pub ptransaction_index: Option<String>,
    pub poriginal: Option<Box<Posting>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub tindex: i64,
    pub tprecedingcomment: String,
    pub tsourcepos: SourceSpan,
    pub tdate: String,
    pub tdate2: Option<String>,
    pub tstatus: Status,
    pub tcode: String,
    pub tdescription: String,
    pub tcomment: String,
    pub ttags: Vec<HledgerTag>,
    pub tpostings: Vec<Posting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionModifier {
    pub tmquerytxt: String,
    pub tmpostingrules: Vec<TMPostingRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TMPostingRule {
    #[serde(rename = "tmprPosting")]
    pub tmpr_posting: Posting,
    #[serde(rename = "tmprIsMultiplier")]
    pub tmpr_is_multiplier: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodicTransaction {
    pub ptperiodexpr: String,
    pub ptinterval: serde_json::Value,
    pub ptspan: serde_json::Value,
    pub ptsourcepos: SourceSpan,
    pub ptstatus: Status,
    pub ptcode: String,
    pub ptdescription: String,
    pub ptcomment: String,
    pub pttags: Vec<HledgerTag>,
    pub ptpostings: Vec<Posting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeclockEntry {
    pub tlsourcepos: SourcePos,
    pub tlcode: TimeclockCode,
    pub tldatetime: String,
    pub tlaccount: String,
    pub tldescription: String,
    pub tlcomment: String,
    pub tltags: Vec<HledgerTag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceDirective {
    pub pdsourcepos: SourcePos,
    pub pddate: String,
    pub pdcommodity: String,
    pub pdamount: Amount,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountDeclarationInfo {
    pub adicomment: String,
    pub aditags: Vec<HledgerTag>,
    pub adideclarationorder: i64,
    pub adisourcepos: SourcePos,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayeeDeclarationInfo {
    pub pdicomment: String,
    pub pditags: Vec<HledgerTag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagDeclarationInfo {
    pub tdicomment: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commodity {
    pub csymbol: String,
    pub cformat: Option<AmountStyle>,
    pub ccomment: String,
    pub ctags: Vec<HledgerTag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostBasis {
    #[serde(rename = "cbCost")]
    pub cb_cost: Option<Box<Amount>>,
    #[serde(rename = "cbDate")]
    pub cb_date: Option<String>,
    #[serde(rename = "cbLabel")]
    pub cb_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amount {
    pub acommodity: String,
    pub aquantity: DecimalRaw,
    pub astyle: Option<AmountStyle>,
    pub acost: Option<AmountCost>,
    pub acostbasis: Option<CostBasis>,
}

pub type MixedAmount = Vec<Amount>;

pub type HledgerTag = (String, String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmountStyle {
    pub ascommodityside: Side,
    pub ascommodityspaced: bool,
    pub asdigitgroups: Option<DigitGroupStyle>,
    pub asdecimalmark: Option<char>,
    pub asprecision: AmountPrecision,
    pub asrounding: Rounding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    L,
    R,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
pub enum DigitGroupStyle {
    DigitGroups(char, Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmountPrecision {
    Precision(u8),
    Natural,
}

mod amount_precision_serde {
    use super::AmountPrecision;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &AmountPrecision, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            AmountPrecision::Precision(number) => serializer.serialize_some(number),
            AmountPrecision::Natural => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<AmountPrecision, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<u8>::deserialize(deserializer)?;
        Ok(match value {
            Some(number) => AmountPrecision::Precision(number),
            None => AmountPrecision::Natural,
        })
    }
}

impl Serialize for AmountPrecision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        amount_precision_serde::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for AmountPrecision {
    fn deserialize<D>(deserializer: D) -> Result<AmountPrecision, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        amount_precision_serde::deserialize(deserializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounding {
    NoRounding,
    SoftRounding,
    HardRounding,
    AllRounding,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
pub enum AmountCost {
    UnitCost(Box<Amount>),
    TotalCost(Box<Amount>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostingType {
    RegularPosting,
    VirtualPosting,
    BalancedVirtualPosting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Unmarked,
    Pending,
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeclockCode {
    SetBalance,
    SetRequiredHours,
    In,
    Out,
    FinalOut,
}

pub fn from_hledger_journal(journal: &Journal) -> (super::Ledger, ConversionReport) {
    let mut report = ConversionReport::default();
    if !journal.extra.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::Journal",
            "extra fields dropped from journal",
        );
    }

    let mut entries = Vec::new();

    for (name, info) in &journal.jdeclaredaccounts {
        entries.push(Entry::Account(super::AccountDirective {
            account: name.clone(),
            comment: string_to_option(&info.adicomment),
            tags: tags_from_hledger(&info.aditags),
            order: Some(info.adideclarationorder as u64),
            meta: Metadata::new(),
            source: Some(source_pos_to_span(&info.adisourcepos)),
        }));
    }

    for (name, info) in &journal.jdeclaredpayees {
        if !info.pditags.is_empty() {
            report.push(
                ConversionIssueKind::Dropped,
                format!("hledger::Payee {name}"),
                "payee tags have no equivalent in the GL model",
            );
        }
        entries.push(Entry::Payee(super::PayeeDirective {
            payee: name.clone(),
            comment: string_to_option(&info.pdicomment),
            meta: Metadata::new(),
            source: None,
        }));
    }

    for (name, info) in &journal.jdeclaredtags {
        entries.push(Entry::Tag(super::TagDirective {
            name: name.clone(),
            comment: string_to_option(&info.tdicomment),
            meta: Metadata::new(),
            source: None,
        }));
    }

    for (symbol, commodity) in &journal.jdeclaredcommodities {
        entries.push(Entry::Commodity(super::CommodityDirective {
            date: None,
            symbol: symbol.clone(),
            format: commodity.cformat.as_ref().map(amount_style_from_hledger),
            comment: string_to_option(&commodity.ccomment),
            tags: tags_from_hledger(&commodity.ctags),
            meta: Metadata::new(),
            source: None,
        }));
    }

    for price in &journal.jpricedirectives {
        entries.push(Entry::Price(super::PriceDirective {
            date: parse_date(&price.pddate, &mut report, "price.date"),
            commodity: price.pdcommodity.clone(),
            amount: amount_from_hledger(&price.pdamount, &mut report, "price.amount"),
            meta: Metadata::new(),
            source: Some(source_pos_to_span(&price.pdsourcepos)),
        }));
    }

    for txn in &journal.jtxns {
        entries.push(Entry::Transaction(transaction_from_hledger(
            txn,
            &mut report,
        )));
    }

    for modifier in &journal.jtxnmodifiers {
        entries.push(Entry::TransactionModifier(
            transaction_modifier_from_hledger(modifier, &mut report),
        ));
    }

    for periodic in &journal.jperiodictxns {
        entries.push(Entry::PeriodicTransaction(periodic_from_hledger(
            periodic,
            &mut report,
        )));
    }

    for entry in &journal.jtimeclockentries {
        entries.push(Entry::TimeclockEntry(timeclock_from_hledger(
            entry,
            &mut report,
        )));
    }

    report.push(
        ConversionIssueKind::Normalized,
        "hledger::Journal",
        "entry order lost because hledger JSON groups directives by type",
    );

    (
        super::Ledger {
            entries,
            meta: Metadata::new(),
        },
        report,
    )
}

pub fn to_hledger_journal(ledger: &super::Ledger) -> (Journal, ConversionReport) {
    let mut report = ConversionReport::default();
    let mut journal = Journal {
        jtxns: Vec::new(),
        jpricedirectives: Vec::new(),
        jperiodictxns: Vec::new(),
        jtxnmodifiers: Vec::new(),
        jtimeclockentries: Vec::new(),
        jdeclaredpayees: Vec::new(),
        jdeclaredtags: Vec::new(),
        jdeclaredaccounts: Vec::new(),
        jdeclaredcommodities: BTreeMap::new(),
        extra: BTreeMap::new(),
    };

    for entry in &ledger.entries {
        match entry {
            Entry::Account(account) => {
                journal.jdeclaredaccounts.push((
                    account.account.clone(),
                    AccountDeclarationInfo {
                        adicomment: account.comment.clone().unwrap_or_default(),
                        aditags: tags_to_hledger(&account.tags, &mut report, "account.tags"),
                        adideclarationorder: account.order.unwrap_or_default() as i64,
                        adisourcepos: source_span_start(account.source.as_ref()),
                    },
                ));
            }
            Entry::Payee(payee) => {
                journal.jdeclaredpayees.push((
                    payee.payee.clone(),
                    PayeeDeclarationInfo {
                        pdicomment: payee.comment.clone().unwrap_or_default(),
                        pditags: Vec::new(),
                    },
                ));
            }
            Entry::Tag(tag) => {
                journal.jdeclaredtags.push((
                    tag.name.clone(),
                    TagDeclarationInfo {
                        tdicomment: tag.comment.clone().unwrap_or_default(),
                    },
                ));
            }
            Entry::Commodity(commodity) => {
                if commodity.date.is_some() {
                    report.push(
                        ConversionIssueKind::Dropped,
                        format!("hledger::Commodity {}", commodity.symbol),
                        "hledger commodity declarations do not include a date; date dropped",
                    );
                }
                journal.jdeclaredcommodities.insert(
                    commodity.symbol.clone(),
                    Commodity {
                        csymbol: commodity.symbol.clone(),
                        cformat: commodity.format.as_ref().map(amount_style_to_hledger),
                        ccomment: commodity.comment.clone().unwrap_or_default(),
                        ctags: tags_to_hledger(&commodity.tags, &mut report, "commodity.tags"),
                    },
                );
            }
            Entry::Price(price) => {
                journal.jpricedirectives.push(PriceDirective {
                    pdsourcepos: source_span_start(price.source.as_ref()),
                    pddate: format_date(&price.date),
                    pdcommodity: price.commodity.clone(),
                    pdamount: amount_to_hledger(&price.amount, &mut report, "price.amount"),
                });
            }
            Entry::Transaction(txn) => {
                journal.jtxns.push(transaction_to_hledger(txn, &mut report));
            }
            Entry::TransactionModifier(modifier) => {
                journal
                    .jtxnmodifiers
                    .push(transaction_modifier_to_hledger(modifier, &mut report));
            }
            Entry::PeriodicTransaction(periodic) => {
                journal
                    .jperiodictxns
                    .push(periodic_to_hledger(periodic, &mut report));
            }
            Entry::TimeclockEntry(entry) => {
                journal
                    .jtimeclockentries
                    .push(timeclock_to_hledger(entry, &mut report));
            }
            Entry::Open(_)
            | Entry::Close(_)
            | Entry::Pad(_)
            | Entry::Balance(_)
            | Entry::Note(_)
            | Entry::Event(_)
            | Entry::Query(_)
            | Entry::Document(_)
            | Entry::Custom(_) => {
                report.push(
                    ConversionIssueKind::Dropped,
                    "hledger::Entry",
                    "directive has no hledger JSON equivalent",
                );
            }
        }
    }

    if !ledger.meta.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::Journal",
            "ledger-level metadata is not representable in hledger JSON",
        );
    }

    (journal, report)
}

fn transaction_from_hledger(
    txn: &Transaction,
    report: &mut ConversionReport,
) -> super::Transaction {
    if !txn.tcode.is_empty() && txn.tcode.trim().is_empty() {
        report.push(
            ConversionIssueKind::Normalized,
            "hledger::Transaction",
            "transaction code contained only whitespace; normalized to empty",
        );
    }

    super::Transaction {
        date: parse_date(&txn.tdate, report, "transaction.date"),
        date2: txn
            .tdate2
            .as_ref()
            .map(|date| parse_date(date, report, "transaction.date2")),
        status: status_from_hledger(&txn.tstatus),
        flag: None,
        code: string_to_option(&txn.tcode),
        payee: None,
        narration: None,
        description: string_to_option(&txn.tdescription),
        comment: string_to_option(&txn.tcomment),
        preceding_comment: string_to_option(&txn.tprecedingcomment),
        tags: tags_from_hledger(&txn.ttags),
        links: Vec::new(),
        postings: txn
            .tpostings
            .iter()
            .enumerate()
            .map(|(index, posting)| {
                posting_from_hledger(posting, report, &format!("transaction.postings[{index}]"))
            })
            .collect(),
        index: if txn.tindex == 0 {
            None
        } else {
            Some(txn.tindex as u64)
        },
        meta: Metadata::new(),
        source: Some(source_span_from_hledger(&txn.tsourcepos)),
    }
}

fn transaction_to_hledger(txn: &super::Transaction, report: &mut ConversionReport) -> Transaction {
    let description = description_for_hledger(txn, report);
    let status = status_to_hledger(txn.status.as_ref(), txn.flag.as_deref(), report);

    if !txn.links.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::Transaction",
            "transaction links are not representable in hledger JSON",
        );
    }

    if !txn.meta.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::Transaction",
            "transaction metadata is not representable in hledger JSON",
        );
    }

    Transaction {
        tindex: txn.index.unwrap_or_default() as i64,
        tprecedingcomment: txn.preceding_comment.clone().unwrap_or_default(),
        tsourcepos: source_span_to_hledger(txn.source.as_ref()),
        tdate: format_date(&txn.date),
        tdate2: txn.date2.as_ref().map(format_date),
        tstatus: status,
        tcode: txn.code.clone().unwrap_or_default(),
        tdescription: description,
        tcomment: txn.comment.clone().unwrap_or_default(),
        ttags: tags_to_hledger(&txn.tags, report, "transaction.tags"),
        tpostings: txn
            .postings
            .iter()
            .enumerate()
            .map(|(index, posting)| {
                posting_to_hledger(posting, report, &format!("transaction.postings[{index}]"))
            })
            .collect(),
    }
}

fn posting_from_hledger(
    posting: &Posting,
    report: &mut ConversionReport,
    context: &str,
) -> super::Posting {
    if posting.ptransaction_index.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("hledger::Posting {context}"),
            "posting parent transaction index dropped",
        );
    }
    if posting.poriginal.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("hledger::Posting {context}"),
            "original posting dropped",
        );
    }

    let has_missing = posting
        .pamount
        .iter()
        .any(|amount| amount.acommodity == "AUTO");
    if has_missing && posting.pamount.len() > 1 {
        report.push(
            ConversionIssueKind::Normalized,
            format!("hledger::Posting {context}"),
            "missing amount overrides other amounts; extra amounts dropped",
        );
    }

    super::Posting {
        account: posting.paccount.clone(),
        amount: if has_missing {
            None
        } else {
            Some(super::MixedAmount {
                amounts: posting
                    .pamount
                    .iter()
                    .enumerate()
                    .map(|(index, amount)| {
                        amount_from_hledger(amount, report, &format!("{context}.amount[{index}]"))
                    })
                    .collect(),
            })
        },
        lot_cost: None,
        cost_spec: None,
        price: None,
        status: status_from_hledger(&posting.pstatus),
        flag: None,
        tags: tags_from_hledger(&posting.ptags),
        links: Vec::new(),
        comment: Some(posting.pcomment.clone()),
        posting_type: posting_type_from_hledger(&posting.ptype),
        balance_assertion: posting
            .pbalanceassertion
            .as_ref()
            .map(|assertion| balance_assertion_from_hledger(assertion, report, context)),
        date: posting
            .pdate
            .as_ref()
            .map(|date| parse_date(date, report, context)),
        date2: posting
            .pdate2
            .as_ref()
            .map(|date| parse_date(date, report, context)),
        meta: Metadata::new(),
        source: None,
    }
}

fn posting_to_hledger(
    posting: &super::Posting,
    report: &mut ConversionReport,
    context: &str,
) -> Posting {
    let status = status_to_hledger(posting.status.as_ref(), posting.flag.as_deref(), report);
    let mut amounts = if let Some(mixed) = &posting.amount {
        mixed
            .amounts
            .iter()
            .enumerate()
            .map(|(index, amount)| {
                amount_to_hledger(amount, report, &format!("{context}.amount[{index}]"))
            })
            .collect::<Vec<_>>()
    } else {
        report.push(
            ConversionIssueKind::Normalized,
            format!("hledger::Posting {context}"),
            "missing amount represented using the AUTO marker",
        );
        vec![missing_amount()]
    };
    if amounts.is_empty() {
        report.push(
            ConversionIssueKind::Normalized,
            format!("hledger::Posting {context}"),
            "empty amount list represented using the AUTO marker",
        );
        amounts.push(missing_amount());
    }

    if let Some(price) = &posting.price {
        if amounts.len() == 1 {
            let amount = &mut amounts[0];
            if amount.acost.is_some() {
                report.push(
                    ConversionIssueKind::Dropped,
                    format!("hledger::Posting {context}"),
                    "posting has both price and amount cost; price dropped",
                );
            } else {
                amount.acost = Some(match price.price_type {
                    PriceType::Unit => AmountCost::UnitCost(Box::new(amount_to_hledger(
                        &price.amount,
                        report,
                        "posting.price",
                    ))),
                    PriceType::Total => AmountCost::TotalCost(Box::new(amount_to_hledger(
                        &price.amount,
                        report,
                        "posting.price",
                    ))),
                });
            }
        } else {
            report.push(
                ConversionIssueKind::Dropped,
                format!("hledger::Posting {context}"),
                "posting prices require a single amount; price dropped",
            );
        }
    }

    if let Some(lot_cost) = &posting.lot_cost {
        if amounts.len() == 1 {
            let amount = &mut amounts[0];
            if amount.acostbasis.is_some() {
                report.push(
                    ConversionIssueKind::Dropped,
                    format!("hledger::Posting {context}"),
                    "posting has both lot_cost and amount cost basis; lot_cost dropped",
                );
            } else {
                amount.acostbasis = Some(cost_basis_to_hledger_from_lot(lot_cost, report, context));
            }
        } else {
            report.push(
                ConversionIssueKind::Dropped,
                format!("hledger::Posting {context}"),
                "lot cost requires a single amount; lot_cost dropped",
            );
        }
    }

    if posting.cost_spec.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("hledger::Posting {context}"),
            "cost specifications are not representable in hledger JSON",
        );
    }

    if !posting.links.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("hledger::Posting {context}"),
            "posting links are not representable in hledger JSON",
        );
    }

    if !posting.meta.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("hledger::Posting {context}"),
            "posting metadata is not representable in hledger JSON",
        );
    }

    Posting {
        pdate: posting.date.as_ref().map(format_date),
        pdate2: posting.date2.as_ref().map(format_date),
        pstatus: status,
        paccount: posting.account.clone(),
        pamount: amounts,
        pcomment: posting.comment.clone().unwrap_or_default(),
        ptype: posting_type_to_hledger(&posting.posting_type),
        ptags: tags_to_hledger(&posting.tags, report, context),
        pbalanceassertion: posting
            .balance_assertion
            .as_ref()
            .map(|assertion| balance_assertion_to_hledger(assertion, report, context)),
        ptransaction_index: None,
        poriginal: None,
    }
}

fn transaction_modifier_from_hledger(
    modifier: &TransactionModifier,
    report: &mut ConversionReport,
) -> super::TransactionModifier {
    super::TransactionModifier {
        query: modifier.tmquerytxt.clone(),
        posting_rules: modifier
            .tmpostingrules
            .iter()
            .map(|rule| PostingRule {
                posting: posting_from_hledger(&rule.tmpr_posting, report, "txn_modifier.rule"),
                is_multiplier: rule.tmpr_is_multiplier,
            })
            .collect(),
        meta: Metadata::new(),
        source: None,
    }
}

fn transaction_modifier_to_hledger(
    modifier: &super::TransactionModifier,
    report: &mut ConversionReport,
) -> TransactionModifier {
    if !modifier.meta.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::TransactionModifier",
            "metadata is not representable in hledger JSON",
        );
    }
    TransactionModifier {
        tmquerytxt: modifier.query.clone(),
        tmpostingrules: modifier
            .posting_rules
            .iter()
            .map(|rule| TMPostingRule {
                tmpr_posting: posting_to_hledger(&rule.posting, report, "txn_modifier.rule"),
                tmpr_is_multiplier: rule.is_multiplier,
            })
            .collect(),
    }
}

fn periodic_from_hledger(
    periodic: &PeriodicTransaction,
    report: &mut ConversionReport,
) -> super::PeriodicTransaction {
    let interval = serde_json::to_string(&periodic.ptinterval).ok();
    let span = serde_json::to_string(&periodic.ptspan).ok();
    if interval.is_some() || span.is_some() {
        report.push(
            ConversionIssueKind::Normalized,
            "hledger::PeriodicTransaction",
            "interval/span stored as JSON strings",
        );
    }

    super::PeriodicTransaction {
        period_expression: periodic.ptperiodexpr.clone(),
        interval,
        span,
        status: status_from_hledger(&periodic.ptstatus),
        code: string_to_option(&periodic.ptcode),
        description: string_to_option(&periodic.ptdescription),
        comment: string_to_option(&periodic.ptcomment),
        tags: tags_from_hledger(&periodic.pttags),
        postings: periodic
            .ptpostings
            .iter()
            .enumerate()
            .map(|(index, posting)| {
                posting_from_hledger(posting, report, &format!("periodic.postings[{index}]"))
            })
            .collect(),
        meta: Metadata::new(),
        source: Some(source_span_from_hledger(&periodic.ptsourcepos)),
    }
}

fn periodic_to_hledger(
    periodic: &super::PeriodicTransaction,
    report: &mut ConversionReport,
) -> PeriodicTransaction {
    let interval = periodic
        .interval
        .as_ref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(serde_json::Value::Null);
    let span = periodic
        .span
        .as_ref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(serde_json::Value::Null);
    if periodic.interval.is_some() || periodic.span.is_some() {
        report.push(
            ConversionIssueKind::Normalized,
            "hledger::PeriodicTransaction",
            "interval/span JSON strings parsed for hledger output",
        );
    }

    PeriodicTransaction {
        ptperiodexpr: periodic.period_expression.clone(),
        ptinterval: interval,
        ptspan: span,
        ptsourcepos: source_span_to_hledger(periodic.source.as_ref()),
        ptstatus: status_to_hledger(periodic.status.as_ref(), None, report),
        ptcode: periodic.code.clone().unwrap_or_default(),
        ptdescription: periodic.description.clone().unwrap_or_default(),
        ptcomment: periodic.comment.clone().unwrap_or_default(),
        pttags: tags_to_hledger(&periodic.tags, report, "periodic.tags"),
        ptpostings: periodic
            .postings
            .iter()
            .enumerate()
            .map(|(index, posting)| {
                posting_to_hledger(posting, report, &format!("periodic.postings[{index}]"))
            })
            .collect(),
    }
}

fn timeclock_from_hledger(
    entry: &TimeclockEntry,
    report: &mut ConversionReport,
) -> super::TimeclockEntry {
    let datetime = parse_datetime(&entry.tldatetime, report, "timeclock.datetime");
    super::TimeclockEntry {
        code: timeclock_code_from_hledger(&entry.tlcode),
        datetime,
        account: entry.tlaccount.clone(),
        description: string_to_option(&entry.tldescription),
        comment: string_to_option(&entry.tlcomment),
        tags: tags_from_hledger(&entry.tltags),
        meta: Metadata::new(),
        source: Some(source_span_from_hledger(&SourceSpan(
            entry.tlsourcepos.clone(),
            entry.tlsourcepos.clone(),
        ))),
    }
}

fn timeclock_to_hledger(
    entry: &super::TimeclockEntry,
    report: &mut ConversionReport,
) -> TimeclockEntry {
    if entry.datetime.offset_minutes.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            "hledger::TimeclockEntry",
            "timeclock offsets are not representable in hledger JSON",
        );
    }
    TimeclockEntry {
        tlsourcepos: source_span_start(entry.source.as_ref()),
        tlcode: timeclock_code_to_hledger(&entry.code),
        tldatetime: format_datetime(&entry.datetime),
        tlaccount: entry.account.clone(),
        tldescription: entry.description.clone().unwrap_or_default(),
        tlcomment: entry.comment.clone().unwrap_or_default(),
        tltags: tags_to_hledger(&entry.tags, report, "timeclock.tags"),
    }
}

fn amount_from_hledger(
    amount: &Amount,
    report: &mut ConversionReport,
    context: &str,
) -> super::Amount {
    super::Amount {
        commodity: amount.acommodity.clone(),
        quantity: DecimalString(decimal_raw_to_string(&amount.aquantity)),
        style: amount.astyle.as_ref().map(amount_style_from_hledger),
        cost: amount
            .acost
            .as_ref()
            .map(|cost| amount_cost_from_hledger(cost, report, context)),
        cost_basis: amount
            .acostbasis
            .as_ref()
            .map(|basis| cost_basis_from_hledger(basis, report, context)),
    }
}

fn amount_to_hledger(
    amount: &super::Amount,
    report: &mut ConversionReport,
    context: &str,
) -> Amount {
    Amount {
        acommodity: amount.commodity.clone(),
        aquantity: decimal_string_to_raw(&amount.quantity, report, context),
        astyle: amount
            .style
            .as_ref()
            .map(amount_style_to_hledger)
            .or_else(|| Some(default_amount_style())),
        acost: amount
            .cost
            .as_ref()
            .map(|cost| amount_cost_to_hledger(cost, report, context)),
        acostbasis: amount
            .cost_basis
            .as_ref()
            .map(|basis| cost_basis_to_hledger_from_basis(basis, report, context)),
    }
}

fn amount_cost_from_hledger(
    cost: &AmountCost,
    report: &mut ConversionReport,
    context: &str,
) -> super::AmountCost {
    match cost {
        AmountCost::UnitCost(amount) => super::AmountCost::Unit(Box::new(amount_from_hledger(
            amount.as_ref(),
            report,
            context,
        ))),
        AmountCost::TotalCost(amount) => super::AmountCost::Total(Box::new(amount_from_hledger(
            amount.as_ref(),
            report,
            context,
        ))),
    }
}

fn amount_cost_to_hledger(
    cost: &super::AmountCost,
    report: &mut ConversionReport,
    context: &str,
) -> AmountCost {
    match cost {
        super::AmountCost::Unit(amount) => {
            AmountCost::UnitCost(Box::new(amount_to_hledger(amount, report, context)))
        }
        super::AmountCost::Total(amount) => {
            AmountCost::TotalCost(Box::new(amount_to_hledger(amount, report, context)))
        }
    }
}

fn cost_basis_from_hledger(
    basis: &CostBasis,
    report: &mut ConversionReport,
    context: &str,
) -> super::CostBasis {
    super::CostBasis {
        cost: basis
            .cb_cost
            .as_ref()
            .map(|amount| Box::new(amount_from_hledger(amount.as_ref(), report, context))),
        date: basis
            .cb_date
            .as_ref()
            .map(|date| parse_date(date, report, context)),
        label: basis.cb_label.clone(),
    }
}

fn cost_basis_to_hledger_from_lot(
    cost: &super::LotCost,
    report: &mut ConversionReport,
    context: &str,
) -> CostBasis {
    CostBasis {
        cb_cost: Some(Box::new(amount_to_hledger(&cost.amount, report, context))),
        cb_date: cost.date.as_ref().map(format_date),
        cb_label: cost.label.clone(),
    }
}

fn cost_basis_to_hledger_from_basis(
    cost: &super::CostBasis,
    report: &mut ConversionReport,
    context: &str,
) -> CostBasis {
    CostBasis {
        cb_cost: cost
            .cost
            .as_ref()
            .map(|amount| Box::new(amount_to_hledger(amount, report, context))),
        cb_date: cost.date.as_ref().map(format_date),
        cb_label: cost.label.clone(),
    }
}

fn balance_assertion_from_hledger(
    assertion: &BalanceAssertion,
    report: &mut ConversionReport,
    context: &str,
) -> super::BalanceAssertion {
    super::BalanceAssertion {
        amount: amount_from_hledger(&assertion.baamount, report, context),
        total: assertion.batotal,
        inclusive: assertion.bainclusive,
        source: Some(source_pos_from_hledger(&assertion.baposition)),
    }
}

fn balance_assertion_to_hledger(
    assertion: &super::BalanceAssertion,
    report: &mut ConversionReport,
    context: &str,
) -> BalanceAssertion {
    let source = assertion
        .source
        .as_ref()
        .map(source_pos_to_hledger)
        .unwrap_or_else(|| {
            report.push(
                ConversionIssueKind::Assumed,
                format!("hledger::BalanceAssertion {context}"),
                "missing source position defaulted",
            );
            SourcePos {
                source_name: String::new(),
                source_line: 1,
                source_column: 1,
            }
        });

    BalanceAssertion {
        baamount: amount_to_hledger(&assertion.amount, report, context),
        batotal: assertion.total,
        bainclusive: assertion.inclusive,
        baposition: source,
    }
}

fn amount_style_from_hledger(style: &AmountStyle) -> super::AmountStyle {
    super::AmountStyle {
        commodity_side: match style.ascommodityside {
            Side::L => super::CommoditySide::Left,
            Side::R => super::CommoditySide::Right,
        },
        commodity_spaced: style.ascommodityspaced,
        digit_groups: style.asdigitgroups.as_ref().map(|groups| match groups {
            DigitGroupStyle::DigitGroups(separator, groups) => super::DigitGroupStyle {
                separator: *separator,
                groups: groups.clone(),
            },
        }),
        decimal_mark: style.asdecimalmark,
        precision: match style.asprecision {
            AmountPrecision::Natural => super::AmountPrecision::Natural,
            AmountPrecision::Precision(number) => super::AmountPrecision::Precision(number),
        },
        rounding: match style.asrounding {
            Rounding::NoRounding => super::Rounding::None,
            Rounding::SoftRounding => super::Rounding::Soft,
            Rounding::HardRounding => super::Rounding::Hard,
            Rounding::AllRounding => super::Rounding::All,
        },
    }
}

fn amount_style_to_hledger(style: &super::AmountStyle) -> AmountStyle {
    AmountStyle {
        ascommodityside: match style.commodity_side {
            super::CommoditySide::Left => Side::L,
            super::CommoditySide::Right => Side::R,
        },
        ascommodityspaced: style.commodity_spaced,
        asdigitgroups: style
            .digit_groups
            .as_ref()
            .map(|groups| DigitGroupStyle::DigitGroups(groups.separator, groups.groups.clone())),
        asdecimalmark: style.decimal_mark,
        asprecision: match style.precision {
            super::AmountPrecision::Natural => AmountPrecision::Natural,
            super::AmountPrecision::Precision(number) => AmountPrecision::Precision(number),
        },
        asrounding: match style.rounding {
            super::Rounding::None => Rounding::NoRounding,
            super::Rounding::Soft => Rounding::SoftRounding,
            super::Rounding::Hard => Rounding::HardRounding,
            super::Rounding::All => Rounding::AllRounding,
        },
    }
}

fn default_amount_style() -> AmountStyle {
    AmountStyle {
        ascommodityside: Side::L,
        ascommodityspaced: false,
        asdigitgroups: None,
        asdecimalmark: Some('.'),
        asprecision: AmountPrecision::Precision(0),
        asrounding: Rounding::NoRounding,
    }
}

fn posting_type_from_hledger(posting_type: &PostingType) -> super::PostingType {
    match posting_type {
        PostingType::RegularPosting => super::PostingType::Regular,
        PostingType::VirtualPosting => super::PostingType::Virtual,
        PostingType::BalancedVirtualPosting => super::PostingType::BalancedVirtual,
    }
}

fn posting_type_to_hledger(posting_type: &super::PostingType) -> PostingType {
    match posting_type {
        super::PostingType::Regular => PostingType::RegularPosting,
        super::PostingType::Virtual => PostingType::VirtualPosting,
        super::PostingType::BalancedVirtual => PostingType::BalancedVirtualPosting,
    }
}

fn status_from_hledger(status: &Status) -> Option<super::Status> {
    match status {
        Status::Unmarked => None,
        Status::Pending => Some(super::Status::Pending),
        Status::Cleared => Some(super::Status::Cleared),
    }
}

fn status_to_hledger(
    status: Option<&super::Status>,
    flag: Option<&str>,
    report: &mut ConversionReport,
) -> Status {
    if let Some(status) = status {
        return match status {
            super::Status::Unmarked => Status::Unmarked,
            super::Status::Pending => Status::Pending,
            super::Status::Cleared => Status::Cleared,
        };
    }

    match flag {
        Some("!") => {
            report.push(
                ConversionIssueKind::Assumed,
                "hledger::Status",
                "mapped flag ! to Pending status",
            );
            Status::Pending
        }
        Some("*") => {
            report.push(
                ConversionIssueKind::Assumed,
                "hledger::Status",
                "mapped flag * to Cleared status",
            );
            Status::Cleared
        }
        Some(_) => {
            report.push(
                ConversionIssueKind::Dropped,
                "hledger::Status",
                "transaction/posting flags are not representable in hledger JSON",
            );
            Status::Unmarked
        }
        None => Status::Unmarked,
    }
}

fn timeclock_code_from_hledger(code: &TimeclockCode) -> super::TimeclockCode {
    match code {
        TimeclockCode::SetBalance => super::TimeclockCode::SetBalance,
        TimeclockCode::SetRequiredHours => super::TimeclockCode::SetRequiredHours,
        TimeclockCode::In => super::TimeclockCode::In,
        TimeclockCode::Out => super::TimeclockCode::Out,
        TimeclockCode::FinalOut => super::TimeclockCode::FinalOut,
    }
}

fn timeclock_code_to_hledger(code: &super::TimeclockCode) -> TimeclockCode {
    match code {
        super::TimeclockCode::SetBalance => TimeclockCode::SetBalance,
        super::TimeclockCode::SetRequiredHours => TimeclockCode::SetRequiredHours,
        super::TimeclockCode::In => TimeclockCode::In,
        super::TimeclockCode::Out => TimeclockCode::Out,
        super::TimeclockCode::FinalOut => TimeclockCode::FinalOut,
    }
}

fn tags_from_hledger(tags: &[HledgerTag]) -> Vec<super::Tag> {
    tags.iter()
        .map(|(name, value)| Tag {
            name: name.clone(),
            value: if value.is_empty() {
                None
            } else {
                Some(value.clone())
            },
            hidden: name.starts_with('_'),
        })
        .collect()
}

fn tags_to_hledger(
    tags: &[super::Tag],
    report: &mut ConversionReport,
    context: &str,
) -> Vec<HledgerTag> {
    tags.iter()
        .map(|tag| {
            let mut name = tag.name.clone();
            if tag.hidden && !name.starts_with('_') {
                report.push(
                    ConversionIssueKind::Assumed,
                    format!("hledger::Tags {context}"),
                    "hidden tag encoded by prefixing underscore",
                );
                name = format!("_{}", name);
            }
            (name, tag.value.clone().unwrap_or_default())
        })
        .collect()
}

fn string_to_option(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn description_for_hledger(txn: &super::Transaction, report: &mut ConversionReport) -> String {
    if let Some(description) = &txn.description {
        if txn
            .narration
            .as_ref()
            .is_some_and(|narration| narration != description)
        {
            report.push(
                ConversionIssueKind::Dropped,
                "hledger::Transaction",
                "transaction narration dropped in favor of description",
            );
        }
        if txn.payee.is_some() {
            report.push(
                ConversionIssueKind::Dropped,
                "hledger::Transaction",
                "transaction payee dropped in favor of description",
            );
        }
        return description.clone();
    }

    if let Some(narration) = &txn.narration {
        if txn.payee.is_some() {
            report.push(
                ConversionIssueKind::Dropped,
                "hledger::Transaction",
                "transaction payee dropped in favor of narration",
            );
        }
        return narration.clone();
    }

    if let Some(payee) = &txn.payee {
        return payee.clone();
    }

    String::new()
}

fn parse_date(value: &str, report: &mut ConversionReport, context: &str) -> Date {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(year), Ok(month), Ok(day)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<u8>(),
            parts[2].parse::<u8>(),
        ) {
            return Date { year, month, day };
        }
    }
    report.push(
        ConversionIssueKind::Assumed,
        format!("hledger::Date {context}"),
        "invalid date format; defaulting to 1970-01-01",
    );
    Date {
        year: 1970,
        month: 1,
        day: 1,
    }
}

fn format_date(date: &Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

fn parse_datetime(value: &str, report: &mut ConversionReport, context: &str) -> DateTime {
    let (date_part, time_part) = value
        .split_once('T')
        .or_else(|| value.split_once(' '))
        .unwrap_or((value, "00:00:00"));
    let date = parse_date(date_part, report, context);
    let time = parse_time(time_part, report, context);
    DateTime {
        date,
        time,
        offset_minutes: None,
    }
}

fn format_datetime(value: &DateTime) -> String {
    let time = &value.time;
    if time.nanosecond > 0 {
        format!(
            "{} {:02}:{:02}:{:02}.{:09}",
            format_date(&value.date),
            time.hour,
            time.minute,
            time.second,
            time.nanosecond
        )
    } else {
        format!(
            "{} {:02}:{:02}:{:02}",
            format_date(&value.date),
            time.hour,
            time.minute,
            time.second
        )
    }
}

fn parse_time(value: &str, report: &mut ConversionReport, context: &str) -> Time {
    let (main, fractional) = value.split_once('.').unwrap_or((value, ""));
    let parts: Vec<_> = main.split(':').collect();
    if parts.len() >= 2 {
        let hour = parts[0].parse::<u8>().unwrap_or(0);
        let minute = parts[1].parse::<u8>().unwrap_or(0);
        let second = parts
            .get(2)
            .and_then(|val| val.parse::<u8>().ok())
            .unwrap_or(0);
        let nanosecond = parse_fractional_nanos(fractional);
        return Time {
            hour,
            minute,
            second,
            nanosecond,
        };
    }
    report.push(
        ConversionIssueKind::Assumed,
        format!("hledger::Time {context}"),
        "invalid time format; defaulting to 00:00:00",
    );
    Time {
        hour: 0,
        minute: 0,
        second: 0,
        nanosecond: 0,
    }
}

fn parse_fractional_nanos(value: &str) -> u32 {
    if value.is_empty() {
        return 0;
    }
    let mut digits = value.chars().take(9).collect::<String>();
    while digits.len() < 9 {
        digits.push('0');
    }
    digits.parse::<u32>().unwrap_or(0)
}

fn decimal_raw_to_string(raw: &DecimalRaw) -> String {
    let mantissa = raw.decimal_mantissa.to_string();
    let (sign, digits) = if let Some(rest) = mantissa.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", mantissa.as_str())
    };
    let places = raw.decimal_places as usize;
    if places == 0 {
        return format!("{sign}{digits}");
    }

    let mut digits = digits.to_string();
    if digits.len() <= places {
        let zeros = "0".repeat(places - digits.len() + 1);
        digits = format!("{zeros}{digits}");
    }
    let split_at = digits.len() - places;
    format!("{sign}{}.{}", &digits[..split_at], &digits[split_at..])
}

fn decimal_string_to_raw(
    value: &DecimalString,
    report: &mut ConversionReport,
    context: &str,
) -> DecimalRaw {
    let raw = value.0.trim();
    let (sign, rest) = if let Some(rest) = raw.strip_prefix('-') {
        ("-", rest)
    } else if let Some(rest) = raw.strip_prefix('+') {
        ("", rest)
    } else {
        ("", raw)
    };
    let parts: Vec<_> = rest.split('.').collect();
    let (int_part, frac_part) = if parts.len() == 2 {
        (parts[0], parts[1])
    } else {
        (rest, "")
    };
    let mut digits = format!("{int_part}{frac_part}");
    if digits.is_empty() {
        report.push(
            ConversionIssueKind::Assumed,
            format!("hledger::Decimal {context}"),
            "invalid decimal string; defaulting to 0",
        );
        digits.push('0');
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mantissa = format!("{sign}{digits}");
    let decimal_places = frac_part.len() as u32;
    let floating_point = raw.parse::<f64>().unwrap_or(0.0);

    if raw.parse::<f64>().is_err() {
        report.push(
            ConversionIssueKind::Normalized,
            format!("hledger::Decimal {context}"),
            "floating-point approximation failed; set to 0",
        );
    }

    DecimalRaw {
        decimal_places,
        decimal_mantissa: Number::from_string_unchecked(mantissa),
        floating_point,
    }
}

fn decimal_raw_zero() -> DecimalRaw {
    DecimalRaw {
        decimal_places: 0,
        decimal_mantissa: Number::from(0),
        floating_point: 0.0,
    }
}

fn missing_amount() -> Amount {
    Amount {
        acommodity: "AUTO".to_string(),
        aquantity: decimal_raw_zero(),
        astyle: Some(default_amount_style()),
        acost: None,
        acostbasis: None,
    }
}

fn source_pos_from_hledger(pos: &SourcePos) -> super::SourcePos {
    super::SourcePos {
        file: pos.source_name.clone(),
        line: pos.source_line,
        column: pos.source_column,
    }
}

fn source_span_from_hledger(span: &SourceSpan) -> super::SourceSpan {
    super::SourceSpan {
        start: source_pos_from_hledger(&span.0),
        end: source_pos_from_hledger(&span.1),
    }
}

fn source_pos_to_span(pos: &SourcePos) -> super::SourceSpan {
    let mapped = source_pos_from_hledger(pos);
    super::SourceSpan {
        start: mapped.clone(),
        end: mapped,
    }
}

fn source_pos_to_hledger(pos: &super::SourcePos) -> SourcePos {
    SourcePos {
        source_name: pos.file.clone(),
        source_line: pos.line,
        source_column: pos.column,
    }
}

fn source_span_start(span: Option<&super::SourceSpan>) -> SourcePos {
    span.map(|span| SourcePos {
        source_name: span.start.file.clone(),
        source_line: span.start.line,
        source_column: span.start.column,
    })
    .unwrap_or(SourcePos {
        source_name: String::new(),
        source_line: 1,
        source_column: 1,
    })
}

fn source_span_to_hledger(span: Option<&super::SourceSpan>) -> SourceSpan {
    let start = source_span_start(span);
    SourceSpan(start.clone(), SourcePos { ..start })
}
