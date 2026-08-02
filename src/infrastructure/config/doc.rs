//! Dotted-path helpers over a format-preserving `toml_edit` document.
//!
//! The write path edits *the parsed document* rather than re-serializing a
//! struct. That is what makes both plan requirements hold at once:
//!
//! - **unknown keys survive** — a key this binary doesn't model is simply
//!   never touched, so a newer clash's settings (and a user's `[keymap]`
//!   section) are not silently deleted by an older one's save;
//! - **comments and key order survive** — the reason Decision 1 kept TOML.
//!
//! Mirror of the `toml::Table` helpers in [`super::layers`], against the other
//! representation. Pure.

use toml_edit::{DocumentMut, Item, Table, Value};

/// Read a dotted path out of a document.
pub fn get<'a>(document: &'a DocumentMut, path: &str) -> Option<&'a Item> {
    let mut table = document.as_table();
    let parts: Vec<&str> = path.split('.').collect();
    let (last, parents) = parts.split_last()?;
    for part in parents {
        table = table.get(part)?.as_table()?;
    }
    table.get(last)
}

/// Write a scalar at a dotted path, creating intermediate tables. A non-table
/// value standing where a section belongs is replaced — the alternative is
/// dropping the write.
pub fn set(document: &mut DocumentMut, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let Some((last, parents)) = parts.split_last() else {
        return;
    };
    let mut table = document.as_table_mut();
    for part in parents {
        let entry = table.entry(part).or_insert(Item::Table(Table::new()));
        if !entry.is_table() {
            *entry = Item::Table(Table::new());
        }
        table = entry.as_table_mut().expect("just ensured a table");
    }
    match table.get_mut(last) {
        // Assigning through the existing item keeps its trailing comment and
        // spacing — the whole point of editing rather than rewriting.
        Some(existing) if existing.is_value() => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            let mut new_value = value;
            if let Some(decor) = decor {
                *new_value.decor_mut() = decor;
            }
            *existing = Item::Value(new_value);
        }
        _ => {
            table.insert(last, Item::Value(value));
        }
    }
}

/// Like [`set`], but carries `source_key`'s decor onto the new key.
///
/// Used when a migration *moves* a key: a comment written above `claude_bin`
/// lives on the key, not the value, so a plain remove-and-insert would delete
/// the user's annotation while claiming to preserve formatting.
pub fn set_keeping_decor(
    document: &mut DocumentMut,
    path: &str,
    value: Value,
    source_key: &toml_edit::Key,
) {
    set(document, path, value);
    let parts: Vec<&str> = path.split('.').collect();
    let Some((last, parents)) = parts.split_last() else {
        return;
    };
    let mut table = document.as_table_mut();
    for part in parents {
        match table.get_mut(part).and_then(|i| i.as_table_mut()) {
            Some(inner) => table = inner,
            None => return,
        }
    }
    if let Some((mut key, _)) = table.get_key_value_mut(last) {
        *key.leaf_decor_mut() = source_key.leaf_decor().clone();
        *key.dotted_decor_mut() = source_key.dotted_decor().clone();
    }
}

/// Remove a dotted path, pruning tables it leaves empty. Returns whether
/// anything was removed.
pub fn remove(document: &mut DocumentMut, path: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    remove_parts(document.as_table_mut(), &parts)
}

fn remove_parts(table: &mut Table, parts: &[&str]) -> bool {
    match parts {
        [] => false,
        [last] => table.remove(last).is_some(),
        [head, rest @ ..] => {
            let Some(child) = table.get_mut(head).and_then(|i| i.as_table_mut()) else {
                return false;
            };
            let removed = remove_parts(child, rest);
            if removed && child.is_empty() {
                table.remove(head);
            }
            removed
        }
    }
}

/// Convert a `toml::Value` scalar into the `toml_edit` representation.
///
/// Only scalars: every schema property is one, and arrays/tables are never
/// written through the settings path (`[[ides]]` is hand-edited).
pub fn scalar(value: &toml::Value) -> Option<Value> {
    Some(match value {
        toml::Value::String(s) => Value::from(s.as_str()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::from(*b),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_creates_nested_sections() {
        let mut document = DocumentMut::new();
        set(&mut document, "sessions.refresh_secs", Value::from(5));
        let text = document.to_string();
        assert!(text.contains("[sessions]"), "{}", text);
        assert!(text.contains("refresh_secs = 5"), "{}", text);
        assert_eq!(
            get(&document, "sessions.refresh_secs").and_then(|i| i.as_integer()),
            Some(5)
        );
    }

    #[test]
    fn set_preserves_surrounding_comments_and_unknown_keys() {
        let mut document: DocumentMut = r#"# top of file
schema_version = 2

[sessions]
# how often we poll
refresh_secs = 2
something_new = "from a newer clash"

[keymap]
"session.reload" = "cmd+r"
"#
        .parse()
        .unwrap();

        set(&mut document, "sessions.refresh_secs", Value::from(9));
        let text = document.to_string();

        assert!(text.contains("refresh_secs = 9"), "{}", text);
        // Every one of these would be gone if we re-serialized a struct.
        assert!(text.contains("# top of file"), "{}", text);
        assert!(text.contains("# how often we poll"), "{}", text);
        assert!(text.contains("something_new"), "{}", text);
        assert!(text.contains("[keymap]"), "{}", text);
    }

    #[test]
    fn set_keeps_a_trailing_comment_on_the_edited_line() {
        let mut document: DocumentMut = "[sessions]\nrefresh_secs = 2 # tuned\n".parse().unwrap();
        set(&mut document, "sessions.refresh_secs", Value::from(4));
        let text = document.to_string();
        assert!(text.contains("# tuned"), "{}", text);
        assert!(text.contains("4"), "{}", text);
    }

    #[test]
    fn remove_prunes_emptied_sections() {
        let mut document: DocumentMut = "[paths]\nscratch_dir = \"/x\"\n".parse().unwrap();
        assert!(remove(&mut document, "paths.scratch_dir"));
        assert_eq!(document.to_string().trim(), "");
        assert!(!remove(&mut document, "paths.scratch_dir"));
    }

    #[test]
    fn remove_keeps_a_section_that_still_has_keys() {
        let mut document: DocumentMut = "[paths]\nscratch_dir = \"/x\"\nworkflows_dir = \"/y\"\n"
            .parse()
            .unwrap();
        assert!(remove(&mut document, "paths.scratch_dir"));
        let text = document.to_string();
        assert!(text.contains("[paths]"), "{}", text);
        assert!(text.contains("workflows_dir"), "{}", text);
    }

    #[test]
    fn set_replaces_a_scalar_standing_where_a_section_belongs() {
        let mut document: DocumentMut = "sessions = 1\n".parse().unwrap();
        set(&mut document, "sessions.refresh_secs", Value::from(3));
        assert_eq!(
            get(&document, "sessions.refresh_secs").and_then(|i| i.as_integer()),
            Some(3)
        );
    }

    #[test]
    fn scalar_converts_every_kind_the_schema_can_express() {
        assert!(scalar(&toml::Value::String("x".into())).is_some());
        assert!(scalar(&toml::Value::Integer(1)).is_some());
        assert!(scalar(&toml::Value::Float(1.5)).is_some());
        assert!(scalar(&toml::Value::Boolean(true)).is_some());
        assert!(scalar(&toml::Value::Array(vec![])).is_none());
    }
}
