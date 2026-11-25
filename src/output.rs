use anyhow::Result;
use clap::ValueEnum;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::Serialize;
use std::io::{self, Write};

#[derive(Clone, Copy, Default, ValueEnum)]
pub enum Format {
    #[default]
    Text,
    Json,
}

pub fn json(value: &impl Serialize) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    writeln!(stdout)?;
    Ok(())
}

pub fn list<T: Serialize, const N: usize>(
    format: Format,
    headers: [&str; N],
    rows: &[T],
    cells: impl Fn(&T) -> [String; N],
    empty: &str,
) -> Result<()> {
    if matches!(format, Format::Json) {
        return json(&rows);
    }
    let mut stdout = io::stdout().lock();
    if rows.is_empty() {
        writeln!(stdout, "{empty}")?;
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED).set_header(headers);
        for row in rows {
            table.add_row(cells(row));
        }
        writeln!(stdout, "{table}")?;
    }
    Ok(())
}
