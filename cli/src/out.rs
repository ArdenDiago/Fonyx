//! Output. Every command builds a `serde_json::Value`; `--json` prints it
//! as is, otherwise it is laid out as text: an array of objects becomes a
//! table, an object becomes `key: value` lines.

use serde_json::Value;

/// A number for display: `f32` noise (0.800000011920929) rounded away.
pub fn n(x: f32) -> Value {
    serde_json::json!((f64::from(x) * 1e4).round() / 1e4)
}

/// Linear level as decibels; silence is `null` (JSON has no -inf).
pub fn db(gain: f32) -> Value {
    if gain > 0.0 {
        serde_json::json!((f64::from(crate::resolve::gain_db(gain)) * 10.0).round() / 10.0)
    } else {
        Value::Null
    }
}

pub fn print(value: &Value, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(value).expect("a Value always serializes"));
    } else {
        print!("{}", text(value));
    }
}

pub fn text(value: &Value) -> String {
    match value {
        Value::Array(rows) => table(rows),
        Value::Object(map) => {
            let mut out = String::new();
            // Scalars first, so nested tables don't split the summary.
            for (key, v) in map.iter().filter(|(_, v)| !is_table(v)) {
                out += &format!("{key}: {}\n", cell(v));
            }
            for (key, v) in map.iter().filter(|(_, v)| is_table(v)) {
                out += &format!("\n{key}:\n{}", table(v.as_array().unwrap()));
            }
            out
        }
        other => format!("{}\n", cell(other)),
    }
}

fn is_table(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.first().is_some_and(Value::is_object))
}

fn table(rows: &[Value]) -> String {
    let Some(first) = rows.first().and_then(Value::as_object) else { return "(none)\n".to_owned() };
    let mut lines: Vec<Vec<String>> = vec![first.keys().map(|k| k.to_uppercase()).collect()];
    lines.extend(rows.iter().map(|row| first.keys().map(|k| cell(&row[k])).collect()));
    let widths: Vec<usize> = (0..first.len()).map(|col| lines.iter().map(|l| l[col].chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for line in lines {
        let padded: Vec<String> = line.iter().zip(&widths).map(|(c, w)| format!("{c:<w$}")).collect();
        out += padded.join("  ").trim_end();
        out.push('\n');
    }
    out
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".to_owned(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.to_owned(),
        Value::String(s) => s.clone(),
        Value::Number(_) => v.to_string(),
        Value::Array(items) if items.is_empty() => "-".to_owned(),
        Value::Array(items) => items.iter().map(cell).collect::<Vec<_>>().join(", "),
        Value::Object(map) => map.iter().map(|(k, v)| format!("{k}={}", cell(v))).collect::<Vec<_>>().join(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_list_is_an_aligned_table_and_an_object_is_lines() {
        let rows = json!([{"id": 4, "name": "Default", "live": true}, {"id": 12, "name": "Music", "live": false}]);
        assert_eq!(text(&rows), "ID  NAME     LIVE\n4   Default  yes\n12  Music    no\n");
        assert_eq!(text(&json!([])), "(none)\n");
        let shown = text(&json!({"name": "Music", "bands": [{"band": 1}], "gate": {"hold": 5, "enabled": false}, "inputs": []}));
        assert_eq!(shown, "name: Music\ngate: hold=5 enabled=no\ninputs: -\n\nbands:\nBAND\n1\n");
    }

    #[test]
    fn numbers_lose_their_float_noise() {
        assert_eq!(n(0.8).to_string(), "0.8");
        assert_eq!(db(0.5).to_string(), "-6.0");
        assert_eq!(db(0.0), Value::Null);
    }
}
