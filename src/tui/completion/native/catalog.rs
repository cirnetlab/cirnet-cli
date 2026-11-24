pub(super) type Item = (&'static str, [&'static str; 2]);

pub(super) const GIT: Item = ("git", ["Version control", "Quản lý phiên bản"]);

pub(super) const GIT_ACTIONS: &[Item] = &[
    (
        "status",
        ["Show working tree status", "Xem trạng thái cây làm việc"],
    ),
    ("add", ["Stage file changes", "Đưa thay đổi vào vùng chờ"]),
    (
        "commit",
        ["Record staged changes", "Ghi lại thay đổi đã chuẩn bị"],
    ),
    ("log", ["Show commit history", "Xem lịch sử commit"]),
    ("diff", ["Show changes", "Xem thay đổi"]),
    ("switch", ["Switch branches", "Chuyển nhánh"]),
    (
        "branch",
        ["List or manage branches", "Liệt kê hoặc quản lý nhánh"],
    ),
    (
        "checkout",
        [
            "Switch branches or restore paths",
            "Chuyển nhánh hoặc khôi phục đường dẫn",
        ],
    ),
    ("restore", ["Restore files", "Khôi phục tệp"]),
    ("stash", ["Save work temporarily", "Tạm cất thay đổi"]),
    ("show", ["Show a Git object", "Xem đối tượng Git"]),
    ("merge", ["Merge histories", "Hợp nhất lịch sử"]),
    ("rebase", ["Replay commits", "Áp dụng lại commit"]),
    (
        "reset",
        ["Reset selected state", "Đặt lại trạng thái đã chọn"],
    ),
    ("tag", ["List or manage tags", "Liệt kê hoặc quản lý thẻ"]),
    (
        "remote",
        ["Manage remote repositories", "Quản lý kho từ xa"],
    ),
    (
        "fetch",
        ["Download refs and objects", "Tải ref và đối tượng"],
    ),
    (
        "pull",
        ["Fetch and integrate changes", "Tải và tích hợp thay đổi"],
    ),
    ("push", ["Update remote refs", "Cập nhật ref từ xa"]),
    ("clone", ["Clone a repository", "Sao chép kho mã"]),
    ("init", ["Initialize a repository", "Khởi tạo kho mã"]),
    ("worktree", ["Manage working trees", "Quản lý cây làm việc"]),
    (
        "grep",
        ["Search tracked content", "Tìm trong nội dung được theo dõi"],
    ),
    (
        "cherry-pick",
        ["Apply selected commits", "Áp dụng commit đã chọn"],
    ),
    (
        "revert",
        ["Reverse selected commits", "Đảo ngược commit đã chọn"],
    ),
    (
        "reflog",
        ["Inspect the reference log", "Kiểm tra nhật ký ref"],
    ),
    (
        "bisect",
        [
            "Find a commit by binary search",
            "Tìm commit bằng tìm kiếm nhị phân",
        ],
    ),
    (
        "blame",
        ["Show the commit for each line", "Hiện commit của từng dòng"],
    ),
    ("submodule", ["Manage submodules", "Quản lý mô-đun con"]),
    (
        "describe",
        ["Name a commit using refs", "Đặt tên commit bằng ref"],
    ),
    (
        "config",
        ["Read or set configuration", "Đọc hoặc đặt cấu hình"],
    ),
    (
        "archive",
        ["Create an archive from a tree", "Tạo bản lưu trữ từ cây"],
    ),
    ("bundle", ["Manage bundle files", "Quản lý tệp bundle"]),
    (
        "clean",
        ["Remove untracked files", "Xóa tệp chưa được theo dõi"],
    ),
    (
        "maintenance",
        ["Manage repository maintenance", "Quản lý bảo trì kho mã"],
    ),
    (
        "ls-files",
        ["List files in the index", "Liệt kê tệp trong vùng chờ"],
    ),
    (
        "for-each-ref",
        ["List and format refs", "Liệt kê và định dạng ref"],
    ),
    ("rev-parse", ["Resolve revisions", "Phân giải phiên bản"]),
];

pub(super) const GIT_FLAGS: &[Item] = &[
    (
        "--version",
        ["Display the Git version", "Hiện phiên bản Git"],
    ),
    ("--help", ["Display Git help", "Hiện trợ giúp Git"]),
    ("--no-pager", ["Disable the pager", "Tắt trình phân trang"]),
    ("-C", ["Run from a directory", "Chạy từ một thư mục"]),
    (
        "-c",
        [
            "Set configuration for this invocation",
            "Đặt cấu hình cho lần chạy",
        ],
    ),
    ("--git-dir", ["Set the Git directory", "Đặt thư mục Git"]),
    ("--work-tree", ["Set the working tree", "Đặt cây làm việc"]),
];

const DETACH: Item = ("--detach", ["Use a detached HEAD", "Dùng HEAD tách rời"]);
const FORCE: Item = (
    "--force",
    ["Force the operation", "Buộc thực hiện thao tác"],
);
const QUIET: Item = (
    "--quiet",
    ["Suppress normal output", "Ẩn đầu ra thông thường"],
);
const TRACK: Item = ("--track", ["Set an upstream branch", "Đặt nhánh theo dõi"]);
const PATCH: Item = (
    "--patch",
    ["Select changes interactively", "Chọn thay đổi tương tác"],
);
const CACHED: Item = ("--cached", ["Use the staging area", "Dùng vùng chờ"]);
const ALL: Item = ("--all", ["Include all entries", "Bao gồm tất cả mục"]);
const VERBOSE: Item = (
    "--verbose",
    ["Show detailed output", "Hiện đầu ra chi tiết"],
);

pub(super) fn git_options(command: &str) -> &'static [Item] {
    match command {
        "checkout" => &[
            DETACH,
            TRACK,
            FORCE,
            QUIET,
            PATCH,
            (
                "--ours",
                ["Use our version", "Dùng phiên bản của nhánh hiện tại"],
            ),
            (
                "--theirs",
                ["Use their version", "Dùng phiên bản của nhánh kia"],
            ),
            ("-b", ["Create a branch", "Tạo nhánh"]),
        ],
        "switch" => &[
            DETACH,
            TRACK,
            QUIET,
            ("--create", ["Create a branch", "Tạo nhánh"]),
            (
                "--force-create",
                ["Create or reset a branch", "Tạo hoặc đặt lại nhánh"],
            ),
            (
                "--discard-changes",
                ["Discard local changes", "Bỏ thay đổi cục bộ"],
            ),
            ("-c", ["Create a branch", "Tạo nhánh"]),
            ("-C", ["Create or reset a branch", "Tạo hoặc đặt lại nhánh"]),
        ],
        "status" => &[
            (
                "--short",
                ["Show a compact status", "Hiện trạng thái rút gọn"],
            ),
            (
                "--branch",
                ["Include branch information", "Bao gồm thông tin nhánh"],
            ),
            (
                "--porcelain",
                [
                    "Use stable machine-readable output",
                    "Dùng đầu ra ổn định cho chương trình",
                ],
            ),
            (
                "--untracked-files",
                [
                    "Select the untracked-file mode",
                    "Chọn chế độ hiển thị tệp chưa theo dõi",
                ],
            ),
        ],
        "log" | "show" => &[
            (
                "--oneline",
                ["Show one line per commit", "Hiện mỗi commit trên một dòng"],
            ),
            ("--graph", ["Draw the commit graph", "Vẽ sơ đồ commit"]),
            ("--decorate", ["Show ref names", "Hiện tên ref"]),
            (
                "--stat",
                ["Show a change summary", "Hiện thống kê thay đổi"],
            ),
            ALL,
        ],
        "add" => &[
            ALL,
            PATCH,
            FORCE,
            VERBOSE,
            (
                "--dry-run",
                [
                    "Preview staged changes",
                    "Xem trước thay đổi sẽ đưa vào vùng chờ",
                ],
            ),
            (
                "--update",
                [
                    "Stage changes to tracked paths",
                    "Đưa thay đổi của đường dẫn đã theo dõi vào vùng chờ",
                ],
            ),
        ],
        "commit" => &[
            (
                "--message",
                ["Set the commit message", "Đặt nội dung commit"],
            ),
            ("--amend", ["Amend the last commit", "Sửa commit gần nhất"]),
            (
                "--no-edit",
                ["Keep the existing message", "Giữ nội dung hiện có"],
            ),
            ALL,
            VERBOSE,
            QUIET,
        ],
        "diff" => &[
            (
                "--stat",
                ["Show a change summary", "Hiện thống kê thay đổi"],
            ),
            (
                "--staged",
                ["Show staged changes", "Xem thay đổi trong vùng chờ"],
            ),
            (
                "--name-only",
                ["Show changed paths", "Hiện đường dẫn đã thay đổi"],
            ),
            CACHED,
        ],
        "branch" => &[
            ALL,
            ("--list", ["List branches", "Liệt kê nhánh"]),
            (
                "--show-current",
                ["Show the current branch", "Hiện nhánh hiện tại"],
            ),
            ("--delete", ["Delete a branch", "Xóa nhánh"]),
            VERBOSE,
        ],
        "restore" => &[
            (
                "--staged",
                ["Restore the staging area", "Khôi phục vùng chờ"],
            ),
            (
                "--worktree",
                ["Restore the working tree", "Khôi phục cây làm việc"],
            ),
            ("--source", ["Choose the source tree", "Chọn cây nguồn"]),
            PATCH,
        ],
        "stash" => &[
            PATCH,
            QUIET,
            (
                "--include-untracked",
                ["Include untracked files", "Bao gồm tệp chưa theo dõi"],
            ),
        ],
        "push" => &[
            (
                "--set-upstream",
                ["Set an upstream branch", "Đặt nhánh theo dõi"],
            ),
            (
                "--force-with-lease",
                [
                    "Check expected remote state before overwriting",
                    "Kiểm tra trạng thái từ xa trước khi ghi đè",
                ],
            ),
            ("--dry-run", ["Preview the push", "Xem trước thao tác push"]),
            ALL,
        ],
        "pull" => &[
            ("--rebase", ["Rebase local commits", "Rebase commit cục bộ"]),
            (
                "--ff-only",
                ["Require a fast-forward", "Yêu cầu hợp nhất tiến nhanh"],
            ),
            QUIET,
        ],
        "fetch" => &[
            ALL,
            (
                "--prune",
                [
                    "Remove stale remote-tracking refs",
                    "Xóa ref theo dõi từ xa đã lỗi thời",
                ],
            ),
            VERBOSE,
        ],
        "clone" => &[
            (
                "--depth",
                ["Limit history depth", "Giới hạn độ sâu lịch sử"],
            ),
            ("--branch", ["Select a branch", "Chọn nhánh"]),
            QUIET,
        ],
        "merge" | "rebase" | "cherry-pick" => &[
            (
                "--abort",
                [
                    "Abort the in-progress operation",
                    "Hủy thao tác đang thực hiện",
                ],
            ),
            (
                "--continue",
                ["Continue the operation", "Tiếp tục thao tác"],
            ),
            QUIET,
        ],
        _ => &[],
    }
}

pub(super) fn git_actions(command: &str) -> &'static [Item] {
    match command {
        "stash" => &[
            ("list", ["List stashes", "Liệt kê thay đổi đã tạm cất"]),
            ("push", ["Save work temporarily", "Tạm cất thay đổi"]),
            ("show", ["Show a stash", "Xem thay đổi đã tạm cất"]),
            ("apply", ["Apply a stash", "Áp dụng thay đổi đã tạm cất"]),
            (
                "pop",
                ["Apply and drop a stash", "Áp dụng rồi bỏ mục tạm cất"],
            ),
            ("drop", ["Remove a stash", "Xóa mục tạm cất"]),
        ],
        "remote" => &[
            ("add", ["Add a remote", "Thêm kho từ xa"]),
            ("show", ["Show a remote", "Xem kho từ xa"]),
            ("rename", ["Rename a remote", "Đổi tên kho từ xa"]),
            ("remove", ["Remove a remote", "Xóa kho từ xa"]),
        ],
        "worktree" => &[
            ("list", ["List working trees", "Liệt kê cây làm việc"]),
            ("add", ["Add a working tree", "Thêm cây làm việc"]),
            ("remove", ["Remove a working tree", "Xóa cây làm việc"]),
        ],
        "submodule" => &[
            ("add", ["Add a submodule", "Thêm mô-đun con"]),
            (
                "status",
                ["Show submodule state", "Hiện trạng thái mô-đun con"],
            ),
            ("init", ["Initialize submodules", "Khởi tạo các mô-đun con"]),
            (
                "update",
                [
                    "Update submodule checkouts",
                    "Cập nhật bản làm việc của mô-đun con",
                ],
            ),
            (
                "sync",
                ["Synchronize submodule URLs", "Đồng bộ URL mô-đun con"],
            ),
        ],
        "bisect" => &[
            ("start", ["Start a bisection", "Bắt đầu tìm kiếm nhị phân"]),
            (
                "good",
                ["Mark a revision as good", "Đánh dấu phiên bản tốt"],
            ),
            ("bad", ["Mark a revision as bad", "Đánh dấu phiên bản lỗi"]),
            (
                "skip",
                [
                    "Skip untestable revisions",
                    "Bỏ qua phiên bản không thể kiểm thử",
                ],
            ),
            ("reset", ["End the bisection", "Kết thúc tìm kiếm nhị phân"]),
        ],
        "maintenance" => &[
            ("run", ["Run maintenance tasks", "Chạy tác vụ bảo trì"]),
            ("start", ["Schedule maintenance", "Lên lịch bảo trì"]),
            ("stop", ["Stop scheduled maintenance", "Dừng lịch bảo trì"]),
            (
                "register",
                ["Register this repository", "Đăng ký kho mã này"],
            ),
            (
                "unregister",
                ["Unregister this repository", "Hủy đăng ký kho mã này"],
            ),
        ],
        "reflog" => &[
            ("show", ["Show a reflog", "Hiện nhật ký ref"]),
            ("exists", ["Check for a reflog", "Kiểm tra nhật ký ref"]),
            (
                "expire",
                ["Expire old reflog entries", "Hết hạn mục nhật ký ref cũ"],
            ),
            ("delete", ["Delete a reflog entry", "Xóa mục nhật ký ref"]),
        ],
        "bundle" => &[
            ("create", ["Create a bundle", "Tạo bundle"]),
            ("verify", ["Verify a bundle", "Xác minh bundle"]),
            (
                "list-heads",
                ["List bundle refs", "Liệt kê ref trong bundle"],
            ),
            (
                "unbundle",
                ["Unpack bundle objects", "Giải nén đối tượng trong bundle"],
            ),
        ],
        _ => &[],
    }
}

pub(super) const POWERSHELL: &[Item] = &[
    (
        "Get-ChildItem",
        ["List items in a location", "Liệt kê mục tại vị trí"],
    ),
    (
        "Get-Content",
        ["Read content from a file", "Đọc nội dung tệp"],
    ),
    (
        "Get-Command",
        ["Find PowerShell commands", "Tìm lệnh PowerShell"],
    ),
    (
        "Get-Location",
        ["Show the current location", "Hiện vị trí hiện tại"],
    ),
    (
        "Get-Help",
        ["Display help for a command", "Hiện trợ giúp của lệnh"],
    ),
    (
        "Set-Location",
        ["Change the current location", "Đổi vị trí hiện tại"],
    ),
    (
        "Test-Path",
        ["Test whether a path exists", "Kiểm tra đường dẫn tồn tại"],
    ),
    (
        "Resolve-Path",
        [
            "Resolve a PowerShell path",
            "Phân giải đường dẫn PowerShell",
        ],
    ),
    ("Copy-Item", ["Copy items", "Sao chép mục"]),
    ("Move-Item", ["Move items", "Di chuyển mục"]),
    ("New-Item", ["Create an item", "Tạo mục"]),
    ("Remove-Item", ["Remove items", "Xóa mục"]),
    ("Rename-Item", ["Rename an item", "Đổi tên mục"]),
    (
        "Select-Object",
        [
            "Select objects or properties",
            "Chọn đối tượng hoặc thuộc tính",
        ],
    ),
    ("Where-Object", ["Filter objects", "Lọc đối tượng"]),
    (
        "Write-Output",
        [
            "Write objects to the pipeline",
            "Ghi đối tượng vào pipeline",
        ],
    ),
    ("cd", ["Alias for Set-Location", "Bí danh của Set-Location"]),
    (
        "dir",
        ["Alias for Get-ChildItem", "Bí danh của Get-ChildItem"],
    ),
    (
        "ls",
        ["Alias for Get-ChildItem", "Bí danh của Get-ChildItem"],
    ),
    (
        "gci",
        ["Alias for Get-ChildItem", "Bí danh của Get-ChildItem"],
    ),
    ("cat", ["Alias for Get-Content", "Bí danh của Get-Content"]),
    ("gc", ["Alias for Get-Content", "Bí danh của Get-Content"]),
    (
        "pwd",
        ["Alias for Get-Location", "Bí danh của Get-Location"],
    ),
    ("cp", ["Alias for Copy-Item", "Bí danh của Copy-Item"]),
    ("mv", ["Alias for Move-Item", "Bí danh của Move-Item"]),
    ("rm", ["Alias for Remove-Item", "Bí danh của Remove-Item"]),
    ("Get-Module", ["List modules", "Liệt kê mô-đun"]),
    ("Import-Module", ["Import a module", "Nhập mô-đun"]),
    ("Get-Process", ["List processes", "Liệt kê tiến trình"]),
    (
        "Select-String",
        ["Search text for patterns", "Tìm mẫu trong văn bản"],
    ),
    (
        "Measure-Object",
        ["Measure object properties", "Đo thuộc tính đối tượng"],
    ),
    ("Sort-Object", ["Sort objects", "Sắp xếp đối tượng"]),
    (
        "ConvertFrom-Json",
        ["Read JSON as objects", "Đọc JSON thành đối tượng"],
    ),
];

pub(super) fn ps_command(command: &str) -> &str {
    match command {
        "ls" | "dir" | "gci" => "get-childitem",
        "cat" | "gc" | "type" => "get-content",
        "cd" | "chdir" | "sl" => "set-location",
        "cp" | "copy" => "copy-item",
        "mv" | "move" => "move-item",
        "rm" | "del" => "remove-item",
        _ => command,
    }
}

const PATH: Item = ("-Path", ["Set the item path", "Đặt đường dẫn của mục"]);
const LITERAL_PATH: Item = (
    "-LiteralPath",
    ["Use the path literally", "Dùng nguyên văn đường dẫn"],
);
const FILTER: Item = ("-Filter", ["Filter the results", "Lọc kết quả"]);
const PS_RECURSE: Item = (
    "-Recurse",
    ["Include nested items", "Bao gồm mục lồng nhau"],
);
const PS_FORCE: Item = (
    "-Force",
    [
        "Include otherwise restricted items",
        "Bao gồm mục thường bị hạn chế",
    ],
);
const DESTINATION: Item = (
    "-Destination",
    ["Set the destination path", "Đặt đường dẫn đích"],
);
const WHAT_IF: Item = ("-WhatIf", ["Preview the operation", "Xem trước thao tác"]);

pub(super) fn powershell_options(command: &str) -> &'static [Item] {
    match command {
        "get-childitem" => &[
            PATH,
            LITERAL_PATH,
            FILTER,
            PS_RECURSE,
            PS_FORCE,
            ("-Directory", ["List directories", "Liệt kê thư mục"]),
            ("-File", ["List files", "Liệt kê tệp"]),
            ("-Name", ["Show item names", "Hiện tên mục"]),
            ("-Hidden", ["Include hidden items only", "Chỉ hiện mục ẩn"]),
            (
                "-Depth",
                ["Limit recursion depth", "Giới hạn độ sâu đệ quy"],
            ),
        ],
        "get-content" => &[
            PATH,
            LITERAL_PATH,
            FILTER,
            (
                "-Raw",
                ["Read content as one string", "Đọc nội dung thành một chuỗi"],
            ),
            ("-Tail", ["Read the last lines", "Đọc các dòng cuối"]),
            (
                "-TotalCount",
                ["Limit the number of lines", "Giới hạn số dòng"],
            ),
            ("-Encoding", ["Choose the file encoding", "Chọn mã hóa tệp"]),
        ],
        "set-location" | "test-path" | "resolve-path" => &[PATH, LITERAL_PATH],
        "copy-item" | "move-item" => &[PATH, LITERAL_PATH, DESTINATION, PS_FORCE, WHAT_IF],
        "remove-item" => &[PATH, LITERAL_PATH, PS_RECURSE, PS_FORCE, WHAT_IF],
        "new-item" => &[
            PATH,
            PS_FORCE,
            WHAT_IF,
            ("-ItemType", ["Set the item type", "Đặt loại mục"]),
        ],
        "rename-item" => &[
            PATH,
            LITERAL_PATH,
            WHAT_IF,
            ("-NewName", ["Set the new name", "Đặt tên mới"]),
        ],
        "select-object" => &[
            (
                "-First",
                ["Select the first objects", "Chọn các đối tượng đầu"],
            ),
            (
                "-Last",
                ["Select the last objects", "Chọn các đối tượng cuối"],
            ),
            ("-Property", ["Choose properties", "Chọn thuộc tính"]),
        ],
        "get-command" | "get-help" => &[("-Name", ["Set the command name", "Đặt tên lệnh"])],
        "get-process" => &[
            (
                "-Name",
                ["Filter by process name", "Lọc theo tên tiến trình"],
            ),
            ("-Id", ["Filter by process ID", "Lọc theo ID tiến trình"]),
        ],
        "get-module" => &[
            ("-Name", ["Set the module name", "Đặt tên mô-đun"]),
            (
                "-ListAvailable",
                ["List installed modules", "Liệt kê mô-đun đã cài"],
            ),
        ],
        "select-string" => &[
            PATH,
            LITERAL_PATH,
            ("-Pattern", ["Set the search pattern", "Đặt mẫu tìm kiếm"]),
            (
                "-SimpleMatch",
                ["Match literal text", "Khớp văn bản nguyên văn"],
            ),
            (
                "-CaseSensitive",
                ["Match letter case", "Phân biệt hoa thường"],
            ),
        ],
        _ => &[],
    }
}

pub(super) const CMD: &[Item] = &[
    (
        "dir",
        ["List files and directories", "Liệt kê tệp và thư mục"],
    ),
    ("cd", ["Change directories", "Đổi thư mục"]),
    ("chdir", ["Change directories", "Đổi thư mục"]),
    ("type", ["Display a file", "Hiện nội dung tệp"]),
    ("copy", ["Copy files", "Sao chép tệp"]),
    ("move", ["Move files", "Di chuyển tệp"]),
    ("where", ["Locate matching files", "Tìm các tệp phù hợp"]),
    (
        "findstr",
        ["Search text for patterns", "Tìm mẫu trong văn bản"],
    ),
    ("mkdir", ["Create directories", "Tạo thư mục"]),
    ("rmdir", ["Remove directories", "Xóa thư mục"]),
    ("del", ["Delete files", "Xóa tệp"]),
    ("echo", ["Display a message", "Hiện thông báo"]),
    (
        "set",
        [
            "Show or set environment variables",
            "Hiện hoặc đặt biến môi trường",
        ],
    ),
    (
        "ver",
        ["Show the Windows version", "Hiện phiên bản Windows"],
    ),
    (
        "assoc",
        ["Manage file associations", "Quản lý liên kết tệp"],
    ),
    (
        "call",
        ["Call a batch script or label", "Gọi tập lệnh lô hoặc nhãn"],
    ),
    (
        "exit",
        ["Exit CMD or a batch script", "Thoát CMD hoặc tập lệnh lô"],
    ),
    ("for", ["Repeat a command", "Lặp lại một lệnh"]),
    (
        "ftype",
        ["Manage file type commands", "Quản lý lệnh cho loại tệp"],
    ),
    (
        "goto",
        ["Jump to a batch label", "Chuyển đến nhãn trong tập lệnh lô"],
    ),
    (
        "if",
        ["Run a conditional command", "Chạy lệnh có điều kiện"],
    ),
    (
        "pushd",
        ["Save and change directories", "Lưu và đổi thư mục"],
    ),
    (
        "popd",
        [
            "Restore the previous directory",
            "Khôi phục thư mục trước đó",
        ],
    ),
    (
        "setlocal",
        [
            "Start local environment changes",
            "Bắt đầu thay đổi môi trường cục bộ",
        ],
    ),
    (
        "endlocal",
        [
            "End local environment changes",
            "Kết thúc thay đổi môi trường cục bộ",
        ],
    ),
    ("pause", ["Wait for a key press", "Chờ thao tác nhấn phím"]),
];

pub(super) const CMD_FLAGS: &[Item] = &[
    ("/C", ["Run a command and exit", "Chạy lệnh rồi thoát"]),
    (
        "/K",
        ["Run a command and stay open", "Chạy lệnh và giữ shell mở"],
    ),
    ("/D", ["Disable AutoRun commands", "Tắt các lệnh AutoRun"]),
];

pub(super) fn cmd_options(command: &str) -> &'static [Item] {
    match command {
        "dir" => &[
            ("/B", ["Show bare names", "Chỉ hiện tên"]),
            ("/A", ["Filter by attributes", "Lọc theo thuộc tính"]),
            ("/O", ["Choose the sort order", "Chọn thứ tự sắp xếp"]),
            ("/S", ["Include subdirectories", "Bao gồm thư mục con"]),
            ("/W", ["Use a wide listing", "Dùng danh sách dạng rộng"]),
            ("/P", ["Pause after each screen", "Dừng sau mỗi màn hình"]),
        ],
        "cd" | "chdir" => &[("/D", ["Change drive and directory", "Đổi ổ đĩa và thư mục"])],
        "copy" | "move" => &[(
            "/Y",
            ["Suppress overwrite confirmation", "Bỏ xác nhận ghi đè"],
        )],
        "findstr" => &[
            ("/I", ["Ignore case", "Bỏ qua chữ hoa và chữ thường"]),
            ("/S", ["Search subdirectories", "Tìm trong thư mục con"]),
            ("/N", ["Print matching line numbers", "Hiện số dòng khớp"]),
        ],
        "tasklist" => &[
            ("/FI", ["Filter running tasks", "Lọc tác vụ đang chạy"]),
            ("/FO", ["Set the output format", "Đặt định dạng đầu ra"]),
            ("/NH", ["Omit column headings", "Bỏ tiêu đề cột"]),
            ("/V", ["Show task details", "Hiện chi tiết tác vụ"]),
        ],
        "ipconfig" => &[
            (
                "/all",
                [
                    "Show the full adapter configuration",
                    "Hiện đầy đủ cấu hình bộ điều hợp",
                ],
            ),
            (
                "/displaydns",
                [
                    "Display the DNS resolver cache",
                    "Hiện bộ nhớ đệm phân giải DNS",
                ],
            ),
        ],
        "where" => &[
            (
                "/R",
                [
                    "Search from a directory recursively",
                    "Tìm đệ quy từ một thư mục",
                ],
            ),
            (
                "/Q",
                [
                    "Return a status without listing matches",
                    "Trả trạng thái mà không liệt kê kết quả",
                ],
            ),
            (
                "/F",
                [
                    "Quote matching file names",
                    "Đặt tên tệp khớp trong dấu ngoặc kép",
                ],
            ),
        ],
        "set" => &[
            (
                "/A",
                ["Evaluate an arithmetic expression", "Tính biểu thức số học"],
            ),
            (
                "/P",
                [
                    "Read a variable value from input",
                    "Đọc giá trị biến từ đầu vào",
                ],
            ),
        ],
        _ => &[],
    }
}

pub(super) const BASH: &[Item] = &[
    (
        "ls",
        ["List files and directories", "Liệt kê tệp và thư mục"],
    ),
    ("cd", ["Change directories", "Đổi thư mục"]),
    ("cat", ["Display files", "Hiện nội dung tệp"]),
    (
        "grep",
        ["Search text for patterns", "Tìm mẫu trong văn bản"],
    ),
    ("find", ["Search a directory tree", "Tìm trong cây thư mục"]),
    (
        "pwd",
        ["Print the working directory", "In thư mục làm việc"],
    ),
    ("echo", ["Display a message", "Hiện thông báo"]),
    (
        "printf",
        ["Format and print text", "Định dạng và in văn bản"],
    ),
    ("cp", ["Copy files", "Sao chép tệp"]),
    ("mv", ["Move files", "Di chuyển tệp"]),
    ("rm", ["Remove files", "Xóa tệp"]),
    ("mkdir", ["Create directories", "Tạo thư mục"]),
    ("rmdir", ["Remove empty directories", "Xóa thư mục rỗng"]),
    ("head", ["Read the beginning of files", "Đọc phần đầu tệp"]),
    ("tail", ["Read the end of files", "Đọc phần cuối tệp"]),
    ("less", ["Page through text", "Phân trang văn bản"]),
    (
        "which",
        ["Find executables on PATH", "Tìm tệp thực thi trong PATH"],
    ),
    (
        "export",
        ["Set exported variables", "Đặt biến xuất ra môi trường"],
    ),
    (
        "source",
        ["Read shell commands from a file", "Đọc lệnh shell từ tệp"],
    ),
    ("bash", ["Start the Bash shell", "Mở Bash"]),
    (
        "read",
        ["Read values into variables", "Đọc giá trị vào biến"],
    ),
    (
        "command",
        ["Run or inspect a command", "Chạy hoặc kiểm tra một lệnh"],
    ),
    (
        "type",
        ["Describe command resolution", "Mô tả cách phân giải lệnh"],
    ),
    (
        "set",
        [
            "Set shell options or arguments",
            "Đặt tùy chọn shell hoặc đối số",
        ],
    ),
    (
        "shopt",
        [
            "Set Bash optional behavior",
            "Đặt hành vi tùy chọn của Bash",
        ],
    ),
    ("jobs", ["Show background jobs", "Hiện các tác vụ chạy nền"]),
    ("wait", ["Wait for jobs to finish", "Chờ tác vụ hoàn tất"]),
];

pub(super) const WSL_FLAGS: &[Item] = &[
    ("--exec", ["Run a Linux command", "Chạy lệnh Linux"]),
    (
        "--distribution",
        ["Choose a Linux distribution", "Chọn bản phân phối Linux"],
    ),
    ("--user", ["Choose the Linux user", "Chọn người dùng Linux"]),
    ("--status", ["Show WSL settings", "Hiện thiết lập WSL"]),
    (
        "--list",
        ["List Linux distributions", "Liệt kê bản phân phối Linux"],
    ),
    (
        "--cd",
        [
            "Set the starting Linux directory",
            "Đặt thư mục Linux ban đầu",
        ],
    ),
    (
        "--shutdown",
        ["Shut down the WSL virtual machine", "Tắt máy ảo WSL"],
    ),
    (
        "--terminate",
        ["Stop a selected distribution", "Dừng bản phân phối đã chọn"],
    ),
    (
        "--set-default",
        ["Set the default distribution", "Đặt bản phân phối mặc định"],
    ),
    (
        "--set-version",
        [
            "Change a distribution's WSL version",
            "Đổi phiên bản WSL của bản phân phối",
        ],
    ),
    ("--export", ["Export a distribution", "Xuất bản phân phối"]),
    ("--import", ["Import a distribution", "Nhập bản phân phối"]),
];

pub(super) fn bash_options(command: &str) -> &'static [Item] {
    match command {
        "ls" => &[
            ALL,
            (
                "--human-readable",
                ["Use readable file sizes", "Hiện kích thước tệp dễ đọc"],
            ),
            ("--almost-all", ["Omit . and ..", "Bỏ qua . và .."]),
            (
                "--directory",
                ["List directories themselves", "Liệt kê chính thư mục"],
            ),
            (
                "--recursive",
                ["List subdirectories", "Liệt kê thư mục con"],
            ),
            ("--sort", ["Choose the sort order", "Chọn thứ tự sắp xếp"]),
            (
                "--color",
                ["Choose when to use color", "Chọn khi nào dùng màu"],
            ),
            (
                "-l",
                [
                    "Use a long listing",
                    "Dùng danh sách với thông tin chi tiết",
                ],
            ),
            ("-a", ["Include hidden entries", "Bao gồm mục ẩn"]),
        ],
        "grep" => &[
            (
                "--ignore-case",
                ["Ignore case", "Bỏ qua chữ hoa và chữ thường"],
            ),
            (
                "--recursive",
                ["Search directories recursively", "Tìm đệ quy trong thư mục"],
            ),
            (
                "--line-number",
                ["Show matching line numbers", "Hiện số dòng khớp"],
            ),
            (
                "--fixed-strings",
                [
                    "Treat patterns as literal strings",
                    "Dùng mẫu dưới dạng chuỗi ký tự",
                ],
            ),
            (
                "--invert-match",
                ["Select nonmatching lines", "Chọn các dòng không khớp"],
            ),
        ],
        "find" => &[
            ("-name", ["Match by name", "Khớp theo tên"]),
            ("-type", ["Match by file type", "Khớp theo loại tệp"]),
            (
                "-maxdepth",
                ["Limit traversal depth", "Giới hạn độ sâu duyệt"],
            ),
            ("-print", ["Print matching paths", "In đường dẫn khớp"]),
        ],
        "cp" | "mv" | "rm" => &[
            (
                "--interactive",
                ["Prompt before acting", "Hỏi trước khi thực hiện"],
            ),
            VERBOSE,
        ],
        "mkdir" => &[
            (
                "--parents",
                [
                    "Create missing parent directories",
                    "Tạo thư mục cha còn thiếu",
                ],
            ),
            VERBOSE,
        ],
        "head" | "tail" => &[("--lines", ["Set the number of lines", "Đặt số dòng"])],
        _ => bash_portable_options(command),
    }
}

pub(super) fn bash_portable_options(command: &str) -> &'static [Item] {
    const INTERACTIVE: Item = ("-i", ["Prompt before acting", "Hỏi trước khi thực hiện"]);
    match command {
        "ls" => &[
            ("-a", ["Include hidden entries", "Bao gồm mục ẩn"]),
            (
                "-d",
                ["List directories themselves", "Liệt kê chính thư mục"],
            ),
            (
                "-l",
                [
                    "Use a long listing",
                    "Dùng danh sách với thông tin chi tiết",
                ],
            ),
            ("-R", ["List subdirectories", "Liệt kê thư mục con"]),
        ],
        "grep" => &[
            ("-i", ["Ignore case", "Bỏ qua chữ hoa và chữ thường"]),
            ("-n", ["Show matching line numbers", "Hiện số dòng khớp"]),
            (
                "-F",
                [
                    "Treat patterns as literal strings",
                    "Dùng mẫu dưới dạng chuỗi ký tự",
                ],
            ),
            (
                "-v",
                ["Select nonmatching lines", "Chọn các dòng không khớp"],
            ),
        ],
        "find" => &[
            ("-name", ["Match by name", "Khớp theo tên"]),
            ("-type", ["Match by file type", "Khớp theo loại tệp"]),
            ("-print", ["Print matching paths", "In đường dẫn khớp"]),
        ],
        "cp" | "mv" | "rm" => &[INTERACTIVE],
        "mkdir" => &[(
            "-p",
            [
                "Create missing parent directories",
                "Tạo thư mục cha còn thiếu",
            ],
        )],
        "head" | "tail" => &[("-n", ["Set the number of lines", "Đặt số dòng"])],
        "read" => &[
            (
                "-r",
                [
                    "Read backslashes literally",
                    "Đọc dấu gạch chéo ngược nguyên văn",
                ],
            ),
            ("-p", ["Display a prompt", "Hiện lời nhắc"]),
            ("-t", ["Set the input timeout", "Đặt thời gian chờ đầu vào"]),
        ],
        "type" => &[("-a", ["Show all command matches", "Hiện mọi lệnh khớp"])],
        _ => &[],
    }
}
