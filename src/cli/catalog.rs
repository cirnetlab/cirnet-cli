pub(crate) use crate::localization::{Localized, text};

pub(crate) struct Entry {
    pub label: Localized,
    pub detail: Localized,
    pub command: &'static str,
    pub recommendation: Option<&'static str>,
}

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        label: ["Diagnostics", "Chẩn đoán"],
        detail: [
            "Inspect the installed toolchains and runtime prerequisites.",
            "Kiểm tra toolchain và các điều kiện để chạy hệ thống.",
        ],
        command: "doctor",
        recommendation: None,
    },
    Entry {
        label: ["Environment", "Môi trường"],
        detail: [
            "Review the selected profile and its local ports.",
            "Xem hồ sơ đang chọn và các cổng cục bộ.",
        ],
        command: "env",
        recommendation: Some("show"),
    },
    Entry {
        label: ["Development setup", "Thiết lập phát triển"],
        detail: [
            "Prepare a development profile and optional Chromium.",
            "Chuẩn bị hồ sơ phát triển và Chromium nếu cần.",
        ],
        command: "setup",
        recommendation: Some("--browser"),
    },
    Entry {
        label: ["Repositories", "Kho mã nguồn"],
        detail: [
            "Discover and inspect the independent CIRNETLAB repositories.",
            "Tìm và kiểm tra các kho mã nguồn CIRNETLAB.",
        ],
        command: "repos",
        recommendation: Some("status"),
    },
    Entry {
        label: ["Build", "Bản dựng"],
        detail: [
            "Build the full native stack or selected components in dependency order.",
            "Build hệ thống native hoặc thành phần đã chọn theo thứ tự phụ thuộc.",
        ],
        command: "build",
        recommendation: Some("--all"),
    },
    Entry {
        label: ["Run Program", "Chạy chương trình"],
        detail: [
            "Start Interface in development mode or choose the packaged build.",
            "Chạy Interface ở chế độ phát triển hoặc chọn bản đóng gói.",
        ],
        command: "run",
        recommendation: Some("--desktop interface"),
    },
    Entry {
        label: ["Runtime status", "Trạng thái hệ thống"],
        detail: [
            "Show the owned services, health, ports and Interface URL.",
            "Hiển thị dịch vụ thuộc CLI, tình trạng, cổng và URL Interface.",
        ],
        command: "status",
        recommendation: None,
    },
    Entry {
        label: ["Verification", "Kiểm chứng"],
        detail: [
            "Run candidate checks or verify selected components.",
            "Kiểm chứng ứng viên hoặc các thành phần đã chọn.",
        ],
        command: "verify",
        recommendation: None,
    },
    Entry {
        label: ["Stop", "Dừng"],
        detail: [
            "Ask the supervisor to shut down the owned application.",
            "Yêu cầu bộ giám sát dừng ứng dụng do CLI quản lý.",
        ],
        command: "stop",
        recommendation: None,
    },
    Entry {
        label: ["Runtime logs", "Nhật ký runtime"],
        detail: [
            "Read the current or latest owned service log.",
            "Đọc nhật ký gần nhất của dịch vụ do CLI quản lý.",
        ],
        command: "logs",
        recommendation: None,
    },
    Entry {
        label: ["Native shell", "Shell native"],
        detail: [
            "Open a native shell in the current terminal.",
            "Mở shell native trong terminal hiện tại.",
        ],
        command: "shell",
        recommendation: None,
    },
    Entry {
        label: ["Shell completion", "Gợi ý lệnh trên shell"],
        detail: [
            "Generate a shell completion script.",
            "Tạo script gợi ý lệnh cho shell.",
        ],
        command: "completion",
        recommendation: None,
    },
    Entry {
        label: ["Command reference", "Tham chiếu lệnh"],
        detail: [
            "Show command syntax and available options.",
            "Hiển thị cú pháp lệnh và các tùy chọn.",
        ],
        command: "--help",
        recommendation: None,
    },
    Entry {
        label: ["Exit", "Thoát"],
        detail: ["Close the terminal menu.", "Đóng menu terminal."],
        command: "",
        recommendation: None,
    },
];

pub(crate) fn detail(words: &str, vi: bool) -> Option<&'static str> {
    ENTRIES
        .iter()
        .find(|entry| {
            entry.command == words || entry.command.split_whitespace().next() == Some(words)
        })
        .map(|entry| text(entry.detail, vi))
}

pub(crate) fn localize(command: clap::Command, vi: bool) -> clap::Command {
    localize_command(command, "", vi)
}

fn localize_command(mut command: clap::Command, parent: &str, vi: bool) -> clap::Command {
    let name = command.get_name();
    let description = match (parent, name) {
        ("repos", "status") => Some([
            "Inspect repository branches, revisions and working trees",
            "Kiểm tra nhánh, revision và cây làm việc của các repo",
        ]),
        (_, name) => match name {
            "cnl" => Some(["CIRNETLAB developer tools", "Công cụ phát triển CIRNETLAB"]),
            "tui" => Some(["Open the CIRNET LAB terminal", "Mở terminal CIRNET LAB"]),
            "dev" => Some([
                "Start Interface in development mode (legacy)",
                "Chạy Interface ở chế độ phát triển (lệnh tương thích)",
            ]),
            "show" => Some(["Show the current profile", "Hiển thị hồ sơ hiện tại"]),
            "check" => Some([
                "Check profile and toolchain prerequisites",
                "Kiểm tra hồ sơ và điều kiện toolchain",
            ]),
            "set" => Some([
                "Update local profile ports",
                "Cập nhật cổng của hồ sơ cục bộ",
            ]),
            "discover" => Some([
                "List expected repository locations",
                "Liệt kê vị trí repo dự kiến",
            ]),
            "clone" => Some([
                "Clone selected missing checkouts",
                "Clone các checkout còn thiếu đã chọn",
            ]),
            "update" => Some([
                "Fast-forward selected clean checkouts",
                "Cập nhật fast-forward các checkout sạch đã chọn",
            ]),
            _ => None,
        },
    }
    .map(|value| text(value, vi))
    .or_else(|| detail(name, vi));
    if let Some(description) = description {
        command = command.about(description);
    }
    let parent = command.get_name().to_owned();
    if vi {
        command = command
            .disable_help_flag(true)
            .arg(
                clap::Arg::new("help")
                    .short('h')
                    .long("help")
                    .action(clap::ArgAction::Help),
            )
            .subcommand_help_heading("Các lệnh")
            .help_template(
                "{before-help}{about-with-newline}\nCách dùng: {usage}\n\n{all-args}{after-help}",
            );
        if parent == "cnl" {
            command = command.disable_version_flag(true).arg(
                clap::Arg::new("version")
                    .short('V')
                    .long("version")
                    .action(clap::ArgAction::Version),
            );
        }
    }
    command
        .mut_args(
            |arg| match crate::localization::option(arg.get_id().as_str()) {
                Some(label) if vi || arg.get_help().is_none() => {
                    if vi {
                        let heading = if arg.get_long().is_some() {
                            "Tùy chọn"
                        } else {
                            "Đối số"
                        };
                        arg.help(text(label, vi)).help_heading(heading)
                    } else {
                        arg.help(text(label, vi))
                    }
                }
                None => arg,
                Some(_) => arg,
            },
        )
        .mut_subcommands(|child| localize_command(child, &parent, vi))
}
