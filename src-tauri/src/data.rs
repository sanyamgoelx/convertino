//! Data files (CSV, TSV, JSON, YAML, XML in; CSV, TSV, JSON, YAML, XLSX out),
//! converted in Rust with no outside tools.
//!
//! Everything goes through a JSON value. Tables become arrays of objects
//! keyed by the header row; going back to a table, nested objects become
//! dotted columns ("address.city") and lists are written as JSON text.

use serde_json::{Map, Value};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Tsv,
    Json,
    Yaml,
    Xml,
    Xlsx,
}

impl Format {
    pub fn from_ext(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "csv" => Format::Csv,
            "tsv" | "tab" => Format::Tsv,
            "json" | "jsonl" | "ndjson" | "geojson" => Format::Json,
            "yaml" | "yml" => Format::Yaml,
            "xml" => Format::Xml,
            "xlsx" => Format::Xlsx,
            _ => return None,
        })
    }
}

pub fn convert(input: &Path, to: Format, output: &Path) -> Result<(), String> {
    let ext = input.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let from = Format::from_ext(&ext).ok_or_else(|| format!("Convertino can't read .{ext} data files."))?;
    let bytes = std::fs::read(input).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let text = decode(&bytes);
    let value = read(&text, from)?;
    let sheet = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Sheet1".into());
    write(&value, to, output, &sheet)
}

/// UTF-8 (with or without BOM), UTF-16 with BOM, or Windows-1252 as a last resort.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.len() >= 2 && (bytes[..2] == [0xFF, 0xFE] || bytes[..2] == [0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // Old Excel CSV exports: Windows-1252, which matches Latin-1 for letters people type.
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

// ---------- reading ----------

pub fn read(text: &str, from: Format) -> Result<Value, String> {
    match from {
        Format::Csv => {
            // Excel in many European locales writes semicolons.
            let first = text.lines().next().unwrap_or("");
            let delim = if first.matches(';').count() > first.matches(',').count() { b';' } else { b',' };
            read_table(text, delim)
        }
        Format::Tsv => read_table(text, b'\t'),
        Format::Json => read_json(text),
        Format::Yaml => serde_norway::from_str::<Value>(text).map_err(|e| format!("This YAML file has an error: {e}")),
        Format::Xml => read_xml(text),
        Format::Xlsx => Err("Excel files are converted through LibreOffice.".into()),
    }
}

/// A cell's text as a number or true/false when that's clearly what it is.
/// Text like "007" or "1e5" stays text so nothing is silently changed.
fn infer(cell: &str) -> Value {
    let t = cell.trim();
    if t.is_empty() {
        return Value::Null;
    }
    match t {
        "true" | "TRUE" | "True" => return Value::Bool(true),
        "false" | "FALSE" | "False" => return Value::Bool(false),
        _ => {}
    }
    let digits = t.strip_prefix('-').unwrap_or(t);
    let plain = !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.matches('.').count() <= 1
        && !digits.starts_with('.')
        && !digits.ends_with('.')
        && !(digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0."));
    if plain && t == cell {
        if let Ok(i) = t.parse::<i64>() {
            return Value::from(i);
        }
        if let Ok(f) = t.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return Value::Number(n);
            }
        }
    }
    Value::String(cell.to_string())
}

fn read_table(text: &str, delim: u8) -> Result<Value, String> {
    let mut rdr = csv::ReaderBuilder::new().delimiter(delim).flexible(true).from_reader(text.as_bytes());
    let mut headers: Vec<String> = Vec::new();
    for (i, h) in rdr.headers().map_err(|e| format!("Couldn't read the header row: {e}"))?.iter().enumerate() {
        let base = if h.trim().is_empty() { format!("column {}", i + 1) } else { h.trim().to_string() };
        let mut name = base.clone();
        let mut n = 2;
        while headers.contains(&name) {
            name = format!("{base} {n}");
            n += 1;
        }
        headers.push(name);
    }
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("Row problem: {e}"))?;
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let mut obj = Map::new();
        for (i, cell) in rec.iter().enumerate() {
            let key = headers.get(i).cloned().unwrap_or_else(|| format!("column {}", i + 1));
            obj.insert(key, infer(cell));
        }
        rows.push(Value::Object(obj));
    }
    Ok(Value::Array(rows))
}

fn read_json(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => Ok(v),
        Err(first_error) => {
            // JSON Lines: one value per line.
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() > 1 {
                if let Ok(items) = lines.iter().map(|l| serde_json::from_str::<Value>(l)).collect::<Result<Vec<_>, _>>() {
                    return Ok(Value::Array(items));
                }
            }
            Err(format!("This JSON file has an error: {first_error}"))
        }
    }
}

/// Elements become objects; attributes are "@name", mixed text is "#text",
/// repeated children become lists, and text-only elements become values.
fn read_xml(text: &str) -> Result<Value, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    struct Node {
        name: String,
        map: Map<String, Value>,
        text: String,
    }
    fn add(map: &mut Map<String, Value>, key: String, v: Value) {
        match map.get_mut(&key) {
            Some(Value::Array(list)) => list.push(v),
            Some(existing) => {
                let old = existing.take();
                *existing = Value::Array(vec![old, v]);
            }
            None => {
                map.insert(key, v);
            }
        }
    }
    fn finish(node: Node) -> (String, Value) {
        let text = node.text.trim().to_string();
        let v = if node.map.is_empty() {
            infer(&text)
        } else {
            let mut m = node.map;
            if !text.is_empty() {
                m.insert("#text".into(), Value::String(text));
            }
            Value::Object(m)
        };
        (node.name, v)
    }
    fn open(e: &quick_xml::events::BytesStart) -> Result<Node, String> {
        let name = e.name().as_ref().to_string();
        let mut map = Map::new();
        for a in e.attributes().flatten() {
            let key = format!("@{}", a.key.as_ref());
            let val = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
            map.insert(key, Value::String(val));
        }
        Ok(Node { name, map, text: String::new() })
    }

    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<(String, Value)> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => stack.push(open(&e)?),
            Ok(Event::Empty(e)) => {
                let (k, v) = finish(open(&e)?);
                match stack.last_mut() {
                    Some(parent) => add(&mut parent.map, k, v),
                    None => root = Some((k, v)),
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(match &*r {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(_)) => {
                let node = stack.pop().ok_or("This XML file has a closing tag with no opening tag.")?;
                let (k, v) = finish(node);
                match stack.last_mut() {
                    Some(parent) => add(&mut parent.map, k, v),
                    None => root = Some((k, v)),
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("This XML file has an error at byte {}: {e}", reader.error_position())),
        }
    }
    let (k, v) = root.ok_or("This XML file is empty.")?;
    let mut m = Map::new();
    m.insert(k, v);
    Ok(Value::Object(m))
}

// ---------- writing ----------

/// The rows of a value: the first list of records found (so `{"users": [...]}`
/// and XML like `<list><item/>...</list>` both give their records), else one row.
fn records(v: &Value) -> Vec<&Value> {
    fn find(v: &Value, depth: u8) -> Option<&Vec<Value>> {
        match v {
            Value::Array(items) if !items.is_empty() => Some(items),
            Value::Object(m) if depth < 4 => {
                // Only descend through wrappers: objects whose values are mostly one list.
                let lists: Vec<&Value> = m.values().filter(|x| x.is_array() || x.is_object()).collect();
                if lists.len() == 1 && m.len() <= 2 {
                    find(lists[0], depth + 1)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    match find(v, 0) {
        Some(items) => items.iter().collect(),
        None => vec![v],
    }
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, x) in m {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&key, x, out);
            }
        }
        Value::Array(_) | Value::Object(_) => out.push((prefix.to_string(), Value::String(v.to_string()))),
        other => {
            let key = if prefix.is_empty() { "value".to_string() } else { prefix.to_string() };
            out.push((key, other.clone()));
        }
    }
}

/// Header row plus cells, columns in first-seen order.
pub fn table(v: &Value) -> (Vec<String>, Vec<Vec<Value>>) {
    let mut headers: Vec<String> = Vec::new();
    let mut flat_rows = Vec::new();
    for rec in records(v) {
        let mut cells = Vec::new();
        match rec {
            // A row given as a list: numbered columns.
            Value::Array(items) => {
                for (i, x) in items.iter().enumerate() {
                    let mut one = Vec::new();
                    flatten(&format!("column {}", i + 1), x, &mut one);
                    cells.extend(one);
                }
            }
            _ => flatten("", rec, &mut cells),
        }
        for (k, _) in &cells {
            if !headers.contains(k) {
                headers.push(k.clone());
            }
        }
        flat_rows.push(cells);
    }
    let keys = headers.clone();
    // XML attribute and text markers read badly as column names: "@sku" -> "sku".
    let mut headers: Vec<String> = Vec::new();
    for k in &keys {
        let nice = k.split('.').map(|p| p.trim_start_matches(['@', '#'])).collect::<Vec<_>>().join(".");
        headers.push(if nice.is_empty() || keys.contains(&nice) || headers.contains(&nice) { k.clone() } else { nice });
    }
    let rows = flat_rows
        .into_iter()
        .map(|cells| {
            keys
                .iter()
                .map(|h| cells.iter().find(|(k, _)| k == h).map(|(_, v)| v.clone()).unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    (headers, rows)
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn write(v: &Value, to: Format, output: &Path, sheet: &str) -> Result<(), String> {
    let save = |bytes: Vec<u8>| std::fs::write(output, bytes).map_err(|e| format!("Couldn't save {}: {e}", output.display()));
    match to {
        Format::Json => save(format!("{}\n", serde_json::to_string_pretty(v).map_err(|e| e.to_string())?).into_bytes()),
        Format::Yaml => save(serde_norway::to_string(v).map_err(|e| e.to_string())?.into_bytes()),
        Format::Csv | Format::Tsv => {
            let (headers, rows) = table(v);
            let mut w = csv::WriterBuilder::new()
                .delimiter(if to == Format::Tsv { b'\t' } else { b',' })
                .from_writer(Vec::new());
            w.write_record(&headers).map_err(|e| e.to_string())?;
            for row in rows {
                w.write_record(row.iter().map(cell_text)).map_err(|e| e.to_string())?;
            }
            let body = w.into_inner().map_err(|e| e.to_string())?;
            // The byte-order mark makes Excel read accented letters and emoji correctly.
            let mut bytes = if to == Format::Csv { b"\xEF\xBB\xBF".to_vec() } else { Vec::new() };
            bytes.extend(body);
            save(bytes)
        }
        Format::Xlsx => write_xlsx(v, output, sheet),
        Format::Xml => Err("Writing XML isn't supported yet.".into()),
    }
}

fn write_xlsx(v: &Value, output: &Path, sheet: &str) -> Result<(), String> {
    use rust_xlsxwriter::{Format as Style, Workbook};
    let (headers, rows) = table(v);
    let mut book = Workbook::new();
    let ws = book.add_worksheet();
    let name: String = sheet.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if !name.trim().is_empty() {
        let _ = ws.set_name(name.trim());
    }
    let err = |e: rust_xlsxwriter::XlsxError| e.to_string();
    let bold = Style::new().set_bold();
    for (c, h) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, h, &bold).map_err(err)?;
    }
    for (r, row) in rows.iter().enumerate() {
        let r = (r + 1) as u32;
        if r > 1_048_575 {
            return Err("Excel sheets hold at most 1,048,576 rows; this file has more.".into());
        }
        for (c, cell) in row.iter().enumerate() {
            let c = c as u16;
            match cell {
                Value::Null => {}
                Value::Bool(b) => {
                    ws.write_boolean(r, c, *b).map_err(err)?;
                }
                Value::Number(n) => {
                    ws.write_number(r, c, n.as_f64().unwrap_or(0.0)).map_err(err)?;
                }
                other => {
                    let text: String = cell_text(other).chars().take(32_767).collect();
                    ws.write_string(r, c, text).map_err(err)?;
                }
            }
        }
    }
    ws.set_freeze_panes(1, 0).map_err(err)?;
    ws.autofit();
    book.save(output).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_are_inferred_carefully() {
        assert_eq!(infer("42"), json!(42));
        assert_eq!(infer("-3.5"), json!(-3.5));
        assert_eq!(infer("007"), json!("007"));
        assert_eq!(infer("1e5"), json!("1e5"));
        assert_eq!(infer(" 5"), json!(" 5"));
        assert_eq!(infer("TRUE"), json!(true));
        assert_eq!(infer(""), Value::Null);
    }

    #[test]
    fn csv_round_trips_through_json() {
        let v = read("name;age;zip\nAnn;31;02139\nBo;;\n", Format::Csv).unwrap_or_else(|e| panic!("{e}"));
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["age"], json!(31));
        assert_eq!(rows[0]["zip"], json!("02139"));
        let (h, r) = table(&v);
        assert_eq!(h.len(), 3);
        assert_eq!(r[1][1], Value::Null);
    }

    #[test]
    fn nested_json_flattens() {
        let v = json!({"users": [{"name": "Ann", "address": {"city": "Pune"}, "tags": ["a", "b"]}, {"name": "Bo", "extra": 1}]});
        let (h, r) = table(&v);
        assert_eq!(h, ["name", "address.city", "tags", "extra"]);
        assert_eq!(r[0][2], json!("[\"a\",\"b\"]"));
        assert_eq!(r[1][3], json!(1));
    }

    #[test]
    fn xml_becomes_records() {
        let v = read(
            r#"<?xml version="1.0"?><books><book id="1"><title>Dune &amp; more</title><year>1965</year></book><book id="2"><title>Emma</title></book></books>"#,
            Format::Xml,
        )
        .unwrap();
        let (h, r) = table(&v);
        assert_eq!(h, ["id", "title", "year"]);
        assert_eq!(r[0][1], json!("Dune & more"));
        assert_eq!(r[0][2], json!(1965));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn json_lines_are_read() {
        let v = read("{\"a\":1}\n{\"a\":2}\n", Format::Json).unwrap();
        assert_eq!(v, json!([{"a": 1}, {"a": 2}]));
    }

    #[test]
    fn every_output_format_writes() {
        let d = std::env::temp_dir().join(format!("convertino-data-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("people.csv");
        std::fs::write(&src, "name,age\nAnn,31\nZoë,27\n").unwrap();
        for (f, ext) in [(Format::Json, "json"), (Format::Yaml, "yaml"), (Format::Tsv, "tsv"), (Format::Xlsx, "xlsx"), (Format::Csv, "csv")] {
            let out = d.join(format!("out.{ext}"));
            convert(&src, f, &out).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(out.metadata().unwrap().len() > 10, "{ext}");
        }
        let yaml = std::fs::read_to_string(d.join("out.yaml")).unwrap();
        assert!(yaml.contains("Zoë"));
        let back = read(&yaml, Format::Yaml).unwrap();
        assert_eq!(back[1]["age"], json!(27));
        let _ = std::fs::remove_dir_all(&d);
    }
}
