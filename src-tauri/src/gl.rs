use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type AccountName = String;
pub type CommoditySymbol = String;
pub type Currency = String;
pub type Link = String;
pub type Payee = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecimalString(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateTime {
    pub date: Date,
    pub time: Time,
    pub offset_minutes: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePos {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: SourcePos,
    pub end: SourcePos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommoditySide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigitGroupStyle {
    pub separator: char,
    pub groups: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountPrecision {
    Natural,
    Precision(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    None,
    Soft,
    Hard,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmountStyle {
    pub commodity_side: CommoditySide,
    pub commodity_spaced: bool,
    pub digit_groups: Option<DigitGroupStyle>,
    pub decimal_mark: Option<char>,
    pub precision: AmountPrecision,
    pub rounding: Rounding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub value: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Unmarked,
    Pending,
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostingType {
    Regular,
    Virtual,
    BalancedVirtual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostBasis {
    pub cost: Option<Box<Amount>>,
    pub date: Option<Date>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountCost {
    Unit(Box<Amount>),
    Total(Box<Amount>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amount {
    pub commodity: CommoditySymbol,
    pub quantity: DecimalString,
    pub style: Option<AmountStyle>,
    pub cost: Option<AmountCost>,
    pub cost_basis: Option<CostBasis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedAmount {
    pub amounts: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotCost {
    pub amount: Amount,
    pub date: Option<Date>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostSpec {
    pub per_unit: Option<DecimalString>,
    pub total: Option<DecimalString>,
    pub currency: Option<CommoditySymbol>,
    pub date: Option<Date>,
    pub label: Option<String>,
    pub merge: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceType {
    Unit,
    Total,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceSpec {
    pub amount: Amount,
    pub price_type: PriceType,
}

pub type Metadata = BTreeMap<String, MetaValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaValue {
    String(String),
    Bool(bool),
    Number(DecimalString),
    Date(Date),
    DateTime(DateTime),
    Amount(Amount),
    Account(AccountName),
    Currency(Currency),
    Tag(Tag),
    Link(Link),
    List(Vec<MetaValue>),
    Map(BTreeMap<String, MetaValue>),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Booking {
    Strict,
    StrictWithSize,
    None,
    Average,
    Fifo,
    Lifo,
    Hifo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceAssertion {
    pub amount: Amount,
    pub total: bool,
    pub inclusive: bool,
    pub source: Option<SourcePos>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: AccountName,
    pub amount: Option<MixedAmount>,
    pub lot_cost: Option<LotCost>,
    pub cost_spec: Option<CostSpec>,
    pub price: Option<PriceSpec>,
    pub status: Option<Status>,
    pub flag: Option<String>,
    pub tags: Vec<Tag>,
    pub links: Vec<Link>,
    pub comment: Option<String>,
    pub posting_type: PostingType,
    pub balance_assertion: Option<BalanceAssertion>,
    pub date: Option<Date>,
    pub date2: Option<Date>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub date: Date,
    pub date2: Option<Date>,
    pub status: Option<Status>,
    pub flag: Option<String>,
    pub code: Option<String>,
    pub payee: Option<Payee>,
    pub narration: Option<String>,
    pub description: Option<String>,
    pub comment: Option<String>,
    pub preceding_comment: Option<String>,
    pub tags: Vec<Tag>,
    pub links: Vec<Link>,
    pub postings: Vec<Posting>,
    pub index: Option<u64>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Open {
    pub date: Date,
    pub account: AccountName,
    pub currencies: Option<Vec<Currency>>,
    pub booking: Option<Booking>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Close {
    pub date: Date,
    pub account: AccountName,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDirective {
    pub account: AccountName,
    pub comment: Option<String>,
    pub tags: Vec<Tag>,
    pub order: Option<u64>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommodityDirective {
    pub date: Option<Date>,
    pub symbol: CommoditySymbol,
    pub format: Option<AmountStyle>,
    pub comment: Option<String>,
    pub tags: Vec<Tag>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pad {
    pub date: Date,
    pub account: AccountName,
    pub source_account: AccountName,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    pub date: Date,
    pub account: AccountName,
    pub amount: Amount,
    pub tolerance: Option<DecimalString>,
    pub diff_amount: Option<Amount>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub date: Date,
    pub account: AccountName,
    pub comment: String,
    pub tags: Vec<Tag>,
    pub links: Vec<Link>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub date: Date,
    pub event_type: String,
    pub description: String,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub date: Date,
    pub name: String,
    pub query: String,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceDirective {
    pub date: Date,
    pub commodity: CommoditySymbol,
    pub amount: Amount,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub date: Date,
    pub account: AccountName,
    pub filename: String,
    pub tags: Vec<Tag>,
    pub links: Vec<Link>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custom {
    pub date: Date,
    pub custom_type: String,
    pub values: Vec<MetaValue>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionModifier {
    pub query: String,
    pub posting_rules: Vec<PostingRule>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostingRule {
    pub posting: Posting,
    pub is_multiplier: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicTransaction {
    pub period_expression: String,
    pub interval: Option<String>,
    pub span: Option<String>,
    pub status: Option<Status>,
    pub code: Option<String>,
    pub description: Option<String>,
    pub comment: Option<String>,
    pub tags: Vec<Tag>,
    pub postings: Vec<Posting>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeclockCode {
    SetBalance,
    SetRequiredHours,
    In,
    Out,
    FinalOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeclockEntry {
    pub code: TimeclockCode,
    pub datetime: DateTime,
    pub account: AccountName,
    pub description: Option<String>,
    pub comment: Option<String>,
    pub tags: Vec<Tag>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDirective {
    pub name: String,
    pub comment: Option<String>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayeeDirective {
    pub payee: Payee,
    pub comment: Option<String>,
    pub meta: Metadata,
    pub source: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Entry {
    Open(Open),
    Close(Close),
    Account(AccountDirective),
    Commodity(CommodityDirective),
    Pad(Pad),
    Balance(Balance),
    Transaction(Transaction),
    Note(Note),
    Event(Event),
    Query(Query),
    Price(PriceDirective),
    Document(Document),
    Custom(Custom),
    TransactionModifier(TransactionModifier),
    PeriodicTransaction(PeriodicTransaction),
    TimeclockEntry(TimeclockEntry),
    Tag(TagDirective),
    Payee(PayeeDirective),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub entries: Vec<Entry>,
    pub meta: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionIssueKind {
    Dropped,
    Normalized,
    Unsupported,
    Assumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionIssue {
    pub kind: ConversionIssueKind,
    pub context: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConversionReport {
    pub issues: Vec<ConversionIssue>,
}

impl ConversionReport {
    pub fn push(
        &mut self,
        kind: ConversionIssueKind,
        context: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.issues.push(ConversionIssue {
            kind,
            context: context.into(),
            detail: detail.into(),
        });
    }
}

pub mod beancount;
pub mod hledger;

#[cfg(test)]
mod tests;
