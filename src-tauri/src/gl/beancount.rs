use super::*;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub entries: Vec<Directive>,
    pub meta: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Open {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
    pub currencies: Option<Vec<Currency>>,
    pub booking: Option<Booking>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Close {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commodity {
    pub meta: Metadata,
    pub date: Date,
    pub currency: Currency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pad {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
    pub source_account: AccountName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
    pub amount: Amount,
    pub tolerance: Option<DecimalString>,
    pub diff_amount: Option<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostingCost {
    Cost(LotCost),
    CostSpec(CostSpec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: AccountName,
    pub units: Option<Amount>,
    pub cost: Option<PostingCost>,
    pub price: Option<Amount>,
    pub flag: Option<String>,
    pub meta: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub meta: Metadata,
    pub date: Date,
    pub flag: Option<String>,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub tags: BTreeSet<String>,
    pub links: BTreeSet<String>,
    pub postings: Vec<Posting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
    pub comment: String,
    pub tags: Option<BTreeSet<String>>,
    pub links: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub meta: Metadata,
    pub date: Date,
    pub event_type: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub meta: Metadata,
    pub date: Date,
    pub name: String,
    pub query_string: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    pub meta: Metadata,
    pub date: Date,
    pub currency: Currency,
    pub amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub meta: Metadata,
    pub date: Date,
    pub account: AccountName,
    pub filename: String,
    pub tags: Option<BTreeSet<String>>,
    pub links: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custom {
    pub meta: Metadata,
    pub date: Date,
    pub custom_type: String,
    pub values: Vec<MetaValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Directive {
    Open(Open),
    Close(Close),
    Commodity(Commodity),
    Pad(Pad),
    Balance(Balance),
    Transaction(Transaction),
    Note(Note),
    Event(Event),
    Query(Query),
    Price(Price),
    Document(Document),
    Custom(Custom),
}

pub fn to_beancount(ledger: &super::Ledger) -> (Ledger, ConversionReport) {
    let mut report = ConversionReport::default();
    let mut entries = Vec::new();
    for entry in &ledger.entries {
        entries.extend(to_beancount_entry(entry, &mut report));
    }
    (
        Ledger {
            entries,
            meta: ledger.meta.clone(),
        },
        report,
    )
}

pub fn from_beancount(ledger: &Ledger) -> (super::Ledger, ConversionReport) {
    let mut report = ConversionReport::default();
    let mut entries = Vec::new();
    for entry in &ledger.entries {
        if let Some(mapped) = from_beancount_entry(entry, &mut report) {
            entries.push(mapped);
        }
    }
    (
        super::Ledger {
            entries,
            meta: ledger.meta.clone(),
        },
        report,
    )
}

fn to_beancount_entry(entry: &Entry, report: &mut ConversionReport) -> Vec<Directive> {
    match entry {
        Entry::Open(open) => vec![Directive::Open(Open {
            meta: open.meta.clone(),
            date: open.date.clone(),
            account: open.account.clone(),
            currencies: open.currencies.clone(),
            booking: open.booking.clone(),
        })],
        Entry::Close(close) => vec![Directive::Close(Close {
            meta: close.meta.clone(),
            date: close.date.clone(),
            account: close.account.clone(),
        })],
        Entry::Commodity(commodity) => {
            let date = match &commodity.date {
                Some(date) => date.clone(),
                None => {
                    report.push(
                        ConversionIssueKind::Dropped,
                        "beancount::Commodity",
                        "commodity directives require a date; none provided",
                    );
                    return Vec::new();
                }
            };
            vec![Directive::Commodity(Commodity {
                meta: commodity.meta.clone(),
                date,
                currency: commodity.symbol.clone(),
            })]
        }
        Entry::Pad(pad) => vec![Directive::Pad(Pad {
            meta: pad.meta.clone(),
            date: pad.date.clone(),
            account: pad.account.clone(),
            source_account: pad.source_account.clone(),
        })],
        Entry::Balance(balance) => vec![Directive::Balance(Balance {
            meta: balance.meta.clone(),
            date: balance.date.clone(),
            account: balance.account.clone(),
            amount: amount_for_beancount(&balance.amount, report, "balance.amount"),
            tolerance: balance.tolerance.clone(),
            diff_amount: balance
                .diff_amount
                .as_ref()
                .map(|amount| amount_for_beancount(amount, report, "balance.diff_amount")),
        })],
        Entry::Transaction(txn) => to_beancount_transaction_entries(txn, report),
        Entry::Note(note) => vec![Directive::Note(Note {
            meta: note.meta.clone(),
            date: note.date.clone(),
            account: note.account.clone(),
            comment: note.comment.clone(),
            tags: tags_to_set_opt(&note.tags, report, "note.tags"),
            links: links_to_set_opt(&note.links, report, "note.links"),
        })],
        Entry::Event(event) => vec![Directive::Event(Event {
            meta: event.meta.clone(),
            date: event.date.clone(),
            event_type: event.event_type.clone(),
            description: event.description.clone(),
        })],
        Entry::Query(query) => vec![Directive::Query(Query {
            meta: query.meta.clone(),
            date: query.date.clone(),
            name: query.name.clone(),
            query_string: query.query.clone(),
        })],
        Entry::Price(price) => vec![Directive::Price(Price {
            meta: price.meta.clone(),
            date: price.date.clone(),
            currency: price.commodity.clone(),
            amount: amount_for_beancount(&price.amount, report, "price.amount"),
        })],
        Entry::Document(document) => vec![Directive::Document(Document {
            meta: document.meta.clone(),
            date: document.date.clone(),
            account: document.account.clone(),
            filename: document.filename.clone(),
            tags: tags_to_set_opt(&document.tags, report, "document.tags"),
            links: links_to_set_opt(&document.links, report, "document.links"),
        })],
        Entry::Custom(custom) => vec![Directive::Custom(Custom {
            meta: custom.meta.clone(),
            date: custom.date.clone(),
            custom_type: custom.custom_type.clone(),
            values: custom.values.clone(),
        })],
        Entry::Account(_)
        | Entry::TransactionModifier(_)
        | Entry::PeriodicTransaction(_)
        | Entry::TimeclockEntry(_)
        | Entry::Tag(_)
        | Entry::Payee(_) => {
            report.push(
                ConversionIssueKind::Dropped,
                "beancount::Directive",
                "directive has no beancount equivalent",
            );
            Vec::new()
        }
    }
}

fn from_beancount_entry(entry: &Directive, report: &mut ConversionReport) -> Option<Entry> {
    match entry {
        Directive::Open(open) => Some(Entry::Open(super::Open {
            date: open.date.clone(),
            account: open.account.clone(),
            currencies: open.currencies.clone(),
            booking: open.booking.clone(),
            meta: open.meta.clone(),
            source: None,
        })),
        Directive::Close(close) => Some(Entry::Close(super::Close {
            date: close.date.clone(),
            account: close.account.clone(),
            meta: close.meta.clone(),
            source: None,
        })),
        Directive::Commodity(commodity) => Some(Entry::Commodity(super::CommodityDirective {
            date: Some(commodity.date.clone()),
            symbol: commodity.currency.clone(),
            format: None,
            comment: None,
            tags: Vec::new(),
            meta: commodity.meta.clone(),
            source: None,
        })),
        Directive::Pad(pad) => Some(Entry::Pad(super::Pad {
            date: pad.date.clone(),
            account: pad.account.clone(),
            source_account: pad.source_account.clone(),
            meta: pad.meta.clone(),
            source: None,
        })),
        Directive::Balance(balance) => Some(Entry::Balance(super::Balance {
            date: balance.date.clone(),
            account: balance.account.clone(),
            amount: balance.amount.clone(),
            tolerance: balance.tolerance.clone(),
            diff_amount: balance.diff_amount.clone(),
            meta: balance.meta.clone(),
            source: None,
        })),
        Directive::Transaction(txn) => {
            Some(Entry::Transaction(from_beancount_transaction(txn, report)))
        }
        Directive::Note(note) => Some(Entry::Note(super::Note {
            date: note.date.clone(),
            account: note.account.clone(),
            comment: note.comment.clone(),
            tags: set_to_tags(note.tags.as_ref()),
            links: set_to_links(note.links.as_ref()),
            meta: note.meta.clone(),
            source: None,
        })),
        Directive::Event(event) => Some(Entry::Event(super::Event {
            date: event.date.clone(),
            event_type: event.event_type.clone(),
            description: event.description.clone(),
            meta: event.meta.clone(),
            source: None,
        })),
        Directive::Query(query) => Some(Entry::Query(super::Query {
            date: query.date.clone(),
            name: query.name.clone(),
            query: query.query_string.clone(),
            meta: query.meta.clone(),
            source: None,
        })),
        Directive::Price(price) => Some(Entry::Price(super::PriceDirective {
            date: price.date.clone(),
            commodity: price.currency.clone(),
            amount: price.amount.clone(),
            meta: price.meta.clone(),
            source: None,
        })),
        Directive::Document(document) => Some(Entry::Document(super::Document {
            date: document.date.clone(),
            account: document.account.clone(),
            filename: document.filename.clone(),
            tags: set_to_tags(document.tags.as_ref()),
            links: set_to_links(document.links.as_ref()),
            meta: document.meta.clone(),
            source: None,
        })),
        Directive::Custom(custom) => Some(Entry::Custom(super::Custom {
            date: custom.date.clone(),
            custom_type: custom.custom_type.clone(),
            values: custom.values.clone(),
            meta: custom.meta.clone(),
            source: None,
        })),
    }
}

fn to_beancount_transaction_entries(
    txn: &super::Transaction,
    report: &mut ConversionReport,
) -> Vec<Directive> {
    let mut postings = Vec::new();
    let mut missing_count = 0;
    for (index, posting) in txn.postings.iter().enumerate() {
        let context = format!("posting[{index}]");
        let group_id = format!("{index}");
        let mapped = to_beancount_postings(posting, report, &context, &group_id);
        for mapped_posting in &mapped {
            if mapped_posting.units.is_none() {
                missing_count += 1;
            }
        }
        postings.extend(mapped);
    }

    if missing_count > 1 {
        report.push(
            ConversionIssueKind::Dropped,
            "beancount::Transaction",
            "transactions may only have one missing amount; stored as custom",
        );
        return vec![Directive::Custom(custom_from_transaction(txn, report))];
    }

    if postings.is_empty() {
        report.push(
            ConversionIssueKind::Dropped,
            "beancount::Transaction",
            "transaction had no postings after conversion; stored as custom",
        );
        return vec![Directive::Custom(custom_from_transaction(txn, report))];
    }

    let mut meta = txn.meta.clone();
    if let Some(date2) = &txn.date2 {
        meta.insert(
            "gl_transaction_date2".to_string(),
            MetaValue::String(format_date(date2)),
        );
    }
    if let Some(status) = &txn.status {
        meta.insert(
            "gl_transaction_status".to_string(),
            MetaValue::String(format!("{status:?}").to_lowercase()),
        );
    }
    if let Some(code) = &txn.code {
        meta.insert(
            "gl_transaction_code".to_string(),
            MetaValue::String(code.clone()),
        );
    }
    if let Some(description) = &txn.description {
        meta.insert(
            "gl_transaction_description".to_string(),
            MetaValue::String(description.clone()),
        );
    }
    if let Some(comment) = &txn.comment {
        meta.insert(
            "gl_transaction_comment".to_string(),
            MetaValue::String(comment.clone()),
        );
    }
    if let Some(comment) = &txn.preceding_comment {
        meta.insert(
            "gl_transaction_preceding_comment".to_string(),
            MetaValue::String(comment.clone()),
        );
    }
    if let Some(index) = txn.index {
        meta.insert(
            "gl_transaction_index".to_string(),
            MetaValue::String(index.to_string()),
        );
    }
    if let Some(value) = json_string(&txn.tags, report, "transaction.tags") {
        meta.insert("gl_transaction_tags".to_string(), MetaValue::String(value));
    }

    vec![Directive::Transaction(Transaction {
        meta,
        date: txn.date.clone(),
        flag: txn.flag.clone(),
        payee: txn.payee.clone(),
        narration: txn.narration.clone(),
        tags: tags_to_set(&txn.tags, report, "transaction.tags"),
        links: links_to_set(&txn.links, report, "transaction.links"),
        postings,
    })]
}

fn from_beancount_transaction(
    txn: &Transaction,
    report: &mut ConversionReport,
) -> super::Transaction {
    let postings = merge_postings_from_beancount(txn, report);
    super::Transaction {
        date: txn.date.clone(),
        date2: meta_date(&txn.meta, "gl_transaction_date2"),
        status: meta_status(&txn.meta, "gl_transaction_status"),
        flag: txn.flag.clone(),
        code: meta_string(&txn.meta, "gl_transaction_code"),
        payee: txn.payee.clone(),
        narration: txn.narration.clone(),
        description: meta_string(&txn.meta, "gl_transaction_description"),
        comment: meta_string(&txn.meta, "gl_transaction_comment"),
        preceding_comment: meta_string(&txn.meta, "gl_transaction_preceding_comment"),
        tags: meta_tags(&txn.meta, &txn.tags, report),
        links: set_to_links(Some(&txn.links)),
        postings,
        index: meta_u64(&txn.meta, "gl_transaction_index"),
        meta: txn.meta.clone(),
        source: None,
    }
}

fn to_beancount_postings(
    posting: &super::Posting,
    report: &mut ConversionReport,
    context: &str,
    group_id: &str,
) -> Vec<Posting> {
    let mut results = Vec::new();
    let amounts = posting
        .amount
        .as_ref()
        .map(|mixed| mixed.amounts.as_slice())
        .unwrap_or(&[]);
    let needs_backup = posting_requires_backup(posting, amounts.len());

    if amounts.is_empty() {
        let meta = build_posting_meta(posting, report, group_id, 0, needs_backup);
        results.push(Posting {
            account: posting.account.clone(),
            units: None,
            cost: None,
            price: None,
            flag: posting.flag.clone(),
            meta: meta_to_option(meta),
        });
        return results;
    }

    for (amount_index, amount) in amounts.iter().enumerate() {
        let mut meta = build_posting_meta(posting, report, group_id, amount_index, needs_backup);
        let units = amount_for_beancount(amount, report, "posting.amount");
        let cost = if amounts.len() == 1 {
            posting_cost_for_beancount(posting, report, context)
        } else {
            if posting.lot_cost.is_some() || posting.cost_spec.is_some() {
                report.push(
                    ConversionIssueKind::Dropped,
                    format!("beancount::Posting {context}"),
                    "cost data cannot be mapped when posting has multiple amounts; preserved in metadata",
                );
            }
            None
        };
        let price = if amounts.len() == 1 {
            posting_price_for_beancount(posting, report, context, &mut meta)
        } else {
            None
        };
        results.push(Posting {
            account: posting.account.clone(),
            units: Some(units),
            cost,
            price,
            flag: posting.flag.clone(),
            meta: meta_to_option(meta),
        });
    }

    results
}

fn from_beancount_posting(
    posting: &Posting,
    report: &mut ConversionReport,
    context: &str,
) -> super::Posting {
    let amount = posting.units.as_ref().map(|unit| MixedAmount {
        amounts: vec![unit.clone()],
    });

    let (lot_cost, cost_spec) = match &posting.cost {
        Some(PostingCost::Cost(cost)) => (Some(cost.clone()), None),
        Some(PostingCost::CostSpec(spec)) => (None, Some(spec.clone())),
        None => (None, None),
    };

    let price = posting.price.as_ref().map(|amount| {
        report.push(
            ConversionIssueKind::Assumed,
            format!("beancount::Posting {context}"),
            "beancount posting prices do not distinguish unit vs total",
        );
        PriceSpec {
            amount: amount.clone(),
            price_type: PriceType::Unit,
        }
    });

    super::Posting {
        account: posting.account.clone(),
        amount,
        lot_cost,
        cost_spec,
        price,
        status: None,
        flag: posting.flag.clone(),
        tags: Vec::new(),
        links: Vec::new(),
        comment: None,
        posting_type: PostingType::Regular,
        balance_assertion: None,
        date: None,
        date2: None,
        meta: posting.meta.clone().unwrap_or_default(),
        source: None,
    }
}

#[derive(Debug, Clone)]
struct GroupInfo {
    id: String,
    amount_index: Option<usize>,
    original_json: Option<String>,
    meta: Metadata,
}

#[derive(Debug, Clone)]
struct GroupedPosting {
    posting: super::Posting,
    info: GroupInfo,
}

fn from_beancount_posting_with_group(
    posting: &Posting,
    report: &mut ConversionReport,
    context: &str,
) -> (super::Posting, Option<GroupInfo>) {
    let mut mapped = from_beancount_posting(posting, report, context);
    let meta = posting.meta.clone().unwrap_or_default();
    apply_posting_meta(&meta, &mut mapped, report, context);
    let group_id = meta_string(&meta, "gl_posting_group");
    let amount_index = meta_usize(&meta, "gl_posting_amount_index");
    let original_json = meta_string(&meta, "gl_posting_original");
    let info = group_id.map(|id| GroupInfo {
        id,
        amount_index,
        original_json,
        meta,
    });
    (mapped, info)
}

fn merge_postings_from_beancount(
    txn: &Transaction,
    report: &mut ConversionReport,
) -> Vec<super::Posting> {
    let mut grouped: BTreeMap<String, Vec<GroupedPosting>> = BTreeMap::new();
    let mut slots = Vec::new();
    let mut seen_groups = BTreeSet::new();

    for (index, posting) in txn.postings.iter().enumerate() {
        let (mapped, info) =
            from_beancount_posting_with_group(posting, report, &format!("posting[{index}]"));
        if let Some(info) = info {
            let group_id = info.id.clone();
            grouped
                .entry(group_id.clone())
                .or_default()
                .push(GroupedPosting {
                    posting: mapped,
                    info,
                });
            if seen_groups.insert(group_id.clone()) {
                slots.push(PostingSlot::Group(group_id));
            }
        } else {
            slots.push(PostingSlot::Ungrouped(Box::new(mapped)));
        }
    }

    let mut merged_posts = BTreeMap::new();
    for (group_id, mut members) in grouped {
        let merged = merge_grouped_postings(group_id.clone(), &mut members, report);
        merged_posts.insert(group_id, merged);
    }

    let mut out = Vec::new();
    for slot in slots {
        match slot {
            PostingSlot::Ungrouped(posting) => out.push(*posting),
            PostingSlot::Group(group_id) => {
                if let Some(posting) = merged_posts.remove(&group_id) {
                    out.push(posting);
                }
            }
        }
    }
    out
}

#[derive(Debug)]
enum PostingSlot {
    Ungrouped(Box<super::Posting>),
    Group(String),
}

fn merge_grouped_postings(
    group_id: String,
    members: &mut [GroupedPosting],
    report: &mut ConversionReport,
) -> super::Posting {
    members.sort_by_key(|member| member.info.amount_index.unwrap_or(0));
    if let Some(original) = members
        .iter()
        .find_map(|member| member.info.original_json.as_ref())
    {
        match serde_json::from_str::<super::Posting>(original) {
            Ok(restored) => return restored,
            Err(error) => {
                report.push(
                    ConversionIssueKind::Dropped,
                    format!("beancount::Posting {group_id}"),
                    format!("failed to parse original posting JSON: {error}"),
                );
            }
        }
    }

    let mut merged = members
        .first()
        .map(|member| member.posting.clone())
        .unwrap_or_else(|| super::Posting {
            account: String::new(),
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
        });

    let mut amounts = Vec::new();
    for member in members.iter() {
        if let Some(mixed) = &member.posting.amount {
            amounts.extend(mixed.amounts.iter().cloned());
        } else {
            report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Posting {group_id}"),
                "missing amount while merging grouped postings",
            );
        }
    }
    merged.amount = if amounts.is_empty() {
        None
    } else {
        Some(MixedAmount { amounts })
    };
    if let Some(meta) = members.first().map(|member| &member.info.meta) {
        apply_posting_meta(meta, &mut merged, report, "grouped_posting");
    }
    merged
}

fn posting_requires_backup(posting: &super::Posting, amount_count: usize) -> bool {
    amount_count > 1
        || posting.posting_type != PostingType::Regular
        || !posting.tags.is_empty()
        || !posting.links.is_empty()
        || posting.balance_assertion.is_some()
        || posting.date.is_some()
        || posting.date2.is_some()
        || posting.status.is_some()
        || posting.comment.is_some()
        || (posting.lot_cost.is_some() && posting.cost_spec.is_some())
        || matches!(
            posting.price,
            Some(PriceSpec {
                price_type: PriceType::Total,
                ..
            })
        )
}

fn build_posting_meta(
    posting: &super::Posting,
    report: &mut ConversionReport,
    group_id: &str,
    amount_index: usize,
    include_backup: bool,
) -> Metadata {
    let mut meta = posting.meta.clone();
    if include_backup {
        if let Some(value) = json_string(posting, report, "posting") {
            meta.insert("gl_posting_original".to_string(), MetaValue::String(value));
        }
        meta.insert(
            "gl_posting_group".to_string(),
            MetaValue::String(group_id.to_string()),
        );
        meta.insert(
            "gl_posting_amount_index".to_string(),
            MetaValue::String(amount_index.to_string()),
        );
    }
    if posting.posting_type != PostingType::Regular {
        meta.insert(
            "gl_posting_type".to_string(),
            MetaValue::String(
                match posting.posting_type {
                    PostingType::Regular => "regular",
                    PostingType::Virtual => "virtual",
                    PostingType::BalancedVirtual => "balanced_virtual",
                }
                .to_string(),
            ),
        );
    }
    if !posting.tags.is_empty() {
        if let Some(value) = json_string(&posting.tags, report, "posting.tags") {
            meta.insert("gl_posting_tags".to_string(), MetaValue::String(value));
        }
    }
    if !posting.links.is_empty() {
        if let Some(value) = json_string(&posting.links, report, "posting.links") {
            meta.insert("gl_posting_links".to_string(), MetaValue::String(value));
        }
    }
    if let Some(date) = &posting.date {
        meta.insert(
            "gl_posting_date".to_string(),
            MetaValue::String(format_date(date)),
        );
    }
    if let Some(date) = &posting.date2 {
        meta.insert(
            "gl_posting_date2".to_string(),
            MetaValue::String(format_date(date)),
        );
    }
    if let Some(status) = &posting.status {
        meta.insert(
            "gl_posting_status".to_string(),
            MetaValue::String(format!("{status:?}").to_lowercase()),
        );
    }
    if let Some(comment) = &posting.comment {
        meta.insert(
            "gl_posting_comment".to_string(),
            MetaValue::String(comment.clone()),
        );
    }
    if let Some(assertion) = &posting.balance_assertion {
        if let Some(value) = json_string(assertion, report, "posting.balance_assertion") {
            meta.insert("gl_balance_assertion".to_string(), MetaValue::String(value));
        }
    }
    meta
}

fn posting_cost_for_beancount(
    posting: &super::Posting,
    report: &mut ConversionReport,
    context: &str,
) -> Option<PostingCost> {
    if let Some(lot_cost) = &posting.lot_cost {
        if posting.cost_spec.is_some() {
            report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Posting {context}"),
                "both lot_cost and cost_spec present; cost_spec stored in metadata",
            );
        }
        return Some(PostingCost::Cost(lot_cost.clone()));
    }
    posting
        .cost_spec
        .as_ref()
        .map(|spec| PostingCost::CostSpec(spec.clone()))
}

fn posting_price_for_beancount(
    posting: &super::Posting,
    report: &mut ConversionReport,
    context: &str,
    meta: &mut Metadata,
) -> Option<Amount> {
    let spec = posting.price.as_ref()?;
    if matches!(spec.price_type, PriceType::Total) {
        report.push(
            ConversionIssueKind::Dropped,
            format!("beancount::Posting {context}"),
            "total price not representable in beancount posting data; preserved in metadata",
        );
        meta.insert(
            "gl_price_type".to_string(),
            MetaValue::String("total".to_string()),
        );
    }
    Some(amount_for_beancount(&spec.amount, report, "posting.price"))
}

fn meta_to_option(meta: Metadata) -> Option<Metadata> {
    if meta.is_empty() {
        None
    } else {
        Some(meta)
    }
}

fn amount_for_beancount(amount: &Amount, report: &mut ConversionReport, context: &str) -> Amount {
    let mut out = amount.clone();
    if out.style.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("beancount::Amount {context}"),
            "beancount amounts do not store display styles",
        );
        out.style = None;
    }
    if out.cost.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("beancount::Amount {context}"),
            "beancount amounts do not store transaction prices",
        );
        out.cost = None;
    }
    if out.cost_basis.is_some() {
        report.push(
            ConversionIssueKind::Dropped,
            format!("beancount::Amount {context}"),
            "beancount amounts do not store cost bases",
        );
        out.cost_basis = None;
    }
    out
}

fn json_string<T: Serialize>(
    value: &T,
    report: &mut ConversionReport,
    context: &str,
) -> Option<String> {
    serde_json::to_string(value)
        .map_err(|error| {
            report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Json {context}"),
                format!("failed to serialize JSON: {error}"),
            );
        })
        .ok()
}

fn format_date(date: &Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

fn custom_from_transaction(txn: &super::Transaction, report: &mut ConversionReport) -> Custom {
    let value = json_string(txn, report, "transaction").unwrap_or_else(|| format!("{txn:?}"));
    Custom {
        meta: txn.meta.clone(),
        date: txn.date.clone(),
        custom_type: "gl.transaction".to_string(),
        values: vec![MetaValue::String(value)],
    }
}

fn meta_string(meta: &Metadata, key: &str) -> Option<String> {
    match meta.get(key) {
        Some(MetaValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn meta_u64(meta: &Metadata, key: &str) -> Option<u64> {
    meta_string(meta, key).and_then(|value| value.parse().ok())
}

fn meta_usize(meta: &Metadata, key: &str) -> Option<usize> {
    meta_string(meta, key).and_then(|value| value.parse().ok())
}

fn meta_date(meta: &Metadata, key: &str) -> Option<Date> {
    meta_string(meta, key).and_then(|value| parse_date(&value))
}

fn meta_status(meta: &Metadata, key: &str) -> Option<Status> {
    meta_string(meta, key).and_then(|value| match value.as_str() {
        "pending" => Some(Status::Pending),
        "cleared" => Some(Status::Cleared),
        "unmarked" => Some(Status::Unmarked),
        _ => None,
    })
}

fn meta_tags(
    meta: &Metadata,
    fallback: &BTreeSet<String>,
    report: &mut ConversionReport,
) -> Vec<Tag> {
    if let Some(value) = meta_string(meta, "gl_transaction_tags") {
        if let Ok(tags) = serde_json::from_str::<Vec<Tag>>(&value) {
            return tags;
        }
        report.push(
            ConversionIssueKind::Dropped,
            "beancount::Transaction",
            "failed to parse transaction tags from metadata",
        );
    }
    set_to_tags(Some(fallback))
}

fn parse_date(value: &str) -> Option<Date> {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year = parts[0].parse::<i32>().ok()?;
    let month = parts[1].parse::<u8>().ok()?;
    let day = parts[2].parse::<u8>().ok()?;
    Some(Date { year, month, day })
}

fn apply_posting_meta(
    meta: &Metadata,
    posting: &mut super::Posting,
    report: &mut ConversionReport,
    context: &str,
) {
    if let Some(value) = meta_string(meta, "gl_posting_type") {
        posting.posting_type = match value.as_str() {
            "virtual" => PostingType::Virtual,
            "balanced_virtual" => PostingType::BalancedVirtual,
            _ => PostingType::Regular,
        };
    }
    if let Some(value) = meta_string(meta, "gl_posting_tags") {
        match serde_json::from_str::<Vec<Tag>>(&value) {
            Ok(tags) => posting.tags = tags,
            Err(error) => report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Posting {context}"),
                format!("failed to parse posting tags: {error}"),
            ),
        }
    }
    if let Some(value) = meta_string(meta, "gl_posting_links") {
        match serde_json::from_str::<Vec<Link>>(&value) {
            Ok(links) => posting.links = links,
            Err(error) => report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Posting {context}"),
                format!("failed to parse posting links: {error}"),
            ),
        }
    }
    if let Some(value) = meta_string(meta, "gl_posting_date") {
        if let Some(date) = parse_date(&value) {
            posting.date = Some(date);
        }
    }
    if let Some(value) = meta_string(meta, "gl_posting_date2") {
        if let Some(date) = parse_date(&value) {
            posting.date2 = Some(date);
        }
    }
    if let Some(value) = meta_string(meta, "gl_posting_status") {
        let current = posting.status.clone();
        posting.status = match value.as_str() {
            "pending" => Some(Status::Pending),
            "cleared" => Some(Status::Cleared),
            "unmarked" => Some(Status::Unmarked),
            _ => current,
        };
    }
    if let Some(value) = meta_string(meta, "gl_posting_comment") {
        posting.comment = Some(value);
    }
    if let Some(value) = meta_string(meta, "gl_balance_assertion") {
        match serde_json::from_str::<BalanceAssertion>(&value) {
            Ok(assertion) => posting.balance_assertion = Some(assertion),
            Err(error) => report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Posting {context}"),
                format!("failed to parse balance assertion: {error}"),
            ),
        }
    }
    if let Some(value) = meta_string(meta, "gl_price_type") {
        if let Some(price) = &mut posting.price {
            if value == "total" {
                price.price_type = PriceType::Total;
            }
        }
    }
}

fn tags_to_set(tags: &[Tag], report: &mut ConversionReport, context: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for tag in tags {
        if tag.value.is_some() {
            report.push(
                ConversionIssueKind::Dropped,
                format!("beancount::Tags {context}"),
                "beancount tags do not support values; values dropped",
            );
        }
        if tag.hidden && !tag.name.starts_with('_') {
            report.push(
                ConversionIssueKind::Assumed,
                format!("beancount::Tags {context}"),
                "hidden tags are not supported; preserving name with leading underscore",
            );
            out.insert(format!("_{}", tag.name));
        } else {
            out.insert(tag.name.clone());
        }
    }
    out
}

fn tags_to_set_opt(
    tags: &[Tag],
    report: &mut ConversionReport,
    context: &str,
) -> Option<BTreeSet<String>> {
    if tags.is_empty() {
        None
    } else {
        Some(tags_to_set(tags, report, context))
    }
}

fn links_to_set(links: &[Link], report: &mut ConversionReport, context: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for link in links {
        if !out.insert(link.clone()) {
            report.push(
                ConversionIssueKind::Normalized,
                format!("beancount::Links {context}"),
                "duplicate links removed",
            );
        }
    }
    out
}

fn links_to_set_opt(
    links: &[Link],
    report: &mut ConversionReport,
    context: &str,
) -> Option<BTreeSet<String>> {
    if links.is_empty() {
        None
    } else {
        Some(links_to_set(links, report, context))
    }
}

fn set_to_tags(set: Option<&BTreeSet<String>>) -> Vec<Tag> {
    set.map(|tags| {
        tags.iter()
            .map(|tag| Tag {
                name: tag.clone(),
                value: None,
                hidden: tag.starts_with('_'),
            })
            .collect()
    })
    .unwrap_or_default()
}

fn set_to_links(set: Option<&BTreeSet<String>>) -> Vec<Link> {
    set.map(|links| links.iter().cloned().collect())
        .unwrap_or_default()
}
