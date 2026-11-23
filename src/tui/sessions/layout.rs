use portable_pty::PtySize;

use super::{SessionId, pty_size};

const SIDE_BY_SIDE_WIDTH: u16 = 65;
const MIN_PANE_WIDTH: u16 = 20;
const MIN_PANE_HEIGHT: u16 = 4; // Includes one row for the pane title.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sidebar {
    #[default]
    Auto,
    Visible,
    Hidden,
}

#[derive(Clone, Debug)]
pub(super) struct Arrangement {
    pub(super) panes: [Option<SessionId>; 2],
    pub(super) focused: usize,
    pub(super) sidebar: Sidebar,
    pub(super) sidebar_focused: bool,
    split_ratio: u16,
    maximized: bool,
}

impl Default for Arrangement {
    fn default() -> Self {
        Self {
            panes: [None, None],
            focused: 0,
            sidebar: Sidebar::Auto,
            sidebar_focused: false,
            split_ratio: 50,
            maximized: false,
        }
    }
}

impl Arrangement {
    pub(super) fn active(&self) -> Option<SessionId> {
        self.panes[self.focused]
    }

    pub(super) fn activate(&mut self, id: SessionId) {
        self.focused = self
            .panes
            .iter()
            .position(|pane| *pane == Some(id))
            .unwrap_or(self.focused);
        self.panes[self.focused] = Some(id);
    }

    pub(super) fn split(&mut self, id: SessionId) {
        self.focused = 1 - self.focused;
        self.panes[self.focused] = Some(id);
    }

    pub(super) fn split_active(&self) -> bool {
        self.panes.iter().all(Option::is_some)
    }

    pub(super) fn split_ratio(&self) -> u16 {
        self.split_ratio
    }

    pub(super) fn set_split_ratio(&mut self, ratio: u16) {
        self.split_ratio = ratio.min(100);
    }

    pub(super) fn adjust_split(&mut self, delta: i16) {
        self.split_ratio = (i32::from(self.split_ratio) + i32::from(delta)).clamp(0, 100) as u16;
    }

    pub(super) fn focus_other(&mut self) {
        if self.split_active() {
            self.focused = 1 - self.focused;
            self.sidebar_focused = false;
        }
    }

    pub(super) fn toggle_maximized(&mut self) {
        if self.split_active() {
            self.maximized = !self.maximized;
            self.sidebar_focused = false;
        }
    }

    pub(super) fn is_maximized(&self) -> bool {
        self.maximized
    }

    pub(super) fn unsplit(&mut self) {
        self.panes = [self.active(), None];
        self.focused = 0;
        self.maximized = false;
    }

    pub(super) fn move_pane(&mut self) {
        if self.split_active() {
            self.panes.swap(0, 1);
            self.focused = 1 - self.focused;
        }
    }

    pub(super) fn close(&mut self, id: SessionId, fallback: Option<SessionId>) {
        let Some(index) = self.panes.iter().position(|pane| *pane == Some(id)) else {
            return;
        };
        let remaining = self.panes[1 - index].or(fallback);
        self.panes = [remaining, None];
        self.focused = 0;
        self.maximized = false;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) width: u16,
    pub(super) height: u16,
}

impl Rect {
    pub(super) fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x - self.x < self.width && y >= self.y && y - self.y < self.height
    }

    pub(super) fn right(self) -> u16 {
        self.x + self.width
    }

    pub(super) fn bottom(self) -> u16 {
        self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pane {
    pub(super) id: SessionId,
    pub(super) body: Rect,
}

impl Pane {
    fn new(id: SessionId, area: Rect) -> Self {
        Self {
            id,
            body: Rect {
                y: area.y + 1,
                height: area.height - 1,
                ..area
            },
        }
    }

    pub(super) fn contains(self, x: u16, y: u16) -> bool {
        self.body.contains(x, y)
            || (y == self.body.y - 1 && x >= self.body.x && x < self.body.right())
    }

    pub(super) fn size(self) -> PtySize {
        pty_size((self.body.width, self.body.height))
    }
}

#[derive(Debug)]
pub(super) struct Layout {
    pub(super) panes: [Option<Pane>; 2],
    pub(super) sidebar: Option<Rect>,
    pub(super) area: Rect,
    pub(super) vertical_divider: Option<u16>,
    pub(super) horizontal_divider: Option<u16>,
    body: Rect,
}

impl Layout {
    pub(super) fn new((width, height): (u16, u16), arrangement: &Arrangement) -> Self {
        let area = Rect {
            x: 0,
            y: 1,
            width,
            height: height.saturating_sub(2),
        };
        let sidebar = match arrangement.sidebar {
            Sidebar::Auto => area.height >= 3 && (width >= 88 || arrangement.sidebar_focused),
            Sidebar::Visible => area.height >= 3 && (width >= 54 || arrangement.sidebar_focused),
            Sidebar::Hidden => false,
        };
        let (body, sidebar) = if sidebar && width >= 54 {
            let list_width = (width / 4).clamp(22, 34);
            let main_width = width - list_width - 1;
            (
                Rect {
                    width: main_width,
                    ..area
                },
                Some(Rect {
                    x: main_width + 1,
                    width: list_width,
                    ..area
                }),
            )
        } else if sidebar {
            (Rect { width: 0, ..area }, Some(area))
        } else {
            (area, None)
        };
        let mut layout = Self {
            area,
            panes: [None, None],
            sidebar,
            vertical_divider: None,
            horizontal_divider: None,
            body,
        };
        if body.width == 0 || body.height < 2 {
            return layout;
        }
        let [first, second] = arrangement.panes;
        if !arrangement.is_maximized()
            && let (Some(first), Some(second)) = (first, second)
        {
            let ratio = arrangement.split_ratio();
            if body.width >= SIDE_BY_SIDE_WIDTH && body.height >= MIN_PANE_HEIGHT {
                let share = split_share(body.width, MIN_PANE_WIDTH, ratio);
                let divider = body.x + share;
                layout.vertical_divider = Some(divider);
                layout.panes = [
                    Some(Pane::new(
                        first,
                        Rect {
                            width: share,
                            ..body
                        },
                    )),
                    Some(Pane::new(
                        second,
                        Rect {
                            x: divider + 1,
                            width: body.width - share - 1,
                            ..body
                        },
                    )),
                ];
                return layout;
            }
            if body.width >= MIN_PANE_WIDTH && body.height > 2 * MIN_PANE_HEIGHT {
                let share = split_share(body.height, MIN_PANE_HEIGHT, ratio);
                let divider = body.y + share;
                layout.horizontal_divider = Some(divider);
                layout.panes = [
                    Some(Pane::new(
                        first,
                        Rect {
                            height: share,
                            ..body
                        },
                    )),
                    Some(Pane::new(
                        second,
                        Rect {
                            y: divider + 1,
                            height: body.height - share - 1,
                            ..body
                        },
                    )),
                ];
                return layout;
            }
        }
        let focused = arrangement.focused;
        layout.panes[focused] = arrangement.active().map(|id| Pane::new(id, body));
        layout
    }

    pub(super) fn pane(&self, id: SessionId) -> Option<Pane> {
        self.panes.into_iter().flatten().find(|pane| pane.id == id)
    }

    pub(super) fn divider_at(&self, x: u16, y: u16) -> bool {
        self.body.contains(x, y)
            && (self.vertical_divider == Some(x) || self.horizontal_divider == Some(y))
    }

    pub(super) fn ratio_at(&self, x: u16, y: u16) -> Option<u16> {
        let (position, available, minimum) = if self.vertical_divider.is_some() {
            (
                x.saturating_sub(self.body.x),
                self.body.width - 1,
                MIN_PANE_WIDTH,
            )
        } else if self.horizontal_divider.is_some() {
            (
                y.saturating_sub(self.body.y),
                self.body.height - 1,
                MIN_PANE_HEIGHT,
            )
        } else {
            return None;
        };
        let share = position.clamp(minimum, available - minimum);
        // Round up so looking up the ratio at a cell and flooring the resulting
        // share selects that cell when it is representable in whole percentages.
        Some((100 * u32::from(share)).div_ceil(u32::from(available)) as u16)
    }
}

fn split_share(length: u16, minimum: u16, ratio: u16) -> u16 {
    let available = length - 1;
    let share = (u32::from(available) * u32::from(ratio) / 100) as u16;
    share.clamp(minimum, available - minimum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split() -> Arrangement {
        let mut arrangement = Arrangement {
            sidebar: Sidebar::Hidden,
            ..Arrangement::default()
        };
        arrangement.activate(1);
        arrangement.split(2);
        arrangement
    }

    #[test]
    fn split_bias_tracks_keyboard_changes_across_reflow() {
        let mut arrangement = split();
        let wide = Layout::new((101, 28), &arrangement);
        assert_eq!(wide.vertical_divider, Some(50));

        arrangement.adjust_split(15);
        let wide = Layout::new((101, 28), &arrangement);
        assert_eq!(wide.pane(1).unwrap().size().cols, 65);
        assert_eq!(wide.pane(2).unwrap().size().cols, 35);

        let narrow = Layout::new((60, 23), &arrangement);
        assert_eq!(narrow.horizontal_divider, Some(14));
        assert_eq!(narrow.pane(1).unwrap().size().rows, 12);
        assert_eq!(narrow.pane(2).unwrap().size().rows, 6);
        assert_eq!(arrangement.split_ratio(), 65);
        assert_eq!(arrangement.active(), Some(2));
    }

    #[test]
    fn resizing_at_extremes_keeps_both_panes_usable() {
        let mut arrangement = split();
        arrangement.adjust_split(i16::MIN);
        assert_eq!(arrangement.split_ratio(), 0);
        let wide = Layout::new((65, 8), &arrangement);
        assert_eq!(wide.pane(1).unwrap().size().cols, 20);
        assert_eq!(wide.pane(2).unwrap().size().cols, 44);

        arrangement.set_split_ratio(u16::MAX);
        assert_eq!(arrangement.split_ratio(), 100);
        let narrow = Layout::new((60, 11), &arrangement);
        assert_eq!(narrow.pane(1).unwrap().size().rows, 3);
        assert_eq!(narrow.pane(2).unwrap().size().rows, 3);

        let short = Layout::new((101, 5), &arrangement);
        assert!(short.pane(1).is_none());
        assert_eq!(short.pane(2).unwrap().size().rows, 2);
        let tiny = Layout::new((101, 3), &arrangement);
        assert!(tiny.panes.iter().all(Option::is_none));
    }

    #[test]
    fn divider_hit_testing_and_drag_follow_the_visible_axis() {
        let mut arrangement = split();
        let wide = Layout::new((101, 28), &arrangement);
        assert!(wide.divider_at(50, 1));
        assert!(!wide.divider_at(50, 0));
        assert!(!wide.divider_at(51, 1));
        assert_eq!(wide.ratio_at(66, 17), Some(66));
        assert_eq!(wide.ratio_at(0, 17), Some(20));
        assert_eq!(wide.ratio_at(300, 17), Some(80));

        arrangement.sidebar = Sidebar::Visible;
        let stacked = Layout::new((60, 23), &arrangement);
        assert!(stacked.divider_at(36, 11));
        assert!(!stacked.divider_at(37, 11));
        assert!(!stacked.divider_at(38, 11));
        assert_eq!(stacked.ratio_at(10, 13), Some(60));
        assert_eq!(stacked.ratio_at(10, 0), Some(20));
        assert_eq!(stacked.ratio_at(10, 40), Some(80));
    }

    #[test]
    fn maximizing_and_switching_focus_preserve_both_sessions_for_restore() {
        let mut arrangement = split();
        arrangement.set_split_ratio(62);
        let original = Layout::new((101, 28), &arrangement);

        arrangement.toggle_maximized();
        assert!(arrangement.is_maximized());
        let maximized = Layout::new((101, 28), &arrangement);
        assert!(maximized.pane(1).is_none());
        assert_eq!(maximized.pane(2).unwrap().size().cols, 101);
        assert!(!maximized.divider_at(62, 1));
        assert_eq!(maximized.ratio_at(62, 1), None);

        arrangement.sidebar_focused = true;
        arrangement.focus_other();
        assert_eq!(arrangement.active(), Some(1));
        assert!(!arrangement.sidebar_focused);
        let switched = Layout::new((101, 28), &arrangement);
        assert_eq!(switched.pane(1).unwrap().size().cols, 101);
        assert!(switched.pane(2).is_none());

        arrangement.toggle_maximized();
        assert!(!arrangement.is_maximized());
        assert_eq!(Layout::new((101, 28), &arrangement).panes, original.panes);
        assert_eq!(arrangement.active(), Some(1));

        arrangement.toggle_maximized();
        arrangement.close(1, None);
        arrangement.toggle_maximized();
        assert!(!arrangement.is_maximized());
        assert_eq!(arrangement.active(), Some(2));

        arrangement.split(3);
        arrangement.toggle_maximized();
        arrangement.unsplit();
        assert!(!arrangement.is_maximized());
        assert_eq!(arrangement.panes, [Some(3), None]);
    }
}
