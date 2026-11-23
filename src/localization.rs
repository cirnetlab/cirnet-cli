use std::{cell::Cell, ffi::OsString};

pub(crate) type Localized = [&'static str; 2];

thread_local! {
    static VIETNAMESE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_language(vi: bool) {
    VIETNAMESE.set(vi);
}

pub(crate) fn message(value: Localized) -> &'static str {
    text(value, VIETNAMESE.get())
}

pub(crate) fn text(value: Localized, vi: bool) -> &'static str {
    value[usize::from(vi)]
}

pub(crate) fn requested_language(args: impl IntoIterator<Item = OsString>) -> bool {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if arg == "--lang" {
            return args.next().is_some_and(|value| value == "vi");
        }
        if let Some(value) = arg.to_str().and_then(|value| value.strip_prefix("--lang=")) {
            return value == "vi";
        }
    }
    false
}

pub(crate) fn option(id: &str) -> Option<Localized> {
    Some(match id {
        "workspace" => [
            "Parent directory of the CIRNETLAB checkouts",
            "Thư mục cha của các checkout CIRNETLAB",
        ],
        "state-dir" => [
            "Absolute external directory for state and artifacts",
            "Đường dẫn tuyệt đối ngoài repo để lưu trạng thái và artifact",
        ],
        "profile" => ["Select the runtime profile", "Chọn hồ sơ runtime"],
        "json" => [
            "Emit a JSON report on stdout",
            "Xuất báo cáo JSON qua stdout",
        ],
        "lang" => ["Terminal language", "Ngôn ngữ terminal"],
        "port" => ["Local HTTP port", "Cổng HTTP cục bộ"],
        "hardware-port" => ["Local Hardware RPC port", "Cổng Hardware RPC cục bộ"],
        "dev-port" => ["Local Vite port", "Cổng Vite cục bộ"],
        "receipt" => [
            "Select a verified assembly-result.json",
            "Chọn tệp assembly-result.json đã kiểm chứng",
        ],
        "desktop" => [
            "Select the Interface desktop target",
            "Chọn đích Interface trên desktop",
        ],
        "mode" => [
            "Choose the Interface runtime mode",
            "Chọn chế độ chạy Interface",
        ],
        "initialize-db" => [
            "Initialize a new local SQLite database",
            "Khởi tạo cơ sở dữ liệu SQLite cục bộ mới",
        ],
        "open" => [
            "Open the application after readiness",
            "Mở ứng dụng sau khi dịch vụ sẵn sàng",
        ],
        "dry-run" => ["Preview this operation", "Xem trước thao tác này"],
        "all" => ["Select the full scope", "Chọn toàn bộ phạm vi"],
        "targets" => ["Repository keys", "Khóa repo"],
        "repo" => [
            "Select a repository; repeat to add more",
            "Chọn repo, lặp lại tùy chọn để thêm repo",
        ],
        "offline" => ["Use cached dependency inputs", "Dùng phụ thuộc trong cache"],
        "fpga-toolchain" => ["Pinned RTL toolchain root", "Thư mục RTL toolchain đã ghim"],
        "browser" => [
            "Install the selected candidate's Chromium",
            "Cài Chromium của ứng viên đã chọn",
        ],
        "browser-cache" => [
            "Absolute Playwright Chromium cache path",
            "Đường dẫn tuyệt đối đến cache Chromium của Playwright",
        ],
        "service" => ["Select the runtime log", "Chọn nhật ký runtime"],
        "lines" => ["Maximum number of log lines", "Số dòng nhật ký tối đa"],
        "shell" => ["Select a native shell", "Chọn shell native"],
        "help" => ["Print help", "Hiển thị trợ giúp"],
        "version" => ["Print version", "Hiển thị phiên bản"],
        _ => return None,
    })
}

pub(crate) fn report_field(key: &str, vi: bool) -> &str {
    if !vi {
        return key;
    }
    match key {
        "architecture" => "kiến trúc",
        "browser" => "trình duyệt",
        "browserCache" => "cache trình duyệt",
        "buildOrder" => "thứ tự build",
        "bundle" => "gói",
        "dataDirectory" => "thư mục dữ liệu",
        "database" => "cơ sở dữ liệu",
        "doctor" => "chẩn đoán",
        "dryRun" => "xem trước",
        "initializeDb" => "khởi tạo cơ sở dữ liệu",
        "integration" => "kiểm thử tích hợp",
        "manifestSha256" => "SHA-256 của manifest",
        "mode" => "chế độ",
        "ok" => "đạt",
        "openBrowser" => "mở trình duyệt",
        "operations" => "thao tác",
        "os" => "hệ điều hành",
        "output" => "đầu ra",
        "ownedInstance" => "phiên do CLI quản lý",
        "ports" => "cổng",
        "profile" => "hồ sơ",
        "receipt" => "receipt",
        "repositories" => "kho mã nguồn",
        "state" => "trạng thái",
        "stateDirectory" => "thư mục trạng thái",
        "supportedPlatform" => "nền tảng được hỗ trợ",
        "target" | "targets" => "đích",
        "tools" => "công cụ",
        _ => key,
    }
}

pub(crate) const RUNNING: Localized = ["Running", "Đang chạy"];
pub(crate) const SUCCEEDED: Localized = ["PASS", "ĐẠT"];
pub(crate) const FAILED: Localized = ["FAIL", "LỖI"];
pub(crate) const CANCELLED: Localized = ["CANCELLED", "ĐÃ HỦY"];
pub(crate) const EXIT: Localized = ["Exit", "Mã thoát"];
pub(crate) const DURATION: Localized = ["Duration", "Thời gian"];
pub(crate) const LOG_REDACTED: &str = "[redacted]";
pub(crate) const MISSING_INVENTORY: Localized = [
    "Missing candidate inventory",
    "Thiếu danh mục artifact của ứng viên",
];
pub(crate) const START_FAILED: Localized = ["Could not start", "Không thể khởi động"];
