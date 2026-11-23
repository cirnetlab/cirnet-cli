use portable_pty::PtySize;

use super::{SessionId, pty_size};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sidebar {
    #[default]
    Auto,
    Visible,
    Hidden,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Arrangement {
    pub(super) panes: [Option<SessionId>; 2],
    pub(super) focused: usize,
    pub(super) sidebar: Sidebar,
    pub(super) sidebar_focused: bool,
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

    pub(super) fn unsplit(&mut self) {
        self.panes = [self.active(), None];
        self.focused = 0;
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
            Sidebar::Auto => width >= 88 && area.height >= 4,
            Sidebar::Visible => width >= 54 || arrangement.sidebar_focused,
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
        };
        if body.width == 0 || body.height < 1 {
            return layout;
        }
        let [first, second] = arrangement.panes;
        if let (Some(first), Some(second)) = (first, second) {
            if body.width >= 65 {
                let share = (body.width - 1) / 2;
                layout.vertical_divider = Some(share);
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
                            x: share + 1,
                            width: body.width - share - 1,
                            ..body
                        },
                    )),
                ];
                return layout;
            }
            if body.height >= 9 {
                let share = (body.height - 1) / 2;
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
}
