use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgGroup, Command, value_parser};

fn flag(id: &'static str) -> Arg {
    Arg::new(id).long(id).action(ArgAction::SetTrue)
}
fn option(id: &'static str) -> Arg {
    Arg::new(id).long(id)
}
fn root() -> Arg {
    option("root").value_parser(value_parser!(PathBuf))
}
fn names() -> Arg {
    Arg::new("targets").num_args(1..)
}
fn target() -> Arg {
    Arg::new("target").required(true)
}
fn group() -> Arg {
    Arg::new("group").required(true)
}

fn scope(name: &'static str, required: bool) -> Command {
    Command::new(name)
        .args([
            names(),
            option("select").num_args(1..),
            flag("all"),
            flag("dry-run"),
        ])
        .group(
            ArgGroup::new("repo-scope")
                .args(["targets", "select", "all"])
                .required(required),
        )
}

pub(crate) fn command() -> Command {
    let read = |name| Command::new(name).arg(target());
    let sync = |name, required| {
        scope(name, required).arg(
            option("jobs")
                .value_parser(value_parser!(u8).range(1..=8))
                .default_value("3"),
        )
    };
    Command::new("repos").subcommands([
        Command::new("list"),
        Command::new("discover").args([
            root(),
            flag("refresh").conflicts_with("offline"),
            flag("offline"),
            option("query"),
        ]),
        Command::new("status").arg(names()),
        scope("clone", true).args([
            root(),
            flag("yes"),
            flag("offline"),
            flag("refresh").conflicts_with("offline"),
        ]),
        scope("add", true).args([root(), flag("yes")]),
        scope("remove", true).arg(flag("yes")),
        Command::new("register")
            .arg(target())
            .args([flag("offline"), flag("refresh").conflicts_with("offline")]),
        sync("fetch", true),
        sync("update", false),
        read("switch"),
        read("terminal").arg(option("shell").value_parser(crate::cli::native::SHELLS)),
        read("diff").arg(flag("staged")),
        read("log"),
        read("branches"),
        read("worktrees"),
        read("remotes"),
        read("branch")
            .arg(Arg::new("branch").required(true))
            .arg(flag("switch")),
        read("worktree")
            .arg(
                Arg::new("path")
                    .value_parser(value_parser!(PathBuf))
                    .required(true),
            )
            .arg(option("branch").required(true)),
        read("stage").arg(
            Arg::new("paths")
                .value_parser(value_parser!(PathBuf))
                .num_args(1..)
                .required(true),
        ),
        read("commit").arg(
            Arg::new("message")
                .long("message")
                .short('m')
                .required(true),
        ),
        read("push"),
        read("config").args([
            option("alias").conflicts_with("clear-alias"),
            flag("clear-alias"),
            option("name"),
            option("remote"),
            option("preferred-branch"),
            option("path").value_parser(value_parser!(PathBuf)),
            option("shell").value_parser(crate::cli::native::SHELLS),
        ]),
        Command::new("group")
            .subcommand_required(true)
            .subcommands([
                Command::new("list"),
                Command::new("create").arg(group()),
                Command::new("add").arg(group()).arg(names().required(true)),
                Command::new("remove").arg(group()).arg(names()),
            ]),
    ])
}

pub(crate) fn description(parent: &str, name: &str) -> Option<crate::localization::Localized> {
    Some(match (parent, name) {
        ("repos", "discover") => [
            "Discover local checkouts and accessible GitHub repositories",
            "Tìm checkout cục bộ và repo GitHub có quyền truy cập",
        ],
        ("repos", "list") => [
            "List the workspace repository registry",
            "Liệt kê danh bạ repo của workspace",
        ],
        ("repos", "clone") => [
            "Clone selected repositories into verified destinations",
            "Clone repo đã chọn vào vị trí được xác minh",
        ],
        ("repos", "add") => [
            "Register existing local Git checkouts",
            "Đăng ký checkout Git hiện có ở máy cục bộ",
        ],
        ("repos", "register") => [
            "Register a GitHub repository without cloning",
            "Đăng ký repo GitHub mà chưa clone",
        ],
        ("repos", "remove") => [
            "Remove entries from the registry",
            "Gỡ mục khỏi danh bạ repo",
        ],
        ("repos", "update") => [
            "Fast-forward selected tracked branches",
            "Cập nhật fast-forward các nhánh theo dõi đã chọn",
        ],
        ("repos", "fetch") => [
            "Fetch selected configured remotes",
            "Fetch từ remote đã cấu hình cho các repo đã chọn",
        ],
        ("repos", "switch") => [
            "Select the active repository working directory",
            "Chọn thư mục làm việc của repo hiện hành",
        ],
        ("repos", "terminal") => [
            "Open an interactive shell in a verified repository",
            "Mở shell tương tác trong repo đã xác minh",
        ],
        ("repos", "config") => [
            "Inspect or edit a repository's manager settings",
            "Xem hoặc sửa cấu hình quản lý repo",
        ],
        ("repos", "diff") => [
            "Show the repository's working diff",
            "Hiển thị diff của repo",
        ],
        ("repos", "log") => [
            "Show recent repository history",
            "Hiển thị lịch sử gần đây của repo",
        ],
        ("repos", "branches") => ["List local branches", "Liệt kê nhánh cục bộ"],
        ("repos", "worktrees") => ["List Git worktrees", "Liệt kê Git worktree"],
        ("repos", "remotes") => [
            "Inspect remotes and upstream tracking",
            "Kiểm tra remote và nhánh upstream",
        ],
        ("repos", "branch") => [
            "Create or switch to a branch with Git safety checks",
            "Tạo hoặc chuyển nhánh qua kiểm tra an toàn Git",
        ],
        ("repos", "worktree") => [
            "Add a Git worktree for an existing branch",
            "Thêm Git worktree cho nhánh hiện có",
        ],
        ("repos", "stage") => [
            "Stage explicitly selected paths",
            "Stage các đường dẫn được chỉ định rõ",
        ],
        ("repos", "commit") => [
            "Commit staged changes with the supplied message",
            "Commit thay đổi đã stage bằng thông điệp đã nhập",
        ],
        ("repos", "push") => [
            "Push the checked-out branch to its upstream",
            "Push nhánh đang checkout lên upstream",
        ],
        ("repos", "group") => [
            "Organize repositories into groups",
            "Sắp xếp repo theo nhóm",
        ],
        ("group", "list") => ["List repository groups", "Liệt kê nhóm repo"],
        ("group", "create") => ["Create a repository group", "Tạo nhóm repo"],
        ("group", "add") => [
            "Add selected entries to a group",
            "Thêm các mục đã chọn vào một nhóm",
        ],
        ("group", "remove") => [
            "Remove selected memberships or an entire group",
            "Gỡ mục đã chọn khỏi nhóm hoặc gỡ cả nhóm",
        ],
        _ => return None,
    })
}
