pub(crate) type Localized = [&'static str; 2];

pub(crate) fn text(value: Localized, vi: bool) -> &'static str {
    value[usize::from(vi)]
}

pub(crate) struct Entry {
    pub label: Localized,
    pub detail: Localized,
    pub command: &'static str,
    pub prompt: bool,
}

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        label: ["Diagnostics", "Chẩn đoán"],
        detail: [
            "Inspect the installed toolchains and runtime prerequisites.",
            "Kiểm tra toolchain và các điều kiện để chạy hệ thống.",
        ],
        command: "doctor",
        prompt: false,
    },
    Entry {
        label: ["Environment", "Môi trường"],
        detail: [
            "Review the selected profile and its local ports.",
            "Xem hồ sơ đang chọn và các cổng cục bộ.",
        ],
        command: "env show",
        prompt: false,
    },
    Entry {
        label: ["Development setup", "Thiết lập phát triển"],
        detail: [
            "Prepare a development profile and optional Chromium.",
            "Chuẩn bị hồ sơ phát triển và Chromium nếu cần.",
        ],
        command: "setup",
        prompt: true,
    },
    Entry {
        label: ["Repositories", "Kho mã nguồn"],
        detail: [
            "Discover and inspect the independent CIRNETLAB repositories.",
            "Tìm và kiểm tra các kho mã nguồn CIRNETLAB.",
        ],
        command: "repos status",
        prompt: false,
    },
    Entry {
        label: ["Build", "Bản dựng"],
        detail: [
            "Build the full native stack or selected components in dependency order.",
            "Build hệ thống native hoặc thành phần đã chọn theo thứ tự phụ thuộc.",
        ],
        command: "build --all",
        prompt: true,
    },
    Entry {
        label: ["Run Interface", "Chạy Interface"],
        detail: [
            "Start the native stack and open the packaged Interface after readiness.",
            "Khởi động hệ thống native và mở Interface đóng gói khi sẵn sàng.",
        ],
        command: "run --desktop interface",
        prompt: true,
    },
    Entry {
        label: ["Develop Interface", "Phát triển Interface"],
        detail: [
            "Start native services and serve live Interface sources with Vite.",
            "Khởi động dịch vụ native và phục vụ mã Interface qua Vite.",
        ],
        command: "dev --desktop interface",
        prompt: true,
    },
    Entry {
        label: ["Runtime status", "Trạng thái hệ thống"],
        detail: [
            "Show the owned services, health, ports and Interface URL.",
            "Hiển thị dịch vụ thuộc CLI, tình trạng, cổng và URL Interface.",
        ],
        command: "status",
        prompt: false,
    },
    Entry {
        label: ["Verification", "Kiểm chứng"],
        detail: [
            "Run candidate checks or verify selected components.",
            "Kiểm chứng ứng viên hoặc các thành phần đã chọn.",
        ],
        command: "verify",
        prompt: true,
    },
    Entry {
        label: ["Stop", "Dừng"],
        detail: [
            "Ask the supervisor to shut down the owned application.",
            "Yêu cầu bộ giám sát dừng ứng dụng do CLI quản lý.",
        ],
        command: "stop",
        prompt: true,
    },
    Entry {
        label: ["Command reference", "Tham chiếu lệnh"],
        detail: [
            "Show command syntax and available options.",
            "Hiển thị cú pháp lệnh và các tùy chọn.",
        ],
        command: "--help",
        prompt: false,
    },
    Entry {
        label: ["Exit", "Thoát"],
        detail: ["Close the terminal menu.", "Đóng menu terminal."],
        command: "",
        prompt: false,
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
