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

#[cfg(test)]
mod tests;
