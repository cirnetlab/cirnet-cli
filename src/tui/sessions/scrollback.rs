use vt100::Screen;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Match {
    start: (usize, u16),
    end: (usize, u16),
}

impl Match {
    pub(super) fn visible_text(&self, screen: &Screen) -> Option<String> {
        let top = screen.scrollback_len() - screen.scrollback();
        let start = self.start.0.checked_sub(top)?;
        let end = self.end.0.checked_sub(top)?;
        (end < usize::from(screen.size().0.get())).then(|| {
            screen.contents_between(start as u16, self.start.1, end as u16, self.end.1 + 1)
        })
    }

    pub(super) fn contains(&self, screen: &Screen, row: u16, col: u16) -> bool {
        let absolute_row = screen.scrollback_len() - screen.scrollback() + usize::from(row);
        if !(self.start..=self.end).contains(&(absolute_row, col)) {
            return false;
        }
        // A wrapped historical row can have trailing empty cells after a resize.
        // They are absent from the logical text, even when the match spans this row.
        let columns = screen.size().1.get();
        absolute_row == self.end.0
            || screen
                .cell(row, col)
                .is_some_and(|cell| cell.is_wide_continuation())
            || (col..columns)
                .any(|column| visible_text_cell(screen, row, column, columns).is_some())
    }
}

pub(super) fn find(
    screen: &mut Screen,
    query: &str,
    reverse: bool,
    previous: Option<Match>,
) -> Option<Match> {
    if query.is_empty() {
        return None;
    }
    let query: String = query.chars().flat_map(char::to_lowercase).collect();
    let (rows, columns) = screen.size();
    let height = usize::from(rows.get());
    let history = screen.scrollback_len();
    let offset = screen.scrollback();
    let top = history - offset;
    let anchor = previous.map_or_else(
        || {
            if reverse { (top + height, 0) } else { (top, 0) }
        },
        |hit| hit.start,
    );

    // Paging can materialize historical rows at the current width.
    // Search a snapshot so a miss leaves the live grid untouched.
    let mut snapshot = screen.clone();
    let mut first_in_direction = None;
    let mut next_in_direction = None;
    let mut line = LogicalLine::default();
    let total = history + height;
    let mut row = 0;
    while row < total {
        // Walk every retained row once, using a viewport at a time. The last
        // viewport begins at the live screen and may overlap the previous one.
        let page = row.min(history);
        snapshot.set_scrollback(history - page);
        while row < page + height {
            let visible_row = (row - page) as u16;
            line.append_row(&snapshot, visible_row, row, columns.get());
            row += 1;
            if row < total && snapshot.row_wrapped(visible_row) {
                continue;
            }
            for hit in line.matches(&query) {
                if reverse || first_in_direction.is_none() {
                    first_in_direction = Some(hit);
                }
                if reverse {
                    if hit.start < anchor {
                        next_in_direction = Some(hit);
                    }
                } else if next_in_direction.is_none()
                    && (hit.start > anchor || (previous.is_none() && hit.start == anchor))
                {
                    next_in_direction = Some(hit);
                }
            }
            line.text.clear();
            line.cells.clear();
        }
    }

    let hit = next_in_direction.or(first_in_direction);
    let new_offset = hit.map_or(offset, |hit| {
        let first = hit.start.0;
        let last = hit.end.0;
        let minimum_top = last.saturating_sub(height - 1).min(first);
        let top = top.clamp(minimum_top, first).min(history);
        history - top
    });
    if new_offset != offset {
        screen.set_scrollback(new_offset);
    }
    hit
}

#[derive(Default)]
struct LogicalLine {
    text: String,
    cells: Vec<LineCell>,
}

struct LineCell {
    byte: usize,
    row: usize,
    first_col: u16,
    last_col: u16,
}

impl LogicalLine {
    fn append_row(&mut self, screen: &Screen, row: u16, absolute_row: usize, columns: u16) {
        let mut next_col = 0;
        for col in 0..columns {
            let Some(cell) = visible_text_cell(screen, row, col, columns) else {
                continue;
            };
            for gap in next_col..col {
                self.cells.push(LineCell {
                    byte: self.text.len(),
                    row: absolute_row,
                    first_col: gap,
                    last_col: gap,
                });
                self.text.push(' ');
            }
            let last_col = col + u16::from(cell.is_wide());
            self.cells.push(LineCell {
                byte: self.text.len(),
                row: absolute_row,
                first_col: col,
                last_col,
            });
            self.text
                .extend(cell.contents().chars().flat_map(char::to_lowercase));
            next_col = last_col + 1;
        }
    }

    fn matches<'a>(&'a self, query: &'a str) -> impl Iterator<Item = Match> + 'a {
        self.text.match_indices(query).map(|(start, matched)| {
            let end = start + matched.len();
            let first = self.cells.partition_point(|cell| cell.byte <= start) - 1;
            let last = self.cells.partition_point(|cell| cell.byte < end) - 1;
            let first = &self.cells[first];
            let last = &self.cells[last];
            Match {
                start: (first.row, first.first_col),
                end: (last.row, last.last_col),
            }
        })
    }
}

fn visible_text_cell(screen: &Screen, row: u16, col: u16, columns: u16) -> Option<&vt100::Cell> {
    screen.cell(row, col).filter(|cell| {
        cell.has_contents()
            && !cell.is_wide_continuation()
            && (!cell.is_wide() || col + 1 < columns)
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Selection {
    anchor: SelectedCell,
    focus: SelectedCell,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SelectedCell {
    row: u16,
    first_col: u16,
    last_col: u16,
}

impl SelectedCell {
    fn at(screen: &Screen, row: u16, col: u16) -> Self {
        let (rows, columns) = screen.size();
        let row = row.min(rows.get() - 1);
        let col = col.min(columns.get() - 1);
        let cell = screen.cell(row, col);
        let first_col = if cell.is_some_and(vt100::Cell::is_wide_continuation) {
            col.saturating_sub(1)
        } else {
            col
        };
        let last_col = if cell.is_some_and(vt100::Cell::is_wide) {
            (col + 1).min(columns.get() - 1)
        } else {
            col
        };
        Self {
            row,
            first_col,
            last_col,
        }
    }
}

impl Selection {
    pub(super) fn new(screen: &Screen, row: u16, col: u16) -> Self {
        let cell = SelectedCell::at(screen, row, col);
        Self {
            anchor: cell,
            focus: cell,
        }
    }

    pub(super) fn extend(&mut self, screen: &Screen, row: u16, col: u16) {
        self.focus = SelectedCell::at(screen, row, col);
    }

    pub(super) fn contains(&self, row: u16, col: u16) -> bool {
        let (start, end) = self.ordered();
        ((start.row, start.first_col)..=(end.row, end.last_col)).contains(&(row, col))
    }

    pub(super) fn text(&self, screen: &Screen) -> String {
        let (start, end) = self.ordered();
        let (rows, columns) = screen.size();
        if end.row >= rows.get()
            || start.first_col >= columns.get()
            || end.last_col >= columns.get()
        {
            return String::new();
        }
        screen.contents_between(start.row, start.first_col, end.row, end.last_col + 1)
    }

    fn ordered(&self) -> (SelectedCell, SelectedCell) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;
    use vt100::Parser;

    fn terminal(rows: u16, columns: u16, text: &str) -> Parser {
        let mut parser = Parser::new(
            NonZeroU16::new(rows).unwrap(),
            NonZeroU16::new(columns).unwrap(),
            8,
        );
        parser.process(text.as_bytes());
        parser
    }

    #[test]
    fn find_cycles_between_visible_and_history_matches() {
        let mut parser = terminal(2, 12, "old École\r\nother\r\nnew école\r\nlast");
        let screen = parser.screen_mut();
        let live = find(screen, "ÉCOLE", false, None).expect("visible match");
        assert_eq!(screen.scrollback(), 0);
        assert!(live.contains(screen, 0, 4));
        assert!(live.contains(screen, 0, 8));
        assert!(!live.contains(screen, 0, 9));

        let history = find(screen, "ÉCOLE", false, Some(live)).expect("wrapped history match");
        assert_eq!(screen.scrollback(), 2);
        assert!(history.contains(screen, 0, 4));
        let wrapped = find(screen, "ÉCOLE", true, Some(history)).expect("wrapped live match");
        assert_eq!(wrapped, live);
        assert_eq!(screen.scrollback(), 1);
        assert!(wrapped.contains(screen, 1, 4));
    }

    #[test]
    fn find_maps_a_wrapped_match_to_wide_and_combining_cells() {
        let mut parser = terminal(3, 5, "abc界De\u{301}f\r\nbreak");
        let screen = parser.screen_mut();
        let hit = find(screen, "界DE\u{301}", false, None).expect("soft-wrapped match");
        assert!(!hit.contains(screen, 0, 2));
        assert!(hit.contains(screen, 0, 3));
        assert!(hit.contains(screen, 0, 4));
        assert!(hit.contains(screen, 1, 0));
        assert!(hit.contains(screen, 1, 1));
        assert!(!hit.contains(screen, 1, 2));
        assert!(find(screen, "fbreak", false, None).is_none());
    }

    #[test]
    fn find_reveals_a_soft_wrap_between_history_and_live_rows() {
        let mut parser = terminal(2, 5, "start\r\nabc界Def\r\nlast");
        let screen = parser.screen_mut();
        let hit = find(screen, "界de", false, None).expect("match across history and live");
        assert_eq!(screen.scrollback(), 1);
        assert!(hit.contains(screen, 0, 3));
        assert!(hit.contains(screen, 0, 4));
        assert!(hit.contains(screen, 1, 0));
        assert!(hit.contains(screen, 1, 1));
        assert!(!hit.contains(screen, 1, 2));
    }

    #[test]
    fn find_keeps_the_original_view_and_parser_state_on_miss() {
        let mut parser = terminal(2, 12, "history\r\nsecond\r\nprompt\x1b[?2004h");
        let screen = parser.screen_mut();
        screen.set_scrollback(1);
        let contents = screen.contents();
        let cursor = screen.cursor_position();
        assert!(find(screen, "missing", true, None).is_none());
        assert_eq!(screen.scrollback(), 1);
        assert_eq!(screen.contents(), contents);
        assert_eq!(screen.cursor_position(), cursor);
        assert!(screen.bracketed_paste());
        assert_eq!(screen.scrollback_len(), 1);
    }

    #[test]
    fn selection_copies_a_wide_glyph_from_either_cell() {
        let parser = terminal(2, 8, "a界b");
        let screen = parser.screen();
        let selection = Selection::new(screen, 0, 2);
        assert!(selection.contains(0, 1));
        assert!(selection.contains(0, 2));
        assert!(!selection.contains(0, 3));
        assert_eq!(selection.text(screen), "界");
    }

    #[test]
    fn selection_copies_backward_across_soft_and_hard_line_breaks() {
        let parser = terminal(3, 5, "abc界de\r\nlast");
        let screen = parser.screen();
        let mut selection = Selection::new(screen, 2, 1);
        selection.extend(screen, 0, 2);
        assert!(!selection.contains(0, 1));
        assert!(selection.contains(1, 0));
        assert!(selection.contains(2, 1));
        assert!(!selection.contains(2, 2));
        assert_eq!(selection.text(screen), "c界de\nla");
    }

    #[test]
    fn selection_copies_from_the_visible_history_view() {
        let mut parser = terminal(2, 8, "old line\r\nnew line\r\nbottom");
        let screen = parser.screen_mut();
        screen.set_scrollback(1);
        let mut selection = Selection::new(screen, 0, 4);
        selection.extend(screen, 1, 2);
        assert_eq!(selection.text(screen), "line\nnew");
        assert_eq!(screen.scrollback(), 1);
    }
}
