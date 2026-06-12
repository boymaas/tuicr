//! gitsigns-style "final document" rendering for the single-column diff view.
//!
//! Document view shares the whole single-column walk in [`super::diff_unified`]
//! (review section, file/hunk headers, gaps, comments, the paint tail). This
//! module owns only what is genuinely different: classifying each new-side line
//! into a gutter sign (add / change / delete) via deletion/addition run pairing,
//! and drawing one document row from that classification. Deletions are hidden
//! behind a gutter marker until their hunk is revealed (see
//! `App::toggle_hunk_diff_reveal_at_cursor`).

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::model::{DiffLine, LineOrigin};
use crate::theme::Theme;
use crate::ui::diff_view::cursor_indicator;
use crate::ui::styles;

/// Amber bar for a changed line. There is no dedicated theme colour for
/// "changed" (only add/del), so this fixed amber stands in; it reads distinctly
/// against the green add and red delete bars on both light and dark themes.
const CHANGE_SIGN: Color = Color::Rgb(220, 180, 60);

/// gitsigns gutter classification for one rendered document row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocSign {
    /// Unchanged context line — no gutter bar.
    Context,
    /// A purely added line (no deletion it replaced).
    Add,
    /// An added line that replaced a deletion (a "modified" line).
    Change,
    /// A deletion line, only emitted when its hunk is revealed.
    Delete,
}

/// One planned document row: which line of the hunk it draws, its sign, and
/// whether a hidden pure-deletion run sits immediately above it (drawn as a
/// gutter marker rather than its own row, to keep the row stream in lockstep
/// with the annotation builder).
#[derive(Debug, Clone, Copy)]
pub(super) struct DocRow {
    pub line_idx: usize,
    pub sign: DocSign,
    pub delete_above: bool,
}

/// Classify a hunk's lines into the document rows to render.
///
/// Pairs each deletion run with the addition run that follows it: paired
/// additions are `Change`, surplus additions are `Add`, and surplus/unpaired
/// deletions become a `delete_above` marker on the next emitted row (or, when
/// the hunk is revealed, their own `Delete` rows). The emitted row set exactly
/// mirrors `build_unified_diff_annotations` with `skip_deletions = !revealed`:
/// a row per line whose `new_lineno` is set, plus deletion rows when revealed.
pub(super) fn compute_hunk_signs(lines: &[DiffLine], revealed: bool) -> Vec<DocRow> {
    let mut rows = Vec::with_capacity(lines.len());
    let mut pending_delete_marker = false;
    let mut i = 0;

    while i < lines.len() {
        match lines[i].origin {
            LineOrigin::Context => {
                rows.push(DocRow {
                    line_idx: i,
                    sign: DocSign::Context,
                    delete_above: take(&mut pending_delete_marker),
                });
                i += 1;
            }
            LineOrigin::Addition => {
                // Addition run with no preceding deletion: pure adds.
                rows.push(DocRow {
                    line_idx: i,
                    sign: DocSign::Add,
                    delete_above: take(&mut pending_delete_marker),
                });
                i += 1;
            }
            LineOrigin::Deletion => {
                // Consume the deletion run, then the addition run that follows.
                let del_start = i;
                while i < lines.len() && lines[i].origin == LineOrigin::Deletion {
                    i += 1;
                }
                let del_count = i - del_start;
                let add_start = i;
                while i < lines.len() && lines[i].origin == LineOrigin::Addition {
                    i += 1;
                }
                let add_count = i - add_start;

                if revealed {
                    for k in del_start..add_start {
                        rows.push(DocRow {
                            line_idx: k,
                            sign: DocSign::Delete,
                            delete_above: false,
                        });
                    }
                }
                for (j, k) in (add_start..i).enumerate() {
                    let sign = if j < del_count {
                        DocSign::Change
                    } else {
                        DocSign::Add
                    };
                    rows.push(DocRow {
                        line_idx: k,
                        sign,
                        delete_above: false,
                    });
                }

                // Deletions with no addition to absorb them (or surplus over the
                // additions) are invisible in the collapsed document, so mark
                // the next emitted row. Revealed hunks already show them as rows.
                if !revealed && del_count > add_count {
                    pending_delete_marker = true;
                }
            }
        }
    }

    rows
}

fn take(flag: &mut bool) -> bool {
    std::mem::replace(flag, false)
}

/// Draw one document row into the line buffer, honouring the off-screen
/// fast-path (a cheap placeholder keeps `line_idx` aligned with the annotation
/// stream without building spans for rows outside the viewport).
#[allow(clippy::too_many_arguments)]
pub(super) fn render_document_line(
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    diff_line: &DiffLine,
    row: DocRow,
    theme: &Theme,
    lw: usize,
    visible_start: usize,
    visible_end: usize,
) {
    if *line_idx < visible_start || *line_idx >= visible_end {
        lines.push(Line::default());
        *line_idx += 1;
        return;
    }

    // Line number: new side for everything except a revealed deletion.
    let blank = " ".repeat(lw + 1);
    let lineno = match row.sign {
        DocSign::Delete => diff_line.old_lineno,
        _ => diff_line.new_lineno.or(diff_line.old_lineno),
    };
    let line_num_str = lineno
        .map(|n| format!("{n:>lw$} "))
        .unwrap_or_else(|| blank.clone());

    // Gutter sign. A hidden deletion above wins the cell on an otherwise
    // unmarked context line so the removal stays visible.
    let (sign_char, sign_style) = if row.delete_above && row.sign == DocSign::Context {
        ("▔", Style::default().fg(theme.diff_del))
    } else {
        match row.sign {
            DocSign::Context => (" ", styles::diff_context_style(theme)),
            DocSign::Add => ("▌", Style::default().fg(theme.diff_add)),
            DocSign::Change => ("▌", Style::default().fg(CHANGE_SIGN)),
            DocSign::Delete => ("▌", Style::default().fg(theme.diff_del)),
        }
    };

    let content_style = match row.sign {
        DocSign::Delete => styles::diff_del_style(theme),
        _ => styles::diff_context_style(theme),
    };

    let mut spans = vec![
        Span::styled(
            cursor_indicator(*line_idx, current_line_idx),
            styles::current_line_indicator_style(theme),
        ),
        Span::styled(line_num_str, styles::dim_style(theme)),
        Span::styled(format!("{sign_char} "), sign_style),
    ];

    if let Some(highlighted) = &diff_line.highlighted_spans {
        for (span_style, span_text) in highlighted {
            spans.push(Span::styled(span_text.clone(), *span_style));
        }
    } else {
        spans.push(Span::styled(diff_line.content.clone(), content_style));
    }

    // Revealed deletions carry the classic del background across the full row.
    if row.sign == DocSign::Delete {
        let eol = diff_line
            .highlighted_spans
            .as_ref()
            .map(|_| content_style.bg(theme.syntax_del_bg))
            .unwrap_or(content_style);
        spans.push(Span::styled(String::new(), eol));
    }

    lines.push(Line::from(spans));
    *line_idx += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(origin: LineOrigin, old: Option<u32>, new: Option<u32>) -> DiffLine {
        DiffLine {
            origin,
            content: String::new(),
            old_lineno: old,
            new_lineno: new,
            highlighted_spans: None,
        }
    }

    fn ctx(o: u32, n: u32) -> DiffLine {
        line(LineOrigin::Context, Some(o), Some(n))
    }
    fn add(n: u32) -> DiffLine {
        line(LineOrigin::Addition, None, Some(n))
    }
    fn del(o: u32) -> DiffLine {
        line(LineOrigin::Deletion, Some(o), None)
    }

    /// Collapsed view emits a row per new-side line; deletions never produce
    /// rows, so the emitted set mirrors `build_unified_diff_annotations` with
    /// `skip_deletions = true`.
    #[test]
    fn collapsed_emits_one_row_per_new_side_line() {
        let lines = vec![ctx(1, 1), del(2), add(2), add(3)];
        let rows = compute_hunk_signs(&lines, false);
        let emitted: Vec<usize> = rows.iter().map(|r| r.line_idx).collect();
        // del at index 1 is hidden; context + two additions remain.
        assert_eq!(emitted, vec![0, 2, 3]);
    }

    /// A deletion immediately followed by an addition is a "changed" line; the
    /// surplus addition beyond the deletion count is a pure add.
    #[test]
    fn pairs_deletion_with_addition_as_change() {
        let lines = vec![del(5), add(5), add(6)];
        let rows = compute_hunk_signs(&lines, false);
        let signs: Vec<DocSign> = rows.iter().map(|r| r.sign).collect();
        assert_eq!(signs, vec![DocSign::Change, DocSign::Add]);
    }

    /// A pure deletion (no addition to absorb it) is invisible in the collapsed
    /// document, so it marks the next emitted row instead of producing one.
    #[test]
    fn pure_deletion_marks_following_row() {
        let lines = vec![ctx(1, 1), del(2), ctx(3, 2)];
        let rows = compute_hunk_signs(&lines, false);
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].delete_above);
        assert!(rows[1].delete_above, "row after the deletion is marked");
        assert_eq!(rows[1].line_idx, 2);
    }

    /// Surplus deletions beyond the paired additions still leave a marker.
    #[test]
    fn surplus_deletions_mark_following_row() {
        let lines = vec![del(1), del(2), add(1), ctx(3, 2)];
        let rows = compute_hunk_signs(&lines, false);
        // One addition pairs with one deletion (Change); the extra deletion
        // marks the trailing context row.
        assert_eq!(rows.iter().map(|r| r.sign).collect::<Vec<_>>(), vec![
            DocSign::Change,
            DocSign::Context
        ]);
        assert!(rows[1].delete_above);
    }

    /// Revealing a hunk emits its deletions as their own rows, in source order.
    #[test]
    fn revealed_emits_deletion_rows() {
        let lines = vec![del(5), add(5), add(6)];
        let rows = compute_hunk_signs(&lines, true);
        let pairs: Vec<(usize, DocSign)> = rows.iter().map(|r| (r.line_idx, r.sign)).collect();
        assert_eq!(pairs, vec![
            (0, DocSign::Delete),
            (1, DocSign::Change),
            (2, DocSign::Add),
        ]);
    }
}
