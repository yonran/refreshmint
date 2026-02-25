use crate::hledger::{Amount, Posting, Side, Transaction};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerView {
    pub path: String,
    pub accounts: Vec<AccountRow>,
    pub transactions: Vec<TransactionRow>,
    pub gl_account_conflicts: Vec<crate::login_config::GlAccountConflict>,
}

#[derive(Debug, Serialize)]
pub struct AccountRow {
    pub name: String,
    pub totals: Option<Vec<AmountTotal>>,
    #[serde(rename = "unreconciledCount")]
    pub unreconciled_count: usize,
}

#[derive(Debug, Serialize)]
pub struct TransactionRow {
    pub id: String,
    pub date: String,
    pub description: String,
    #[serde(rename = "descriptionRaw")]
    pub description_raw: String,
    pub comment: String,
    pub accounts: String,
    pub totals: Option<Vec<AmountTotal>>,
    pub postings: Vec<PostingRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmountTotal {
    pub commodity: String,
    pub mantissa: String,
    pub scale: u32,
    pub style: Option<AmountStyleHint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmountStyleHint {
    pub side: Side,
    pub spaced: bool,
}

#[derive(Debug, Serialize)]
pub struct PostingRow {
    pub account: String,
    pub amount: Option<String>,
    pub comment: String,
    pub totals: Option<Vec<AmountTotal>>,
}

#[derive(Clone)]
struct CommodityStyle {
    side: Side,
    spaced: bool,
}

#[derive(Clone)]
struct CommodityTotal {
    mantissa: i128,
    scale: u32,
    style: Option<CommodityStyle>,
}

pub fn open_ledger_dir(path: &Path) -> Result<LedgerView, Box<dyn std::error::Error>> {
    crate::ledger::require_refreshmint_extension(path)?;
    if !path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "ledger directory not found").into());
    }
    let config = crate::ledger::read_refreshmint_config(path)?;
    if config.version != crate::version::APP_VERSION {
        return Err(io::Error::other(format!(
            "ledger version {} does not match app version {}",
            config.version,
            crate::version::APP_VERSION
        ))
        .into());
    }

    let journal_path = path.join("general.journal");
    if !journal_path.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "general.journal not found").into());
    }

    let transactions = run_hledger_print(&journal_path)?;
    let accounts = build_account_rows(path, &transactions)?;
    let transaction_rows = build_transaction_rows(&transactions);
    let gl_account_conflicts = crate::login_config::find_gl_account_conflicts(path);

    Ok(LedgerView {
        path: path.display().to_string(),
        accounts,
        transactions: transaction_rows,
        gl_account_conflicts,
    })
}

fn run_hledger_print(journal_path: &Path) -> io::Result<Vec<Transaction>> {
    let output = Command::new(crate::binpath::hledger_path())
        .arg("print")
        .arg("--output-format=json")
        .arg("-f")
        .arg(journal_path)
        .env("GIT_CONFIG_GLOBAL", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()?;

    if output.status.success() {
        serde_json::from_slice(&output.stdout).map_err(io::Error::other)
    } else {
        Err(io::Error::other(format!(
            "hledger print failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn build_account_rows(
    path: &Path,
    transactions: &[Transaction],
) -> Result<Vec<AccountRow>, Box<dyn std::error::Error>> {
    let mut accounts: BTreeMap<String, Option<BTreeMap<String, CommodityTotal>>> = BTreeMap::new();

    for txn in transactions {
        for posting in &txn.tpostings {
            let entry = accounts
                .entry(posting.paccount.clone())
                .or_insert_with(|| Some(BTreeMap::new()));

            if let Some(totals) = entry {
                for amount in &posting.pamount {
                    if add_amount_total(totals, amount).is_err() {
                        *entry = None;
                        break;
                    }
                }
            }
        }
    }

    // Build GL account -> (login, label) map for unreconciled counts
    let mut gl_to_login: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let logins = crate::login_config::list_logins(path)?;
    for login in &logins {
        let config = crate::login_config::read_login_config(path, login);
        for (label, account_config) in &config.accounts {
            if let Some(gl_account) = &account_config.gl_account {
                gl_to_login
                    .entry(gl_account.clone())
                    .or_default()
                    .push((login.clone(), label.clone()));
            }
        }
    }

    let rows = accounts
        .into_iter()
        .map(|(name, totals)| {
            let mut unreconciled_count = 0;
            if let Some(mappings) = gl_to_login.get(&name) {
                for (login, label) in mappings {
                    if let Ok(unreconciled) =
                        crate::post::get_unposted_login_account(path, login, label)
                    {
                        unreconciled_count += unreconciled.len();
                    }
                }
            }

            AccountRow {
                name,
                totals: totals.and_then(|totals| totals_to_rows(&totals)),
                unreconciled_count,
            }
        })
        .collect();
    Ok(rows)
}

fn build_transaction_rows(transactions: &[Transaction]) -> Vec<TransactionRow> {
    transactions
        .iter()
        .map(|txn| TransactionRow {
            id: txn
                .ttags
                .iter()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| txn.tindex.to_string()),
            date: txn.tdate.clone(),
            description: transaction_description(txn),
            description_raw: txn.tdescription.clone(),
            comment: txn.tcomment.clone(),
            accounts: transaction_accounts(txn),
            totals: transaction_amounts(txn),
            postings: transaction_postings(txn),
        })
        .collect()
}

fn transaction_description(txn: &Transaction) -> String {
    if !txn.tdescription.trim().is_empty() {
        return txn.tdescription.clone();
    }
    if !txn.tcomment.trim().is_empty() {
        return txn.tcomment.clone();
    }
    "(no description)".to_string()
}

fn transaction_accounts(txn: &Transaction) -> String {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for posting in &txn.tpostings {
        if seen.insert(posting.paccount.as_str()) {
            ordered.push(posting.paccount.clone());
        }
    }
    ordered.join(", ")
}

fn transaction_amounts(txn: &Transaction) -> Option<Vec<AmountTotal>> {
    let mut positive_totals: BTreeMap<String, CommodityTotal> = BTreeMap::new();
    let mut saw_positive = false;

    for posting in &txn.tpostings {
        for amount in &posting.pamount {
            let mantissa = parse_mantissa(&amount.aquantity.decimal_mantissa)?;
            if mantissa > 0 {
                saw_positive = true;
                if add_amount_total(&mut positive_totals, amount).is_err() {
                    return None;
                }
            }
        }
    }

    if saw_positive {
        return totals_to_rows(&positive_totals);
    }

    let mut all_totals: BTreeMap<String, CommodityTotal> = BTreeMap::new();
    for posting in &txn.tpostings {
        for amount in &posting.pamount {
            if add_amount_total(&mut all_totals, amount).is_err() {
                return None;
            }
        }
    }

    totals_to_rows(&all_totals)
}

fn transaction_postings(txn: &Transaction) -> Vec<PostingRow> {
    txn.tpostings
        .iter()
        .map(|posting| PostingRow {
            account: posting.paccount.clone(),
            amount: posting_amount_text(posting),
            comment: posting.pcomment.clone(),
            totals: posting_totals(posting),
        })
        .collect()
}

fn posting_amount_text(posting: &Posting) -> Option<String> {
    if posting.pamount.len() != 1 {
        return None;
    }
    format_amount(&posting.pamount[0])
}

fn format_amount(amount: &Amount) -> Option<String> {
    if amount.acost.is_some() || amount.acostbasis.is_some() {
        return None;
    }
    let mantissa = parse_mantissa(&amount.aquantity.decimal_mantissa)?;
    let number = format_decimal(mantissa, amount.aquantity.decimal_places);
    let commodity = amount.acommodity.as_str();
    if commodity.is_empty() {
        return Some(number);
    }
    let style = amount.astyle.as_ref().map(|style| CommodityStyle {
        side: style.ascommodityside.clone(),
        spaced: style.ascommodityspaced,
    });
    let (side, spaced) = style
        .as_ref()
        .map(|s| (s.side.clone(), s.spaced))
        .unwrap_or((Side::R, true));
    let space = if spaced { " " } else { "" };
    let formatted = match side {
        Side::L => format!("{commodity}{space}{number}"),
        Side::R => format!("{number}{space}{commodity}"),
    };
    Some(formatted)
}

fn format_decimal(mantissa: i128, scale: u32) -> String {
    let negative = mantissa < 0;
    let mut digits = mantissa.abs().to_string();
    if scale > 0 {
        let scale_usize = scale as usize;
        if digits.len() <= scale_usize {
            let needed = scale_usize + 1 - digits.len();
            let zeros = "0".repeat(needed);
            digits = format!("{zeros}{digits}");
        }
        let split = digits.len() - scale_usize;
        let (int_part, frac_part) = digits.split_at(split);
        digits = format!("{int_part}.{frac_part}");
    }
    if negative {
        format!("-{digits}")
    } else {
        digits
    }
}

fn posting_totals(posting: &Posting) -> Option<Vec<AmountTotal>> {
    let mut totals: BTreeMap<String, CommodityTotal> = BTreeMap::new();
    for amount in &posting.pamount {
        if add_amount_total(&mut totals, amount).is_err() {
            return None;
        }
    }
    totals_to_rows(&totals)
}

fn add_amount_total(
    totals: &mut BTreeMap<String, CommodityTotal>,
    amount: &Amount,
) -> Result<(), ()> {
    let entry = totals
        .entry(amount.acommodity.clone())
        .or_insert_with(|| CommodityTotal {
            mantissa: 0,
            scale: amount.aquantity.decimal_places,
            style: style_from_amount(amount),
        });

    if entry.style.is_none() {
        entry.style = style_from_amount(amount);
    }

    add_amount_to_total(entry, amount)
}

fn add_amount_to_total(total: &mut CommodityTotal, amount: &Amount) -> Result<(), ()> {
    let mantissa = parse_mantissa(&amount.aquantity.decimal_mantissa).ok_or(())?;
    let scale = amount.aquantity.decimal_places;
    let mut scaled_mantissa = mantissa;

    if scale > total.scale {
        let factor = pow10(scale - total.scale).ok_or(())?;
        total.mantissa = total.mantissa.checked_mul(factor).ok_or(())?;
        total.scale = scale;
    } else if scale < total.scale {
        let factor = pow10(total.scale - scale).ok_or(())?;
        scaled_mantissa = scaled_mantissa.checked_mul(factor).ok_or(())?;
    }

    total.mantissa = total.mantissa.checked_add(scaled_mantissa).ok_or(())?;
    Ok(())
}

fn style_from_amount(amount: &Amount) -> Option<CommodityStyle> {
    amount.astyle.as_ref().map(|style| CommodityStyle {
        side: style.ascommodityside.clone(),
        spaced: style.ascommodityspaced,
    })
}

fn parse_mantissa(number: &serde_json::Number) -> Option<i128> {
    if let Some(value) = number.as_i64() {
        Some(i128::from(value))
    } else if let Some(value) = number.as_u64() {
        Some(i128::from(value))
    } else if let Some(value) = number.as_f64() {
        if value.fract() == 0.0 {
            Some(value as i128)
        } else {
            None
        }
    } else {
        None
    }
}

fn pow10(scale: u32) -> Option<i128> {
    10_i128.checked_pow(scale)
}

fn totals_to_rows(totals: &BTreeMap<String, CommodityTotal>) -> Option<Vec<AmountTotal>> {
    if totals.is_empty() {
        return None;
    }

    let rows = totals
        .iter()
        .map(|(commodity, total)| AmountTotal {
            commodity: commodity.clone(),
            mantissa: total.mantissa.to_string(),
            scale: total.scale,
            style: total.style.as_ref().map(|style| AmountStyleHint {
                side: style.side.clone(),
                spaced: style.spaced,
            }),
        })
        .collect::<Vec<_>>();
    Some(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::hledger::{SourcePos, SourceSpan, Status};

    fn dummy_source_pos() -> SourcePos {
        SourcePos {
            source_name: String::new(),
            source_line: 1,
            source_column: 1,
        }
    }

    fn dummy_span() -> SourceSpan {
        SourceSpan(dummy_source_pos(), dummy_source_pos())
    }

    fn make_txn(tindex: i64, ttags: Vec<(String, String)>) -> Transaction {
        Transaction {
            tindex,
            tprecedingcomment: String::new(),
            tsourcepos: dummy_span(),
            tdate: "2024-01-01".to_string(),
            tdate2: None,
            tstatus: Status::Cleared,
            tcode: String::new(),
            tdescription: "Test".to_string(),
            tcomment: String::new(),
            ttags,
            tpostings: vec![],
        }
    }

    #[test]
    fn build_transaction_rows_uses_id_tag_when_present() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let txn = make_txn(5, vec![("id".to_string(), uuid.to_string())]);
        let rows = build_transaction_rows(&[txn]);
        assert_eq!(rows[0].id, uuid);
    }

    #[test]
    fn build_transaction_rows_falls_back_to_tindex_when_id_absent() {
        let txn = make_txn(7, vec![]);
        let rows = build_transaction_rows(&[txn]);
        assert_eq!(rows[0].id, "7");
    }
}
