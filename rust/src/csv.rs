//! Minimal CSV reading and writing, matching what Python's `csv` module does:
//! `\r\n` line endings and minimal quoting.

/// Quote a field the way `csv.writer` does (QUOTE_MINIMAL).
pub fn field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub struct Writer {
    out: String,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: String::new() }
    }

    pub fn row(&mut self, values: &[String]) {
        let cells = values.iter().map(|v| field(v)).collect::<Vec<_>>();
        self.out.push_str(&cells.join(","));
        self.out.push_str("\r\n");
    }

    pub fn contents(&self) -> &str {
        &self.out
    }
}

/// Parse CSV text into records (used for `summary.csv`).
pub fn read_records(content: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut value = String::new();
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    value.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                value.push(c);
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => record.push(std::mem::take(&mut value)),
            '\n' | '\r' => {
                record.push(std::mem::take(&mut value));
                if !record.is_empty() {
                    records.push(std::mem::take(&mut record));
                }
            }
            _ => value.push(c),
        }
    }

    if !value.is_empty() || !record.is_empty() {
        record.push(value);
        records.push(record);
    }

    records
}

/// Look up a column by header name in parsed records.
pub fn column(records: &[Vec<String>], header: &str) -> Option<usize> {
    records.first()?.iter().position(|name| name == header)
}
