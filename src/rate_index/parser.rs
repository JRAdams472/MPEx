//! MND Daily Mortgage Rate Index parser. The index table is server-rendered
//! HTML: the `mtg-rates` table holds one `<tr>` per product — a bare `<th>`
//! label immediately followed by the current rate in the first `<td>`
//! (e.g. `7.29%`). All-or-nothing: a missing label or implausible value
//! fails the whole parse, so a changed page shape can never write a
//! partial sheet.

use std::collections::BTreeMap;

/// The six index products we track, in MND display order.
pub const INDEX_LABELS: [&str; 6] = [
    "30 Yr. Fixed",
    "15 Yr. Fixed",
    "30 Yr. FHA",
    "30 Yr. Jumbo",
    "7/6 SOFR ARM",
    "30 Yr. VA",
];

/// Plausibility window for a mortgage index rate — catches markup drift
/// that happens to surface a number in the wrong cell.
const MIN_RATE: f64 = 0.5;
const MAX_RATE: f64 = 25.0;

/// First `<table>` whose open tag carries the `mtg-rates` class.
fn index_table(html: &str) -> Result<&str, String> {
    let open = regex::Regex::new(r"(?s)<table[^>]*mtg-rates[^>]*>")
        .expect("table regex compiles")
        .find(html)
        .ok_or("rate index table (mtg-rates) not found")?;
    let rest = &html[open.end()..];
    let end = rest
        .find("</table>")
        .ok_or("rate index table is unterminated")?;
    Ok(&rest[..end])
}

/// Bare `<th>\s*LABEL\s*</th>` immediately followed by `<td>NN.NN%`. The
/// bare-tag requirement keeps detail/comparison tables on the page (whose
/// headers carry class/colspan attributes) out of scope.
fn row_re(label: &str) -> regex::Regex {
    regex::Regex::new(&format!(
        r"<th>\s*{}\s*</th>\s*<td>\s*([0-9]+(?:\.[0-9]+)?)\s*%",
        regex::escape(label)
    ))
    .expect("label regex compiles")
}

/// Extracts every tracked label → current rate. Err on the first missing
/// or implausible row.
pub fn parse_index(html: &str) -> Result<BTreeMap<String, f64>, String> {
    let table = index_table(html)?;
    let mut rates = BTreeMap::new();
    for label in INDEX_LABELS {
        let caps = row_re(label)
            .captures(table)
            .ok_or_else(|| format!("index row for {label:?} not found"))?;
        let rate: f64 = caps[1]
            .parse()
            .map_err(|_| format!("index rate for {label:?} is not numeric"))?;
        if !(MIN_RATE..MAX_RATE).contains(&rate) {
            return Err(format!("index rate for {label:?} out of range: {rate}"));
        }
        rates.insert(label.to_string(), rate);
    }
    Ok(rates)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/mnd_rates.html");

    #[test]
    fn parses_all_six_products() {
        let rates = parse_index(FIXTURE).expect("fixture parses");
        assert_eq!(rates.len(), 6);
        assert_eq!(rates["30 Yr. Fixed"], 7.29);
        assert_eq!(rates["15 Yr. Fixed"], 6.95);
        assert_eq!(rates["30 Yr. FHA"], 6.91);
        assert_eq!(rates["30 Yr. Jumbo"], 7.55);
        assert_eq!(rates["7/6 SOFR ARM"], 6.88);
        assert_eq!(rates["30 Yr. VA"], 6.92);
    }

    #[test]
    fn missing_label_fails_whole_parse() {
        let broken = FIXTURE.replace("30 Yr. Jumbo", "30 Yr. Jmbo");
        let err = parse_index(&broken).unwrap_err();
        assert!(err.contains("30 Yr. Jumbo"), "{err}");
    }

    #[test]
    fn missing_table_fails() {
        let err = parse_index("<html><body>no rates here</body></html>").unwrap_err();
        assert!(err.contains("mtg-rates"), "{err}");
    }

    #[test]
    fn implausible_rate_fails() {
        let broken = FIXTURE.replace("<td>7.29%</td>", "<td>72.9%</td>");
        let err = parse_index(&broken).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
    }
}
