use super::{Dashboard, Effect, Input, Overlay, Row, tr};
use crate::tui::completion;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Discover,
    RefreshRemote,
    Register,
    Clone,
    Add,
    AddPaths,
    Remove,
    Fetch,
    Update,
    Terminal,
    Switch,
    Status,
    Diff,
    History,
    Branches,
    BranchCreate,
    BranchSwitch,
    Worktrees,
    WorktreeAdd,
    Remotes,
    GroupCreate,
    GroupAdd,
    GroupRemove,
}

impl Action {
    const ALL: [Self; 23] = [
        Self::Discover,
        Self::RefreshRemote,
        Self::Register,
        Self::Clone,
        Self::Add,
        Self::AddPaths,
        Self::Remove,
        Self::Fetch,
        Self::Update,
        Self::Terminal,
        Self::Switch,
        Self::Status,
        Self::Diff,
        Self::History,
        Self::Branches,
        Self::BranchCreate,
        Self::BranchSwitch,
        Self::Worktrees,
        Self::WorktreeAdd,
        Self::Remotes,
        Self::GroupCreate,
        Self::GroupAdd,
        Self::GroupRemove,
    ];

    pub(super) fn label(self, vi: bool) -> &'static str {
        let (en, vn) = match self {
            Self::Discover => ("Discover repositories", "Tìm kho mã"),
            Self::RefreshRemote => ("Refresh GitHub discovery", "Làm mới danh sách GitHub"),
            Self::Register => ("Register without cloning", "Đăng ký chưa sao chép"),
            Self::Clone => ("Clone", "Sao chép"),
            Self::Add => (
                "Add selected local checkouts",
                "Thêm bản sao cục bộ đã chọn",
            ),
            Self::AddPaths => ("Add local paths", "Thêm đường dẫn cục bộ"),
            Self::Remove => ("Remove from registry", "Xóa khỏi sổ đăng ký"),
            Self::Fetch => ("Fetch", "Lấy dữ liệu"),
            Self::Update => ("Update (fast-forward only)", "Cập nhật (chỉ fast-forward)"),
            Self::Terminal => ("Open repository terminal", "Mở terminal của kho mã"),
            Self::Switch => ("Switch working context", "Đổi ngữ cảnh làm việc"),
            Self::Status => ("Inspect status", "Kiểm tra trạng thái"),
            Self::Diff => ("Diff", "Xem thay đổi"),
            Self::History => ("History", "Lịch sử"),
            Self::Branches => ("List branches", "Liệt kê nhánh"),
            Self::BranchCreate => ("Create branch", "Tạo nhánh"),
            Self::BranchSwitch => ("Switch branch", "Chuyển nhánh"),
            Self::Worktrees => ("List worktrees", "Liệt kê worktree"),
            Self::WorktreeAdd => ("Add worktree", "Thêm worktree"),
            Self::Remotes => ("Inspect remotes", "Kiểm tra remote"),
            Self::GroupCreate => ("Create group", "Tạo nhóm"),
            Self::GroupAdd => (
                "Add selected repositories to group",
                "Thêm kho mã đã chọn vào nhóm",
            ),
            Self::GroupRemove => (
                "Remove selected repositories from group",
                "Gỡ kho mã đã chọn khỏi nhóm",
            ),
        };
        tr(vi, en, vn)
    }

    fn command(self) -> &'static [&'static str] {
        match self {
            Self::Discover => &["discover"],
            Self::RefreshRemote => &["discover", "--refresh"],
            Self::Register => &["register"],
            Self::Clone => &["clone"],
            Self::Add | Self::AddPaths => &["add"],
            Self::Remove => &["remove"],
            Self::Fetch => &["fetch"],
            Self::Update => &["update"],
            Self::Terminal => &[],
            Self::Switch => &["switch"],
            Self::Status => &["status"],
            Self::Diff => &["diff"],
            Self::History => &["log"],
            Self::Branches => &["branches"],
            Self::BranchCreate => &["branch"],
            Self::BranchSwitch => &["branch", "--switch"],
            Self::Worktrees => &["worktrees"],
            Self::WorktreeAdd => &["worktree"],
            Self::Remotes => &["remotes"],
            Self::GroupCreate => &["group", "create"],
            Self::GroupAdd => &["group", "add"],
            Self::GroupRemove => &["group", "remove"],
        }
    }

    pub(super) fn prompt(self, vi: bool) -> Option<&'static str> {
        Some(match self {
            Self::AddPaths => tr(
                vi,
                "Local paths (quote spaces)",
                "Đường dẫn (đặt dấu nháy quanh khoảng trắng)",
            ),
            Self::BranchCreate | Self::BranchSwitch => tr(vi, "Branch name", "Tên nhánh"),
            Self::WorktreeAdd => tr(
                vi,
                "Path and branch (quote spaces)",
                "Đường dẫn và nhánh (dùng dấu nháy nếu có khoảng trắng)",
            ),
            Self::GroupCreate | Self::GroupAdd | Self::GroupRemove => {
                tr(vi, "Group name", "Tên nhóm")
            }
            _ => return None,
        })
    }

    fn per_focus(self) -> bool {
        matches!(
            self,
            Self::Register
                | Self::Terminal
                | Self::Switch
                | Self::Diff
                | Self::History
                | Self::Branches
                | Self::BranchCreate
                | Self::BranchSwitch
                | Self::Worktrees
                | Self::WorktreeAdd
                | Self::Remotes
        )
    }

    fn without_targets(self) -> bool {
        matches!(
            self,
            Self::Discover | Self::RefreshRemote | Self::AddPaths | Self::GroupCreate
        )
    }

    fn applies_to(self, row: &Row) -> bool {
        let checkout = row.path.is_some() && !matches!(row.state.as_str(), "missing" | "invalid");
        match self {
            Self::Register => !row.registered && row.state == "available",
            Self::Clone => matches!(row.state.as_str(), "available" | "uncloned" | "missing"),
            Self::Add => checkout && !row.registered,
            Self::Remove | Self::GroupAdd | Self::GroupRemove => row.registered,
            Self::Switch => checkout && row.registered,
            Self::Status => true,
            _ => checkout,
        }
    }

    pub(super) fn changes_view(self) -> bool {
        !matches!(
            self,
            Self::Terminal
                | Self::Diff
                | Self::History
                | Self::Branches
                | Self::Worktrees
                | Self::Remotes
        )
    }

    fn needs_preview(self) -> bool {
        self.changes_view()
            && !matches!(
                self,
                Self::Discover | Self::RefreshRemote | Self::Switch | Self::Status
            )
    }
}

pub(super) struct Target {
    pub(super) selector: String,
    pub(super) label: String,
}

impl From<&Row> for Target {
    fn from(row: &Row) -> Self {
        let identity = row
            .path
            .as_deref()
            .map(std::path::Path::to_string_lossy)
            .or_else(|| row.remote.as_deref().map(Into::into));
        let label = identity
            .as_deref()
            .filter(|identity| *identity != row.name)
            .map_or_else(
                || row.name.clone(),
                |identity| format!("{} · {identity}", row.name),
            );
        Self {
            selector: row.selector.clone(),
            label,
        }
    }
}

pub(super) struct Plan {
    pub(super) action: Action,
    pub(super) args: Vec<String>,
    pub(super) labels: Vec<String>,
}

impl Dashboard {
    pub(super) fn open_actions(&mut self) -> Effect {
        self.searching = false;
        let grammar = completion::grammar().find_subcommand("repos");
        let targets = self.targets();
        let focused = self.focused();
        let items = Action::ALL
            .into_iter()
            .filter(|action| {
                action.command().first().is_none_or(|name| {
                    grammar.is_some_and(|grammar| grammar.find_subcommand(name).is_some())
                }) && (!self.update_only
                    || matches!(action, Action::Update | Action::Fetch | Action::Status))
                    && (action.without_targets()
                        || if action.per_focus() {
                            focused.is_some_and(|row| action.applies_to(row))
                        } else {
                            targets.iter().any(|row| action.applies_to(row))
                        })
            })
            .collect();
        self.overlay = Overlay::Actions { items, cursor: 0 };
        Effect::Redraw
    }

    pub(super) fn activate(&mut self, action: Action) -> Effect {
        self.overlay = Overlay::None;
        self.notice.clear();
        let targets = if action.without_targets() {
            Vec::new()
        } else if action.per_focus() {
            self.focused().map(Target::from).into_iter().collect()
        } else {
            self.targets().into_iter().map(Target::from).collect()
        };
        if !action.without_targets() && targets.is_empty() {
            self.notice = tr(
                self.vi,
                "Select a repository first.",
                "Hãy chọn một kho mã trước.",
            )
            .into();
            return Effect::Redraw;
        }
        if action == Action::Terminal {
            return Effect::Terminal(
                targets
                    .into_iter()
                    .next()
                    .expect("selected target")
                    .selector,
            );
        }
        if action.prompt(self.vi).is_some() {
            self.overlay = Overlay::Prompt {
                action,
                targets,
                input: Input::default(),
            };
            return Effect::Redraw;
        }
        self.prepare(action, targets, Vec::new())
    }

    fn prepare(&mut self, action: Action, targets: Vec<Target>, inputs: Vec<String>) -> Effect {
        let mut args: Vec<String> = std::iter::once("repos")
            .chain(action.command().iter().copied())
            .map(str::to_owned)
            .collect();
        if action == Action::Update
            && let Some(jobs) = self.update_jobs
        {
            args.extend(["--jobs".to_owned(), jobs.to_string()]);
        }
        let mut labels: Vec<_> = targets.iter().map(|target| target.label.clone()).collect();
        let identifiers = targets.into_iter().map(|target| target.selector);
        match action {
            Action::GroupCreate | Action::GroupAdd | Action::GroupRemove => {
                labels.extend(inputs.iter().cloned());
                args.push("--".into());
                args.extend(inputs);
                args.extend(identifiers);
            }
            Action::WorktreeAdd => {
                let mut inputs = inputs.into_iter();
                let path = inputs.next().expect("validated worktree path");
                let branch = inputs.next().expect("validated branch name");
                args.push(format!("--branch={branch}"));
                args.push("--".into());
                args.extend(identifiers);
                args.push(path.clone());
                labels.extend([path, branch]);
            }
            Action::AddPaths | Action::BranchCreate | Action::BranchSwitch => {
                labels.extend(inputs.iter().cloned());
                args.push("--".into());
                args.extend(identifiers);
                args.extend(inputs);
            }
            _ => {
                args.extend(identifiers);
            }
        }
        if action.needs_preview() {
            self.overlay = Overlay::Preview {
                plan: Plan {
                    action,
                    args,
                    labels,
                },
                scroll: 0,
            };
            Effect::Redraw
        } else {
            Effect::Run { args, action }
        }
    }

    pub(super) fn submit_prompt(&mut self) -> Effect {
        let Overlay::Prompt { action, input, .. } = &self.overlay else {
            return Effect::Idle;
        };
        let action = *action;
        let input = input.value.trim();
        let inputs = match action {
            Action::AddPaths | Action::WorktreeAdd => completion::arguments(input),
            _ => Ok(vec![input.to_owned()]),
        };
        let Ok(inputs) = inputs else {
            self.notice = tr(
                self.vi,
                "Close the quoted argument before continuing.",
                "Hãy đóng dấu nháy trước khi tiếp tục.",
            )
            .into();
            return Effect::Redraw;
        };
        if inputs.is_empty()
            || inputs.iter().any(String::is_empty)
            || (action == Action::WorktreeAdd && inputs.len() != 2)
        {
            self.notice = tr(
                self.vi,
                "Enter the required values.",
                "Hãy nhập các giá trị bắt buộc.",
            )
            .into();
            return Effect::Redraw;
        }
        let Overlay::Prompt { targets, .. } = std::mem::replace(&mut self.overlay, Overlay::None)
        else {
            unreachable!()
        };
        self.notice.clear();
        self.prepare(action, targets, inputs)
    }
}
