//! Salient tool-result reduction at ingestion (FR-2, Tech Spec §5.3).
//!
//! Mirrors [`crate::truncate`]: a pure, deterministic transform with no I/O
//! and **no model call** (Requirements §8.5). The engine calls
//! [`reduce_output`] *before* [`truncate_output`] so the order is
//! salient-reduction → size backstop (§5.3).
//!
//! Each tool may have a registered reducer that keeps the parts that inform
//! the model's next step and elides the noise *by meaning*, not by position.
//! A tool with no registered reducer falls straight through to the size
//! backstop unchanged. The full, pre-reduction output is preserved in the
//! sidecar by the engine (Tech Spec §3.2); reduction only changes what is
//! sent to the model — never the transcript (HC-7).
//!
//! [`truncate_output`]: crate::truncate_output

// ── Thresholds (initial; tune with use — Requirements §13, Tech Spec §16) ──

/// Lines of `bash` stdout head to keep after collapsing progress lines.
const BASH_STDOUT_HEAD: usize = 60;
/// Lines of `bash` stdout tail to keep after collapsing progress lines.
const BASH_STDOUT_TAIL: usize = 40;
/// Minimum run of consecutive near-identical lines before collapsing.
const PROGRESS_RUN_MIN: usize = 3;
/// Glob path count above which head+tail elision applies.
const GLOB_MAX_PATHS: usize = 50;

// ── Result type ──

/// The outcome of reducing one tool result to its salient content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reduction {
    /// The reduced content to place into the conversation view, with any
    /// reduction markers already embedded when [`reduced`](Self::reduced) is
    /// true.
    pub content: String,
    /// Whether any salient reduction occurred.
    pub reduced: bool,
    /// A brief description of what was withheld and why (e.g. "12 progress
    /// lines collapsed"), for logging and debugging. Empty when not reduced.
    pub withheld: String,
}

impl Reduction {
    /// No reduction — content unchanged, ready for the size backstop.
    #[must_use]
    fn passthrough(raw: &str) -> Self {
        Self {
            content: raw.to_string(),
            reduced: false,
            withheld: String::new(),
        }
    }
}

// ── Entry point ──

/// Reduce a tool result to its salient content by meaning (FR-2, Tech Spec
/// §5.3). Dispatches to a per-tool reducer keyed by tool name; a tool with no
/// registered reducer falls straight through unchanged to the size backstop.
///
/// Pure and deterministic — no I/O, no model call (Requirements §8.5).
#[must_use]
pub fn reduce_output(tool_name: &str, raw: &str) -> Reduction {
    match tool_name {
        "bash" => reduce_bash(raw),
        // All hits are kept (§5.3 — "hits are the point"); no semantic reducer.
        "grep" => Reduction::passthrough(raw),
        "glob" => reduce_glob(raw),
        // read_file: no semantic reducer (§5.3) — already bounded by
        // start_line/end_line and the size backstop; reducing by meaning
        // would risk hiding code the model asked for.
        _ => Reduction::passthrough(raw),
    }
}

// ── bash ──

/// Reduce `bash` output (Tech Spec §5.3): keep the exit status, **all** of
/// stderr, and head+tail of stdout — collapsing runs of near-identical
/// progress/percentage lines first.
fn reduce_bash(raw: &str) -> Reduction {
    let lines: Vec<&str> = raw.lines().collect();

    // Locate section markers (exact line match, robust against markers
    // appearing as content in other sections — `position` finds the first).
    let stdout_idx = lines.iter().position(|l| *l == "--- stdout ---");
    let stderr_idx = lines.iter().position(|l| *l == "--- stderr ---");

    let stdout_start = stdout_idx.map(|i| i + 1);
    let stdout_end = stderr_idx.unwrap_or(lines.len());

    let stdout_lines: &[&str] = match stdout_start {
        Some(s) if s <= stdout_end => &lines[s..stdout_end],
        _ => &[],
    };
    let stderr_lines: &[&str] = match stderr_idx {
        Some(i) => &lines[i + 1..],
        None => &[],
    };
    let header_end = stdout_idx.or(stderr_idx).unwrap_or(lines.len());
    let header: &[&str] = &lines[..header_end];

    // Reduce stdout: collapse progress lines, then head+tail if still long.
    let (reduced_stdout, withheld) = reduce_bash_stdout(stdout_lines);
    if withheld.is_empty() {
        return Reduction::passthrough(raw);
    }

    // Reassemble: header + reduced stdout + full stderr.
    let mut content = String::new();
    for line in header {
        content.push_str(line);
        content.push('\n');
    }
    if stdout_idx.is_some() {
        content.push_str("--- stdout ---\n");
        for line in &reduced_stdout {
            content.push_str(line);
            content.push('\n');
        }
    }
    if stderr_idx.is_some() {
        content.push_str("--- stderr ---\n");
        for line in stderr_lines {
            content.push_str(line);
            content.push('\n');
        }
    }

    Reduction {
        content,
        reduced: true,
        withheld: withheld.join(", "),
    }
}

/// Collapse near-identical progress lines in stdout, then apply head+tail if
/// still over budget. Returns the reduced lines and human-readable
/// descriptions of what was withheld.
fn reduce_bash_stdout(lines: &[&str]) -> (Vec<String>, Vec<String>) {
    let (collapsed, n_collapsed) = collapse_near_identical(lines);

    let budget = BASH_STDOUT_HEAD + BASH_STDOUT_TAIL;
    let (final_lines, n_elided) = if collapsed.len() > budget {
        let elided = collapsed.len() - budget;
        let head = &collapsed[..BASH_STDOUT_HEAD];
        let tail = &collapsed[collapsed.len() - BASH_STDOUT_TAIL..];
        let mut result = head.to_vec();
        result.push(reduction_marker(&format!(
            "{elided} stdout lines elided (kept head+tail)"
        )));
        result.extend_from_slice(tail);
        (result, elided)
    } else {
        (collapsed, 0)
    };

    let mut withheld = Vec::new();
    if n_collapsed > 0 {
        withheld.push(format!("{n_collapsed} progress lines collapsed"));
    }
    if n_elided > 0 {
        withheld.push(format!("{n_elided} stdout lines elided"));
    }

    (final_lines, withheld)
}

/// Group consecutive lines whose [`normalize_for_comparison`] key matches.
/// Runs of [`PROGRESS_RUN_MIN`]+ lines are collapsed to first + marker +
/// last; shorter runs pass through untouched. Returns the reduced lines and
/// the total count of collapsed (withheld) lines.
fn collapse_near_identical(lines: &[&str]) -> (Vec<String>, usize) {
    let mut result: Vec<String> = Vec::new();
    let mut total_collapsed: usize = 0;
    let mut i = 0;

    while i < lines.len() {
        let key = normalize_for_comparison(lines[i]);
        let mut run_end = i + 1;
        while run_end < lines.len() && normalize_for_comparison(lines[run_end]) == key {
            run_end += 1;
        }
        let run_len = run_end - i;

        if run_len >= PROGRESS_RUN_MIN {
            let collapsed_count = run_len - 2;
            total_collapsed += collapsed_count;
            result.push(lines[i].to_string());
            result.push(reduction_marker(&format!(
                "{collapsed_count} similar lines collapsed"
            )));
            result.push(lines[run_end - 1].to_string());
        } else {
            for line in &lines[i..run_end] {
                result.push(line.to_string());
            }
        }
        i = run_end;
    }

    (result, total_collapsed)
}

// ── glob ──

/// Reduce `glob` output (Tech Spec §5.3): keep the path list, but if it
/// exceeds [`GLOB_MAX_PATHS`] paths, keep head+tail with a reduction marker.
fn reduce_glob(raw: &str) -> Reduction {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() <= GLOB_MAX_PATHS {
        return Reduction::passthrough(raw);
    }

    let head_count = GLOB_MAX_PATHS / 2;
    let tail_count = GLOB_MAX_PATHS - head_count;
    let elided = lines.len() - GLOB_MAX_PATHS;

    let mut content = String::new();
    for line in &lines[..head_count] {
        content.push_str(line);
        content.push('\n');
    }
    content.push_str(&reduction_marker(&format!(
        "{elided} paths elided (kept head+tail)"
    )));
    content.push('\n');
    for line in &lines[lines.len() - tail_count..] {
        content.push_str(line);
        content.push('\n');
    }

    Reduction {
        content,
        reduced: true,
        withheld: format!("{elided} paths elided"),
    }
}

// ── Shared helpers ──

/// Build a reduction marker — distinct in wording from the size-truncation
/// marker ([`crate::truncate`]) so the model can tell *reduced-by-meaning*
/// from *trimmed-by-size*. Names what was withheld and offers `/view` to
/// recover the full output (Design §4.3/§8.6).
#[must_use]
fn reduction_marker(description: &str) -> String {
    format!("[reduced: {description} — /view for full output]")
}

/// Normalize a line for near-identical comparison: replace each maximal digit
/// sequence with a single `#`, collapse runs of the same non-alphanumeric
/// character (progress bars), and collapse whitespace. Lines that differ only
/// in their numeric or progress-bar content (e.g. "10%", "20%") normalize to
/// the same key.
fn normalize_for_comparison(line: &str) -> String {
    let mut result = String::new();
    let mut in_digits = false;
    let mut prev_was_space = false;
    let mut prev_char: Option<char> = None;

    for c in line.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                result.push('#');
                in_digits = true;
            }
            prev_was_space = false;
            prev_char = Some('#');
        } else if c.is_whitespace() {
            in_digits = false;
            prev_was_space = true;
            prev_char = None;
        } else {
            in_digits = false;
            let is_repeat = prev_char == Some(c) && !c.is_alphanumeric();
            if !is_repeat {
                if prev_was_space && !result.is_empty() {
                    result.push(' ');
                }
                result.push(c);
            }
            prev_was_space = false;
            prev_char = Some(c);
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── dispatch / passthrough ──

    #[test]
    fn unregistered_tool_passes_through_unchanged() {
        let raw = "line one\nline two\n";
        let r = reduce_output("read_file", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
        assert!(r.withheld.is_empty());
    }

    #[test]
    fn unknown_tool_passes_through_unchanged() {
        let raw = "some output\n";
        let r = reduce_output("totally_unknown_tool", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn grep_passes_through_unchanged() {
        let raw = "src/main.rs:42:let x = 1;\nsrc/lib.rs:7:let y = 2;\n";
        let r = reduce_output("grep", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn empty_output_passes_through() {
        let r = reduce_output("bash", "");
        assert!(!r.reduced);
        assert!(r.content.is_empty());
    }

    #[test]
    fn reduction_marker_is_distinct_from_truncation_marker() {
        let m = reduction_marker("3 progress lines collapsed");
        assert!(m.starts_with("[reduced:"));
        assert!(m.contains("/view"));
        assert!(!m.starts_with("[..."));
    }

    // ── bash reducer ──

    #[test]
    fn bash_short_output_passthrough() {
        let raw = "exit code: 0\n--- stdout ---\nhello\n";
        let r = reduce_output("bash", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn bash_collapses_progress_lines() {
        let raw = "exit code: 0\n\
                   --- stdout ---\n\
                   Building 1%\n\
                   Building 2%\n\
                   Building 3%\n\
                   Building 4%\n\
                   Building 5%\n\
                   Done\n";
        let r = reduce_output("bash", raw);
        assert!(r.reduced);
        assert!(r.content.contains("exit code: 0"));
        assert!(r.content.contains("Building 1%"), "first of run kept");
        assert!(r.content.contains("Building 5%"), "last of run kept");
        assert!(r.content.contains("Done"));
        assert!(r.content.contains("[reduced:"));
        assert!(r.content.contains("similar lines collapsed"));
        // Middle progress lines withheld.
        assert!(
            !r.content.contains("Building 2%\n")
                && !r.content.contains("Building 3%\n")
                && !r.content.contains("Building 4%\n"),
            "middle progress lines should be collapsed"
        );
        assert!(r.withheld.contains("progress lines collapsed"));
    }

    #[test]
    fn bash_preserves_exit_code_and_stderr() {
        let raw = "exit code: 1\n\
                   --- stdout ---\n\
                   1%\n2%\n3%\n4%\n5%\n\
                   --- stderr ---\n\
                   error: something broke\n\
                   warning: deprecated API\n";
        let r = reduce_output("bash", raw);
        assert!(r.reduced);
        assert!(r.content.contains("exit code: 1"));
        // All of stderr is preserved.
        assert!(r.content.contains("error: something broke"));
        assert!(r.content.contains("warning: deprecated API"));
        assert!(r.content.contains("--- stderr ---"));
    }

    #[test]
    fn bash_head_tail_long_unique_stdout() {
        // 150 lines that normalize differently (different letters) — collapse
        // won't fire, but head+tail will.
        let mut stdout: Vec<String> = Vec::new();
        for i in 0..150u32 {
            let letter = (b'a' + (i as u8 % 26)) as char;
            stdout.push(format!("{letter}_{i}"));
        }
        let raw = format!("exit code: 0\n--- stdout ---\n{}\n", stdout.join("\n"));
        let r = reduce_output("bash", &raw);
        assert!(r.reduced);
        assert!(r.content.contains("a_0"), "head kept");
        assert!(r.content.contains("b_1"), "head kept");
        assert!(r.content.contains("t_149"), "tail kept");
        assert!(r.content.contains("[reduced:"));
        assert!(r.content.contains("stdout lines elided"));
        // Middle lines gone.
        assert!(!r.content.contains("c_80\n"), "middle should be elided");
        assert!(r.withheld.contains("stdout lines elided"));
    }

    #[test]
    fn bash_no_sections_passthrough() {
        let raw = "just some text\nno markers here\n";
        let r = reduce_output("bash", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn bash_empty_stdout_passthrough() {
        let raw = "exit code: 0\n--- stderr ---\nerror\n";
        let r = reduce_output("bash", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn bash_collapse_then_head_tail() {
        // Progress lines that collapse + enough unique lines to trigger head/tail.
        let mut stdout: Vec<String> = Vec::new();
        // 50 progress lines (collapse to ~3).
        for i in 1..=50 {
            stdout.push(format!("Downloading {i}%"));
        }
        // 120 unique lines (trigger head/tail after collapse).
        for i in 0..120u32 {
            let letter = (b'a' + (i as u8 % 26)) as char;
            stdout.push(format!("{letter}_{i}"));
        }
        let raw = format!("exit code: 0\n--- stdout ---\n{}\n", stdout.join("\n"));
        let r = reduce_output("bash", &raw);
        assert!(r.reduced);
        assert!(r.content.contains("Downloading 1%"), "first progress kept");
        assert!(r.content.contains("Downloading 50%"), "last progress kept");
        assert!(r.withheld.contains("progress lines collapsed"));
        assert!(r.withheld.contains("stdout lines elided"));
    }

    // ── glob reducer ──

    #[test]
    fn glob_short_list_passthrough() {
        let raw = "src/main.rs\nsrc/lib.rs\nsrc/mod.rs\n";
        let r = reduce_output("glob", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn glob_head_tail_on_overflow() {
        let paths: Vec<String> = (0..100).map(|i| format!("dir/file_{i}.rs")).collect();
        let raw = paths.join("\n");
        let r = reduce_output("glob", &raw);
        assert!(r.reduced);
        assert!(r.content.contains("dir/file_0.rs"), "head kept");
        assert!(r.content.contains("dir/file_99.rs"), "tail kept");
        assert!(r.content.contains("[reduced:"));
        assert!(r.content.contains("paths elided"));
        // Middle paths gone.
        assert!(
            !r.content.contains("dir/file_50.rs\n"),
            "middle should be elided"
        );
        assert!(r.withheld.contains("paths elided"));
    }

    #[test]
    fn glob_no_files_match_passthrough() {
        let raw = "no files match **/*.xyz";
        let r = reduce_output("glob", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    // ── normalize_for_comparison ──

    #[test]
    fn normalize_groups_numeric_variants() {
        assert_eq!(
            normalize_for_comparison("  Downloaded 45 of 200 packages"),
            normalize_for_comparison("  Downloaded 67 of 200 packages")
        );
        assert_eq!(
            normalize_for_comparison("Building 1%"),
            normalize_for_comparison("Building 99%")
        );
    }

    #[test]
    fn normalize_preserves_textual_differences() {
        assert_ne!(
            normalize_for_comparison("Compiling crate-a v0.1.0"),
            normalize_for_comparison("Compiling libc v0.2.155")
        );
    }

    #[test]
    fn normalize_collapses_progress_bars() {
        // Progress bars of different lengths should normalize identically.
        assert_eq!(
            normalize_for_comparison("  45% |███          | 23/50"),
            normalize_for_comparison("  67% |██████       | 31/50")
        );
    }
}
