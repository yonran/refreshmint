use std::collections::HashMap;

/// Shared helpers for `general.journal` transaction block identity.
/// Keep this aligned with:
/// - `ledger_add.rs`, which injects ids for manual/raw GL additions
/// - `migration.rs`, which backfills missing ids in existing ledgers
/// - `post.rs`, which edits/replaces GL transaction blocks by id
pub fn split_journal_blocks(content: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        let starts_new_block = !line.trim().is_empty()
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !current.trim().is_empty();
        if starts_new_block {
            blocks.push(current.trim_end().to_string());
            current.clear();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.trim().is_empty() {
        blocks.push(current.trim_end().to_string());
    }

    blocks
}

pub fn block_transaction_id(block: &str) -> Option<String> {
    for (line_index, line) in block.lines().enumerate() {
        if let Some(id) = parse_id_from_line(line, line_index == 0) {
            return Some(id);
        }
    }
    None
}

pub fn ensure_block_has_id(block: &str) -> (String, String, bool) {
    if let Some(id) = block_transaction_id(block) {
        return (block.trim_end().to_string(), id, false);
    }

    let id = uuid::Uuid::new_v4().to_string();
    let mut lines: Vec<String> = block.lines().map(ToOwned::to_owned).collect();
    if let Some(header_index) = lines.iter().position(|line| {
        !line.trim().is_empty() && !line.starts_with(' ') && !line.starts_with('\t')
    }) {
        lines[header_index].push_str(&format!("  ; id: {id}"));
    } else {
        lines.push(format!("; id: {id}"));
    }
    (lines.join("\n").trim_end().to_string(), id, true)
}

pub fn ensure_journal_has_ids(content: &str) -> (String, Vec<String>) {
    let mut inserted_ids = Vec::new();
    let blocks: Vec<String> = split_journal_blocks(content)
        .into_iter()
        .map(|block| {
            let (updated, id, inserted) = ensure_block_has_id(&block);
            if inserted {
                inserted_ids.push(id);
            }
            updated
        })
        .collect();

    let mut updated = blocks.join("\n\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    (updated, inserted_ids)
}

pub fn replace_txn_ids(ids: &[String], replacements: &HashMap<String, String>) -> Vec<String> {
    let mut updated: Vec<String> = ids
        .iter()
        .map(|id| replacements.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    updated.sort();
    updated.dedup();
    updated
}

fn parse_id_from_line(line: &str, is_header: bool) -> Option<String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("; id: ") {
        let id = rest.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    if !is_header {
        return None;
    }
    let inline = line
        .split(';')
        .skip(1)
        .map(str::trim)
        .find_map(|part| part.strip_prefix("id: ").map(str::trim))?;
    if inline.is_empty() {
        None
    } else {
        Some(inline.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn block_transaction_id_reads_header_and_comment_tags() {
        assert_eq!(
            block_transaction_id(
                "2026-01-01 Example  ; id: gl-1\n  Assets:Cash  1 USD\n  Income:Test\n"
            ),
            Some("gl-1".to_string())
        );
        assert_eq!(
            block_transaction_id(
                "2026-01-01 Example\n    ; id: gl-2\n  Assets:Cash  1 USD\n  Income:Test\n"
            ),
            Some("gl-2".to_string())
        );
    }

    #[test]
    fn ensure_block_has_id_injects_header_id_when_missing() {
        let (updated, id, inserted) =
            ensure_block_has_id("2026-01-01 Example\n  Assets:Cash  1 USD\n  Income:Test\n");
        assert!(inserted);
        assert!(updated.contains(&format!("; id: {id}")));
    }

    #[test]
    fn replace_txn_ids_deduplicates_replacements() {
        let mut replacements = HashMap::new();
        replacements.insert("old-a".to_string(), "new".to_string());
        replacements.insert("old-b".to_string(), "new".to_string());
        assert_eq!(
            replace_txn_ids(
                &["keep".to_string(), "old-a".to_string(), "old-b".to_string()],
                &replacements,
            ),
            vec!["keep".to_string(), "new".to_string()]
        );
    }
}
