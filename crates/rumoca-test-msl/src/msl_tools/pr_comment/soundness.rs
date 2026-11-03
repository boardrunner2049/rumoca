//! The "Simulation Soundness Roster" section of the MSL PR comment
//! (SPEC_0033 simulation soundness, SPEC_0025 PR report).
//!
//! Lists every model that simulated without strict-high parity and without a
//! typed trace exception, grouped by triage package so each parity lane reads
//! its own rows. A model missing from the baseline roster is marked `new`:
//! the roster only shrinks, so such a row fails the quality gate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::msl_tools::band_table::{self, BandRow};

pub(super) fn render_soundness_section(
    results_dir: &Path,
    baseline: Option<&Value>,
) -> Result<String> {
    let path = band_table::band_table_path(results_dir);
    if !path.is_file() {
        return Ok("_No band table was produced in this run._\n".to_string());
    }
    let table = band_table::load_band_table(&path)?;
    let roster = baseline.map(baseline_roster);
    Ok(render_rows(
        table.unexcepted_non_high_rows(),
        roster.as_ref(),
    ))
}

fn baseline_roster(baseline: &Value) -> BTreeSet<String> {
    baseline
        .get("unexcepted_non_high_models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn render_rows<'a>(
    rows: impl Iterator<Item = &'a BandRow>,
    roster: Option<&BTreeSet<String>>,
) -> String {
    let mut by_package = BTreeMap::<&str, Vec<&BandRow>>::new();
    for row in rows {
        by_package
            .entry(band_table::triage_package(&row.model_name))
            .or_default()
            .push(row);
    }
    let total = by_package.values().map(Vec::len).sum::<usize>();
    let mut text = match roster {
        Some(roster) => format!(
            "{total} model(s) simulated without strict-high parity or a typed exception \
             (baseline roster: {}; the target is 0 and the roster only shrinks).\n\n",
            roster.len()
        ),
        None => format!(
            "{total} model(s) simulated without strict-high parity or a typed exception \
             (no baseline roster to compare).\n\n"
        ),
    };
    for (package, rows) in by_package {
        text.push_str(&format!("**{package}** ({})\n\n", rows.len()));
        for row in rows {
            let marker = match roster {
                Some(roster) if !roster.contains(&row.model_name) => " **new**",
                Some(_) | None => "",
            };
            text.push_str(&format!(
                "- `{}`{marker}: {}\n",
                row.model_name,
                row.describe()
            ));
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msl_tools::band_table::{BandLabel, ExitReason};

    fn row(model_name: &str, band: BandLabel, exit_reason: Option<ExitReason>) -> BandRow {
        let mut row: BandRow = serde_json::from_value(serde_json::json!({
            "model_name": model_name,
            "band": band.as_str(),
        }))
        .expect("row");
        row.exit_reason = exit_reason;
        row
    }

    #[test]
    fn roster_rows_group_by_triage_package_and_mark_new_entries() {
        let rows = [
            row(
                "Modelica.Electrical.Analog.Examples.A",
                BandLabel::Near,
                None,
            ),
            row(
                "Modelica.Electrical.Analog.Examples.B",
                BandLabel::Deviation,
                None,
            ),
            row(
                "Modelica.Clocked.Examples.C",
                BandLabel::Absent,
                Some(ExitReason::ReferenceMissing),
            ),
        ];
        let roster = BTreeSet::from([
            "Modelica.Electrical.Analog.Examples.A".to_string(),
            "Modelica.Clocked.Examples.C".to_string(),
        ]);
        let text = render_rows(rows.iter(), Some(&roster));
        assert!(text.starts_with("3 model(s)"), "{text}");
        assert!(text.contains("(baseline roster: 2;"), "{text}");
        assert!(
            text.contains("**Modelica.Electrical.Analog** (2)"),
            "{text}"
        );
        assert!(text.contains("**Modelica.Clocked.Examples** (1)"), "{text}");
        assert!(
            text.contains("`Modelica.Electrical.Analog.Examples.B` **new**"),
            "{text}"
        );
        assert!(
            !text.contains("`Modelica.Electrical.Analog.Examples.A` **new**"),
            "{text}"
        );
    }
}
