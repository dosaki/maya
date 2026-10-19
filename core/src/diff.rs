//! Line diffs for the session viewer: an edit's old and new text, or the
//! patch Claude Code recorded, as added, removed and unchanged lines.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Above this many old×new line pairs the diff stops matching lines and
/// shows every old line removed, then every new one added: the table would
/// cost too much for a history read that runs every second.
pub const MAX_DIFF_CELLS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffOp {
    Add,
    Del,
    Same,
    /// An op this Maya does not know, from a newer one: shown unchanged.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub op: DiffOp,
    /// Its number in the new file (the old one for a removed line), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub text: String,
}

impl DiffLine {
    pub fn new(op: DiffOp, line: Option<u32>, text: &str) -> DiffLine {
        DiffLine { op, line, text: text.to_string() }
    }
}

/// (added, removed) lines.
pub fn counts(diff: &[DiffLine]) -> (u32, u32) {
    diff.iter().fold((0, 0), |(a, r), l| match l.op {
        DiffOp::Add => (a + 1, r),
        DiffOp::Del => (a, r + 1),
        DiffOp::Same | DiffOp::Other => (a, r),
    })
}

/// `old` against `new` line by line: common lines kept, the rest removed
/// then added. No line numbers: an edit's strings do not say where in the
/// file they sit.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Most edits change a few lines in the middle: match the ends first.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let same = |l: &&str| DiffLine::new(DiffOp::Same, None, l);
    let mut out: Vec<DiffLine> = a[..head].iter().map(same).collect();
    out.extend(middle(&a[head..a.len() - tail], &b[head..b.len() - tail]));
    out.extend(a[a.len() - tail..].iter().map(same));
    out
}

/// The changed middle: a longest-common-subsequence walk, or every line
/// removed then added when either side is empty or the table is too big.
fn middle(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_DIFF_CELLS {
        return a.iter().map(|l| DiffLine::new(DiffOp::Del, None, l)).chain(b.iter().map(|l| DiffLine::new(DiffOp::Add, None, l))).collect();
    }
    // lcs[i * (m + 1) + j]: the longest common subsequence of a[i..] and b[j..].
    let at = |i: usize, j: usize| i * (m + 1) + j;
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n + m);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine::new(DiffOp::Same, None, a[i]));
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            out.push(DiffLine::new(DiffOp::Del, None, a[i]));
            i += 1;
        } else {
            out.push(DiffLine::new(DiffOp::Add, None, b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| DiffLine::new(DiffOp::Del, None, l)));
    out.extend(b[j..].iter().map(|l| DiffLine::new(DiffOp::Add, None, l)));
    out
}

/// Claude Code's `structuredPatch` (hunks of `oldStart`, `newStart` and
/// `lines` led by ' ', '-' or '+') as numbered lines: a removed line carries
/// its old number, the others their new one. None unless it holds at least
/// one line of the file.
pub fn from_patch(hunks: &Value) -> Option<Vec<DiffLine>> {
    let hunks = hunks.as_array().filter(|h| !h.is_empty())?;
    let mut out = Vec::new();
    for h in hunks {
        let mut old = h["oldStart"].as_u64().unwrap_or(1) as u32;
        let mut new = h["newStart"].as_u64().unwrap_or(1) as u32;
        for l in h["lines"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            // Anything else ("\ No newline at end of file") is not a line of the file.
            let op = match l.chars().next() {
                Some('+') => DiffOp::Add,
                Some('-') => DiffOp::Del,
                Some(' ') => DiffOp::Same,
                _ => continue,
            };
            let text = &l[1..];
            match op {
                DiffOp::Del => {
                    out.push(DiffLine::new(op, Some(old), text));
                    old += 1;
                }
                DiffOp::Add => {
                    out.push(DiffLine::new(op, Some(new), text));
                    new += 1;
                }
                DiffOp::Same | DiffOp::Other => {
                    out.push(DiffLine::new(op, Some(new), text));
                    old += 1;
                    new += 1;
                }
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Every line of `text` added, numbered from 1: a new file.
pub fn numbered_additions(text: &str) -> Vec<DiffLine> {
    text.lines().enumerate().map(|(i, l)| DiffLine::new(DiffOp::Add, Some(i as u32 + 1), l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(d: &[DiffLine]) -> Vec<(DiffOp, Option<u32>, &str)> {
        d.iter().map(|l| (l.op, l.line, l.text.as_str())).collect()
    }

    #[test]
    fn an_edit_keeps_common_lines_and_marks_the_rest() {
        let d = line_diff("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(ops(&d), vec![(DiffOp::Same, None, "a"), (DiffOp::Del, None, "b"), (DiffOp::Add, None, "B"), (DiffOp::Same, None, "c"), (DiffOp::Add, None, "d")]);
        assert_eq!(counts(&d), (2, 1));
    }

    #[test]
    fn identical_empty_and_one_sided_texts() {
        assert_eq!(ops(&line_diff("x\ny", "x\ny")), vec![(DiffOp::Same, None, "x"), (DiffOp::Same, None, "y")]);
        assert_eq!(ops(&line_diff("", "a\nb")), vec![(DiffOp::Add, None, "a"), (DiffOp::Add, None, "b")]);
        assert_eq!(ops(&line_diff("a", "")), vec![(DiffOp::Del, None, "a")]);
        assert!(line_diff("", "").is_empty());
    }

    #[test]
    fn a_moved_line_is_matched_by_the_longest_common_run() {
        // b and c stay; a moves from the top to the bottom.
        let d = line_diff("a\nb\nc", "b\nc\na");
        assert_eq!(ops(&d), vec![(DiffOp::Del, None, "a"), (DiffOp::Same, None, "b"), (DiffOp::Same, None, "c"), (DiffOp::Add, None, "a")]);
        assert_eq!(counts(&d), (1, 1));
    }

    #[test]
    fn a_huge_edit_falls_back_to_all_removed_then_all_added() {
        let old = (0..5000).map(|i| format!("o{i}")).collect::<Vec<_>>().join("\n");
        let new = (0..5000).map(|i| format!("n{i}")).collect::<Vec<_>>().join("\n");
        let d = line_diff(&old, &new);
        assert_eq!(d.len(), 10_000);
        assert!(d[..5000].iter().all(|l| l.op == DiffOp::Del));
        assert!(d[5000..].iter().all(|l| l.op == DiffOp::Add));
        assert_eq!(counts(&d), (5000, 5000));
    }

    #[test]
    fn a_recorded_patch_numbers_its_lines() {
        let patch = serde_json::json!([
            {"oldStart": 3, "oldLines": 3, "newStart": 3, "newLines": 4, "lines": ["-fn mian() {", "+fn main() {", "     run();", "+    done();", " }", "\\ No newline at end of file"]},
            {"oldStart": 40, "oldLines": 1, "newStart": 41, "newLines": 0, "lines": ["-gone"]},
        ]);
        assert_eq!(ops(&from_patch(&patch).unwrap()), vec![
            (DiffOp::Del, Some(3), "fn mian() {"),
            (DiffOp::Add, Some(3), "fn main() {"),
            (DiffOp::Same, Some(4), "    run();"),
            (DiffOp::Add, Some(5), "    done();"),
            (DiffOp::Same, Some(6), "}"),
            (DiffOp::Del, Some(40), "gone"),
        ]);
        assert_eq!(from_patch(&serde_json::json!([])), None, "a new file's patch is empty");
        assert_eq!(from_patch(&serde_json::Value::Null), None);
    }

    #[test]
    fn a_patch_with_no_lines_of_the_file_is_none() {
        let only_marker = serde_json::json!([{"oldStart": 1, "newStart": 1, "lines": ["\\ No newline at end of file"]}]);
        assert_eq!(from_patch(&only_marker), None);
        assert_eq!(from_patch(&serde_json::json!([{"oldStart": 1, "newStart": 1}])), None);
    }

    #[test]
    fn an_op_from_a_newer_maya_reads_as_other_and_counts_as_neither() {
        let d: Vec<DiffLine> = serde_json::from_value(serde_json::json!([{"op": "moved", "line": 2, "text": "x"}, {"op": "add", "text": "y"}])).unwrap();
        assert_eq!(ops(&d), vec![(DiffOp::Other, Some(2), "x"), (DiffOp::Add, None, "y")]);
        assert_eq!(counts(&d), (1, 0));
    }

    #[test]
    fn a_new_file_is_every_line_added_from_one() {
        assert_eq!(ops(&numbered_additions("# Notes\n\nfirst\n")), vec![(DiffOp::Add, Some(1), "# Notes"), (DiffOp::Add, Some(2), ""), (DiffOp::Add, Some(3), "first")]);
    }
}
