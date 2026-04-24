use anyhow::{Result, anyhow};
use toml_edit::{DocumentMut, Item, Value};

#[allow(dead_code)]
pub fn get_value_by_path(doc: &DocumentMut, path: &str) -> Result<String> {
    let mut current = doc.as_item();
    for key in path.split('.') {
        current = current
            .get(key)
            .ok_or_else(|| anyhow!("Key '{}' not found", key))?;
    }

    if let Some(val) = current.as_value() {
        Ok(val.to_string().trim().to_string())
    } else {
        Ok(current.to_string().trim().to_string())
    }
}

pub fn set_value_by_path(doc: &mut DocumentMut, path: &str, value: &str) -> Result<()> {
    let keys: Vec<&str> = path.split('.').collect();
    let (last_key, parents) = keys.split_last().ok_or_else(|| anyhow!("Empty path"))?;

    let mut current = doc.as_item_mut();
    for key in parents {
        if !current.is_table() {
            *current = Item::Table(toml_edit::Table::new());
        }

        if current.get(*key).is_none() {
            current
                .as_table_mut()
                .unwrap()
                .insert(key, Item::Table(toml_edit::Table::new()));
        }
        current = current.get_mut(key).unwrap();
    }

    let toml_val: Value = value
        .parse()
        .map_err(|_| anyhow!("Failed to parse value as TOML"))?;

    if let Some(table) = current.as_table_mut() {
        if let Some(existing) = table.get_mut(last_key) {
            if let Some(val) = existing.as_value_mut() {
                *val = toml_val;
            } else {
                *existing = Item::Value(toml_val);
            }
        } else {
            table.insert(last_key, Item::Value(toml_val));
        }
    } else if let Some(table) = current.as_inline_table_mut() {
        table.insert(*last_key, toml_val);
    } else {
        return Err(anyhow!("Cannot set value on non-table item"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_value_by_path() {
        let toml = r#"
[agents.coder]
model = "gpt-4"
"#;
        let doc: DocumentMut = toml.parse().unwrap();
        assert_eq!(
            get_value_by_path(&doc, "agents.coder.model").unwrap(),
            "\"gpt-4\""
        );
    }

    #[test]
    fn test_set_value_by_path_preserves_comments() {
        let toml = r#"
[agents.coder]
# This is a comment
model = "gpt-3.5"
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        set_value_by_path(&mut doc, "agents.coder.model", "\"gpt-4\"").unwrap();

        let new_toml = doc.to_string();
        assert!(new_toml.contains("# This is a comment"));
        assert!(new_toml.contains("model = \"gpt-4\""));
    }

    #[test]
    fn test_set_value_creates_missing_tables() {
        let mut doc = DocumentMut::new();
        set_value_by_path(&mut doc, "new.table.key", "true").unwrap();
        assert!(doc.to_string().contains("[new.table]"));
        assert!(doc.to_string().contains("key = true"));
    }
}
