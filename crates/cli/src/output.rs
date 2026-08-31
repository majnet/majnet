//! Rendering. Two audiences, one code path.
//!
//! A human at a terminal wants aligned columns; a script — or an agent driving
//! this CLI — wants the API's JSON, unreshaped. So every command builds the
//! same value and the format flag decides: `--output table` (default when
//! stdout is a terminal) or `--output json`, which prints exactly what the
//! control plane said. Nothing is summarised away in JSON mode, because the
//! moment the CLI paraphrases, a caller has to guess what was dropped.

use anyhow::Result;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Format {
    /// Aligned columns for reading.
    Table,
    /// The API's JSON, pretty-printed.
    Json,
    /// The API's JSON as YAML — easier to eyeball for nested payloads.
    Yaml,
}

/// Longest cell we print in a table before eliding. JSON mode is never elided.
const MAX_CELL: usize = 64;

pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    /// Printed instead of the table when there are no rows.
    empty: String,
}

impl Table {
    pub fn new<S: AsRef<str>>(headers: &[S]) -> Self {
        Self {
            headers: headers.iter().map(|h| h.as_ref().to_uppercase()).collect(),
            rows: Vec::new(),
            empty: "(none)".into(),
        }
    }

    pub fn empty_note(mut self, note: &str) -> Self {
        self.empty = note.into();
        self
    }

    pub fn push<S: Into<String>>(&mut self, row: Vec<S>) {
        self.rows.push(row.into_iter().map(Into::into).collect());
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn print(&self) {
        if self.rows.is_empty() {
            println!("{}", self.empty);
            return;
        }
        let columns = self.headers.len();
        let mut width: Vec<usize> = self.headers.iter().map(|h| display_len(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().take(columns).enumerate() {
                width[i] = width[i].max(display_len(&elide(cell, MAX_CELL)));
            }
        }
        print_row(&self.headers, &width);
        for row in &self.rows {
            let cells: Vec<String> = row.iter().map(|c| elide(c, MAX_CELL)).collect();
            print_row(&cells, &width);
        }
    }
}

fn print_row(cells: &[String], width: &[usize]) {
    let last = cells.len().saturating_sub(1);
    let mut line = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i == last {
            line.push_str(cell);
        } else {
            line.push_str(cell);
            let pad = width[i].saturating_sub(display_len(cell)) + 2;
            line.push_str(&" ".repeat(pad));
        }
    }
    println!("{}", line.trim_end());
}

/// Character count, not byte count — a table of Czech app names must not drift.
fn display_len(s: &str) -> usize {
    s.chars().count()
}

/// Truncate on a character boundary, marking that something was cut. Newlines
/// become spaces: one row is one line, or the columns stop meaning anything.
pub fn elide(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    flat.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// One field of a JSON object as a display string. Strings print bare; numbers,
/// booleans and nested values print as compact JSON rather than as a blank —
/// a missing value and a `false` are different facts.
pub fn cell(value: &Value, key: &str) -> String {
    match value.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_string))
            .collect::<Vec<_>>()
            .join(","),
        Some(other) => other.to_string(),
    }
}

/// A `Value` as a list of objects — tolerating an API that returns a bare
/// object where a list was expected, and `null` for "nothing".
pub fn rows(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

pub fn table_from(value: &Value, columns: &[&str]) -> Table {
    let mut table = Table::new(columns);
    for item in rows(value) {
        table.push(columns.iter().map(|c| cell(&item, c)).collect::<Vec<_>>());
    }
    table
}

/// Print `value` in the requested format; `table` is only built for table mode.
pub fn emit(format: Format, value: &Value, table: impl FnOnce(&Value)) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(value)?),
        Format::Yaml => print!("{}", serde_yaml::to_string(value)?),
        Format::Table => table(value),
    }
    Ok(())
}

/// Human byte sizes for the metrics tables.
pub fn bytes(n: f64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = n;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn elide_is_char_safe_and_flattens_newlines() {
        assert_eq!(elide("abc", 8), "abc");
        assert_eq!(elide("abcdefgh", 4), "abc…");
        assert_eq!(elide("čárka", 3), "čá…");
        assert_eq!(elide("a\nb", 8), "a b");
    }

    /// A `false` or a `0` is information; printing it as an empty cell would be
    /// indistinguishable from "the API didn't say".
    #[test]
    fn cell_renders_non_strings_rather_than_blanking_them() {
        let v = json!({"s": "x", "n": 3, "b": false, "nil": null, "list": ["a", "b"]});
        assert_eq!(cell(&v, "s"), "x");
        assert_eq!(cell(&v, "n"), "3");
        assert_eq!(cell(&v, "b"), "false");
        assert_eq!(cell(&v, "list"), "a,b");
        assert_eq!(cell(&v, "nil"), "");
        assert_eq!(cell(&v, "absent"), "");
    }

    #[test]
    fn rows_tolerates_a_bare_object_and_null() {
        assert_eq!(rows(&json!([1, 2])).len(), 2);
        assert_eq!(rows(&json!({"a": 1})).len(), 1);
        assert_eq!(rows(&Value::Null).len(), 0);
    }

    #[test]
    fn byte_sizes_are_human_readable() {
        assert_eq!(bytes(512.0), "512B");
        assert_eq!(bytes(1536.0), "1.5K");
        assert_eq!(bytes(2.0 * 1024.0 * 1024.0 * 1024.0), "2.0G");
    }
}
