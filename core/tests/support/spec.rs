//! Spec-check ledger: each GBATEK statement becomes one row of a
//! `✅/❌` table in the report *and* a real assertion.
//!
//! ```text
//! let mut c = Checks::new();
//! c.eq("DIVCNT mode 0: 7 / 2", "quot = 3", 3, quot);
//! c.known("…", "…", expected, actual, "why it differs");  // documented gap
//! c.write(&mut doc);   // table into the Markdown report
//! c.finish();          // panics listing every unexpected ❌
//! ```

use std::fmt::Debug;

use super::md::{Doc, L};

struct Row {
    item: String,
    rule: String,
    expected: String,
    actual: String,
    status: &'static str,
}

#[derive(Default)]
pub struct Checks {
    rows: Vec<Row>,
    failures: Vec<String>,
}

impl Checks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asserts `actual == expected`; `rule` quotes the GBATEK statement.
    pub fn eq<T: Debug + PartialEq>(&mut self, item: &str, rule: &str, expected: T, actual: T) {
        let ok = expected == actual;
        self.push(item, rule, format!("{expected:?}"), format!("{actual:?}"), ok, false, "");
    }

    /// Same as [`Checks::eq`] but formats values as hex.
    pub fn hex<T: Into<u64> + Copy>(&mut self, item: &str, rule: &str, expected: T, actual: T) {
        let (e, a) = (expected.into(), actual.into());
        self.push(item, rule, format!("0x{e:X}"), format!("0x{a:X}"), e == a, false, "");
    }

    /// Boolean predicate with a free-form description of what was observed.
    pub fn ok(&mut self, item: &str, rule: &str, ok: bool, observed: impl Into<String>) {
        self.push(item, rule, "true".into(), observed.into(), ok, false, "");
    }

    /// A documented deviation from GBATEK: recorded as ⚠️ when it still
    /// differs (does not fail the test) and as ✅ once it is fixed.
    pub fn known<T: Debug + PartialEq>(
        &mut self,
        item: &str,
        rule: &str,
        expected: T,
        actual: T,
        why: &str,
    ) {
        let ok = expected == actual;
        self.push(item, rule, format!("{expected:?}"), format!("{actual:?}"), ok, true, why);
    }

    #[expect(clippy::too_many_arguments)]
    fn push(
        &mut self,
        item: &str,
        rule: &str,
        expected: String,
        actual: String,
        ok: bool,
        known: bool,
        why: &str,
    ) {
        let status = match (ok, known) {
            (true, _) => "✅",
            (false, true) => "⚠️",
            (false, false) => "❌",
        };
        if !ok && !known {
            self.failures.push(format!("{item}: expected {expected}, got {actual} ({rule})"));
        }
        let rule = if why.is_empty() || ok {
            rule.to_string()
        } else {
            format!("{rule} — **known gap:** {why}")
        };
        self.rows.push(Row { item: item.into(), rule, expected, actual, status });
    }

    /// Writes the ledger as a table.
    pub fn write(&self, doc: &mut Doc) {
        let rows: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|r| {
                vec![
                    r.status.into(),
                    r.item.clone(),
                    r.rule.clone(),
                    r.expected.clone(),
                    r.actual.clone(),
                ]
            })
            .collect();
        doc.table(
            &[("", L), ("Check", L), ("GBATEK rule", L), ("Expected", L), ("Lunaris", L)],
            &rows,
        );
        let (pass, gap) = (
            self.rows.iter().filter(|r| r.status == "✅").count(),
            self.rows.iter().filter(|r| r.status == "⚠️").count(),
        );
        doc.p(&format!(
            "**{pass}/{} checks pass**{}.",
            self.rows.len(),
            if gap > 0 { format!(", {gap} documented gap(s) ⚠️") } else { String::new() }
        ));
    }

    /// Panics if any non-`known` check failed.
    pub fn finish(self) {
        assert!(self.failures.is_empty(), "spec violations:\n  {}", self.failures.join("\n  "));
    }
}
