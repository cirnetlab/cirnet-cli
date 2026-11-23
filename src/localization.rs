use std::{cell::Cell, ffi::OsString, fmt};

use clap::error::{ContextKind, ErrorKind};
use serde_json::{Value, json};

pub(crate) mod terminal;

pub(crate) type Localized = [&'static str; 2];

macro_rules! messages {
    ($($(#[$attribute:meta])* $id:ident => $en:literal, $vi:literal;)*) => {
        $($(#[$attribute])* pub(crate) const $id: Localized = [$en, $vi];)*
    };
}

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

#[derive(Debug)]
pub(crate) struct Diagnostic([String; 2]);

impl Diagnostic {
    pub(crate) fn new(render: impl Fn(bool) -> String) -> Self {
        Self([render(false), render(true)])
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0[usize::from(VIETNAMESE.get())])
    }
}

pub(crate) fn tui_report_nonce() -> Option<String> {
    std::env::var("CNL_TUI_REPORT").ok().filter(|nonce| {
        !nonce.is_empty()
            && nonce.len() <= 64
            && nonce.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

pub(crate) fn report_diagnostic(error: &anyhow::Error) -> Value {
    error.downcast_ref::<Diagnostic>().map_or_else(
        || json!(error.to_string()),
        |detail| detail_value(&detail.0),
    )
}

pub(crate) fn report_message(detail: Localized) -> Value {
    detail_value(&detail)
}

fn detail_value(detail: &[impl AsRef<str>; 2]) -> Value {
    let en = detail[0].as_ref();
    let vi = detail[1].as_ref();
    if crate::process::json_output() {
        json!(en)
    } else if tui_report_nonce().is_some() {
        json!({"$cnlDiagnostic": [en, vi]})
    } else {
        json!(if VIETNAMESE.get() { vi } else { en })
    }
}

pub(crate) fn human_detail(value: &Value, vi: bool) -> Option<&str> {
    value.get("$cnlDiagnostic")?[usize::from(vi)].as_str()
}

pub(crate) fn requested_language(args: impl IntoIterator<Item = OsString>) -> bool {
    let mut vi = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if arg == "--lang" {
            match args.next().as_deref().and_then(|value| value.to_str()) {
                Some("en") => vi = false,
                Some("vi") => vi = true,
                _ => {}
            }
        }
        match arg.to_str().and_then(|value| value.strip_prefix("--lang=")) {
            Some("en") => vi = false,
            Some("vi") => vi = true,
            _ => {}
        }
    }
    vi
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
        "persist-history" => [
            "Save eligible CNL command history between terminal sessions",
            "Lưu lịch sử lệnh CNL đủ điều kiện giữa các phiên terminal",
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
        "cursor" => ["Cursor offset", "Vị trí con trỏ"],
        "line" => ["Command line", "Dòng lệnh"],
        "help" => ["Print help", "Hiển thị trợ giúp"],
        "version" => ["Print version", "Hiển thị phiên bản"],
        _ => return None,
    })
}

pub(crate) fn report_field(key: &str) -> Option<Localized> {
    Some(match key {
        "architecture" => ["Architecture", "Kiến trúc"],
        "branch" => ["Branch", "Nhánh"],
        "browser" => ["Browser", "Trình duyệt"],
        "browserCache" => ["Browser cache", "Bộ nhớ đệm trình duyệt"],
        "buildOrder" => ["Build order", "Thứ tự bản dựng"],
        "bundle" => ["Bundle", "Gói"],
        "changes" => ["Changes", "Thay đổi"],
        "dataDirectory" => ["Data directory", "Thư mục dữ liệu"],
        "database" => ["Database", "Cơ sở dữ liệu"],
        "dependencies" => ["Dependencies", "Thành phần phụ thuộc"],
        "detail" => ["Details", "Chi tiết"],
        "doctor" => ["Diagnostics", "Chẩn đoán"],
        "dryRun" => ["Preview", "Xem trước"],
        "hardwareDetail" => ["Hardware RPC details", "Chi tiết Hardware RPC"],
        "hardwarePid" => ["Hardware PID", "PID của Hardware"],
        "hardwareRpc" => ["Hardware RPC", "Hardware RPC"],
        "hardware_port" => ["Hardware RPC port", "Cổng Hardware RPC"],
        "healthy" => ["Health", "Tình trạng hoạt động"],
        "http" => ["HTTP", "HTTP"],
        "initializeDb" => ["Initialize database", "Khởi tạo cơ sở dữ liệu"],
        "integration" => ["Integration checks", "Kiểm thử tích hợp"],
        "interface" => ["Interface", "Interface"],
        "key" => ["Repository key", "Khóa kho mã nguồn"],
        "logDirectory" => ["Log directory", "Thư mục nhật ký"],
        "manifestSha256" => ["Manifest SHA-256", "SHA-256 của manifest"],
        "mode" => ["Mode", "Chế độ"],
        "native" => ["Native services", "Dịch vụ native"],
        "ok" => ["Result", "Kết quả"],
        "openBrowser" => ["Open browser", "Mở trình duyệt"],
        "operation" => ["Operation", "Thao tác"],
        "operations" => ["Operations", "Các thao tác"],
        "os" => ["Operating system", "Hệ điều hành"],
        "output" => ["Output directory", "Thư mục đầu ra"],
        "ownedInstance" => ["Owned instance", "Phiên do CLI quản lý"],
        "path" => ["Path", "Đường dẫn"],
        "port" => ["HTTP port", "Cổng HTTP"],
        "ports" => ["Ports", "Các cổng"],
        "present" => ["Present", "Có sẵn"],
        "profile" => ["Profile", "Hồ sơ"],
        "receipt" => ["Receipt", "Phiếu xác nhận"],
        "remotePid" => ["Remote PID", "PID của Remote"],
        "remotes" => ["Git remotes", "Remote Git"],
        "repo" => ["Repository", "Kho mã nguồn"],
        "repositories" => ["Repositories", "Các kho mã nguồn"],
        "revision" => ["Revision", "Bản sửa đổi"],
        "service" => ["Service", "Dịch vụ"],
        "services" => ["Service processes", "Tiến trình dịch vụ"],
        "state" => ["Status", "Trạng thái"],
        "stateDirectory" => ["State directory", "Thư mục trạng thái"],
        "supervisorPid" => ["Supervisor PID", "PID bộ giám sát"],
        "supportedPlatform" => ["Supported platform", "Nền tảng được hỗ trợ"],
        "target" => ["Target", "Đích"],
        "targets" => ["Targets", "Các đích"],
        "tool" => ["Tool", "Công cụ"],
        "tools" => ["Tools", "Các công cụ"],
        "url" => ["URL", "URL"],
        "version" => ["Version", "Phiên bản"],
        "vite" => ["Vite", "Vite"],
        "vitePid" => ["Vite PID", "PID của Vite"],
        "dev_port" => ["Vite port", "Cổng Vite"],
        "websocket" => ["WebSocket", "WebSocket"],
        _ => return None,
    })
}

pub(crate) fn report_value(key: &str, value: &str) -> Option<Localized> {
    Some(match (key, value) {
        ("state", "built") => ["Built", "Đã dựng"],
        ("state", "checkout") => ["Available", "Có checkout"],
        ("state", "exited-without-confirmed-stop") => [
            "Exited without confirmed Stop",
            "Đã thoát trước khi xác nhận Stop",
        ],
        ("state", "invalid") => ["Invalid checkout", "Checkout không hợp lệ"],
        ("state", "missing") => ["Missing checkout", "Thiếu checkout"],
        ("state", "ready") => ["Ready", "Sẵn sàng"],
        ("state", "stopped") => ["Stopped", "Đã dừng"],
        ("state", "unhealthy") => ["Unhealthy", "Dịch vụ gặp lỗi"],
        ("state", "verified") => ["Verified", "Đã kiểm chứng"],
        ("mode", "development") => ["Development", "Phát triển"],
        ("mode", "packaged") => ["Packaged", "Đóng gói"],
        ("operation", "clone") => ["Clone", "Sao chép checkout"],
        ("operation", "update") => ["Update", "Cập nhật checkout"],
        ("remotes", "unrecognized remote") => {
            ["Unrecognized remote", "Remote không nhận dạng được"]
        }
        _ => return None,
    })
}

pub(crate) fn report_boolean(key: &str, value: bool) -> Localized {
    match (key, value) {
        ("ok" | "http" | "interface" | "hardwareRpc" | "websocket", true) => SUCCEEDED,
        ("ok" | "http" | "interface" | "hardwareRpc" | "websocket", false) => FAILED,
        ("healthy", true) => ["Healthy", "Hoạt động bình thường"],
        ("healthy", false) => ["Unhealthy", "Dịch vụ gặp lỗi"],
        (_, true) => ["Yes", "Có"],
        (_, false) => ["No", "Không"],
    }
}

// Clap does not expose localized words for its generated help annotations.
// Translate only the grammar metadata it injects into our help template.
fn vietnamese_help(help: &str) -> String {
    help.replace("[default: ", "[mặc định: ")
        .replace("[possible values: ", "[giá trị hợp lệ: ")
        .replace("[aliases: ", "[bí danh: ")
        .replace(
            "Print this message or the help of the given subcommand(s)",
            "Hiển thị trợ giúp cho lệnh hoặc lệnh con đã chọn",
        )
        .replace(
            "Print help for the subcommand(s)",
            "Hiển thị trợ giúp cho lệnh con đã chọn",
        )
}

pub(crate) fn clap_diagnostic(error: &clap::Error) -> String {
    use ContextKind as C;
    use ErrorKind as E;

    match error.kind() {
        E::DisplayHelp | E::DisplayHelpOnMissingArgumentOrSubcommand => {
            return vietnamese_help(&error.to_string());
        }
        E::DisplayVersion => return error.to_string(),
        _ => {}
    }
    let context = |key| error.get(key).map(ToString::to_string).unwrap_or_default();
    let arg = context(C::InvalidArg);
    let subcommand = context(C::InvalidSubcommand);
    let value = context(C::InvalidValue);
    let detail = match error.kind() {
        E::InvalidValue | E::ValueValidation if value.is_empty() => {
            format!("Thiếu giá trị hợp lệ cho '{arg}'.")
        }
        E::InvalidValue | E::ValueValidation => {
            format!("Giá trị không hợp lệ '{value}' cho '{arg}'.")
        }
        E::UnknownArgument => format!("Đối số không hợp lệ '{arg}'."),
        E::InvalidSubcommand => format!("Lệnh con không hợp lệ '{subcommand}'."),
        E::NoEquals => format!("Dùng dấu '=' để gán giá trị cho '{arg}'."),
        E::TooManyValues => format!("Quá nhiều giá trị cho '{arg}' tại '{value}'."),
        E::TooFewValues => format!(
            "'{arg}' cần ít nhất {} giá trị nhưng chỉ nhận được {}.",
            context(C::MinValues),
            context(C::ActualNumValues)
        ),
        E::WrongNumberOfValues => format!(
            "'{arg}' cần {} giá trị nhưng nhận được {}.",
            context(C::ExpectedNumValues),
            context(C::ActualNumValues)
        ),
        E::ArgumentConflict => format!("Không thể dùng '{arg}' cùng với {}.", context(C::PriorArg)),
        E::MissingRequiredArgument => format!("Thiếu đối số bắt buộc: {arg}"),
        E::MissingSubcommand => "Thiếu lệnh con.".to_owned(),
        E::InvalidUtf8 => "Đối số chứa văn bản UTF-8 không hợp lệ.".to_owned(),
        _ => "Không thể phân tích dòng lệnh.".to_owned(),
    };
    let mut output = format!("lỗi: {detail}\n");
    for (key, label) in [
        (C::ValidSubcommand, "Lệnh con hợp lệ"),
        (C::ValidValue, "Giá trị hợp lệ"),
        (C::SuggestedSubcommand, "Lệnh con gợi ý"),
        (C::SuggestedArg, "Đối số gợi ý"),
        (C::SuggestedValue, "Giá trị gợi ý"),
    ] {
        if let Some(value) = error.get(key) {
            output.push_str(&format!("  {label}: {value}\n"));
        }
    }
    if error.kind() == E::ValueValidation {
        let allowed = if ["--port", "--hardware-port", "--dev-port"]
            .iter()
            .any(|name| arg.split_whitespace().next() == Some(name))
        {
            Some("1024..=65535")
        } else if arg.starts_with("--lines ") {
            Some("1..=500")
        } else {
            None
        };
        if let Some(range) = allowed {
            output.push_str(&format!("  Phạm vi hợp lệ: {range}\n"));
        }
    }
    if let Some(usage) = error.get(C::Usage) {
        output.push('\n');
        output.push_str(&usage.to_string().replacen("Usage:", "Cách dùng:", 1));
        output.push('\n');
    }
    output.push_str("\nĐể biết thêm chi tiết, hãy dùng '--help'.\n");
    output
}

pub(crate) const REPO_DETACHED_HEAD: Localized = ["detached", "HEAD tách rời"];
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

// Messages generated by CLI command handlers. External error details remain separate.
messages! {
    SHELL_INTERACTIVE => "Open a native shell from an interactive terminal.", "Mở shell native từ terminal tương tác.";
    COMMAND_REQUIRED => "Select a command in non-interactive mode. See cnl --help.", "Hãy chọn một lệnh khi chạy ở chế độ không tương tác. Xem cnl --help.";
    TOOLCHAIN_INCOMPLETE => "Toolchain prerequisites are incomplete", "Chưa đủ điều kiện toolchain";
    SETUP_PREREQUISITES => "Fix the prerequisites reported by cnl doctor before setup", "Hãy xử lý các điều kiện do cnl doctor báo trước khi chạy setup";
    SHELL_PLATFORM => "Shell unavailable on this platform", "Shell không khả dụng trên nền tảng này";
    SHELL_HELP => "See cnl shell --help.", "Xem cnl shell --help.";
    SHELL_START_FAILED => "Cannot start", "Không thể mở";
    SHELL_INSTALL => "Install it or select an available native shell", "Hãy cài đặt hoặc chọn một shell native đang có";
    SHELL_UNAVAILABLE => "The executable or native shell is unavailable. Check PATH or use cnl shell", "Không tìm thấy executable hoặc shell native. Hãy kiểm tra PATH hoặc dùng cnl shell";
    CANDIDATE_SCHEMA => "Unsupported candidate selection schema", "Không hỗ trợ schema lựa chọn ứng viên";
    CANDIDATE_SCOPE => "Candidate selection belongs to", "Lựa chọn ứng viên thuộc về";
    CANDIDATE_SCOPE_GUIDANCE => "Use that scope or choose a different --state-dir or --profile", "Dùng phạm vi đó hoặc chọn --state-dir hay --profile khác";
    CANDIDATE_RECORDED_RECEIPT => "Invalid recorded receipt path or manifest digest", "Đường dẫn receipt hoặc mã băm manifest đã lưu không hợp lệ";
    LEGACY_RECEIPT_PATH => "The legacy receipt path must be absolute", "Đường dẫn receipt cũ phải là đường dẫn tuyệt đối";
    CANDIDATE_SELECT => "Cannot select a candidate for workspace", "Không thể chọn ứng viên cho workspace";
    CANDIDATE_SELECTION_FILE => "Expected selection file", "Cần tệp lựa chọn";
    CANDIDATE_SELECT_GUIDANCE => "Run cnl build --all with the same scope, or pass --receipt <assembly-result.json>", "Chạy cnl build --all trong cùng phạm vi hoặc truyền --receipt <assembly-result.json>";
    RECEIPT_UNUSABLE => "Cannot use receipt", "Không thể dùng receipt";
    RECEIPT_GUIDANCE => "Pass a valid --receipt <assembly-result.json> or run cnl build --all with the same scope", "Truyền --receipt <assembly-result.json> hợp lệ hoặc chạy cnl build --all trong cùng phạm vi";
    RECEIPT_SESSION => "Candidate receipt must pin a build in its own output session", "Receipt phải tham chiếu bản dựng trong chính phiên xuất artifact của ứng viên";
    RECEIPT_DIGEST => "Candidate receipt digest differs from the recorded selection", "Mã băm receipt của ứng viên khác lựa chọn đã lưu";
    ASSEMBLY_RECEIPT_COUNT => "Remote did not return exactly one verified assembly receipt", "Remote phải trả về đúng một receipt lắp ráp đã kiểm chứng";
    CANDIDATE_INPUT_MISSING => "Missing candidate input", "Thiếu đầu vào của ứng viên";
    CANDIDATE_INPUT_MISMATCH => "External input differs from the selected candidate artifact", "Đầu vào bên ngoài khác artifact của ứng viên đã chọn";
    INTERFACE_SELECTION_MISSING => "Interface .artifacts has no selection receipt. Move or inspect it before refreshing dependencies", "Thư mục .artifacts của Interface thiếu receipt lựa chọn. Hãy di chuyển hoặc kiểm tra trước khi làm mới phụ thuộc";
    INTERFACE_CACHE_UNOWNED => "Refusing to move an unowned Interface cache", "Từ chối di chuyển cache Interface chưa xác minh được quyền quản lý";
    INTERFACE_CACHED_ARTIFACT_MISSING => "Missing cached artifact path", "Thiếu đường dẫn artifact đã lưu trong cache";
    INTERFACE_CACHED_ARTIFACT_MISMATCH => "Cached artifact differs from the recorded selection. Inspect", "Artifact trong cache khác lựa chọn đã lưu. Hãy kiểm tra";
    INTERFACE_CACHE_UNKNOWN_FILES => "Interface cache contains unrecognized files. Inspect", "Cache Interface chứa tệp chưa nhận diện. Hãy kiểm tra";
    INTERFACE_RUNTIME_DIRECTORY => "Interface .runtime directory must be an ordinary directory", "Thư mục .runtime của Interface phải là thư mục thông thường";
    INTERFACE_INPUTS_PRESERVED => "Preserved previous Interface inputs at", "Đã lưu các đầu vào Interface trước đó tại";
    VERIFY_RUNNER_MISSING => "The selected candidate has no staged native smoke runner", "Ứng viên đã chọn thiếu trình chạy kiểm thử nhanh native";
    RTL_TOOLCHAIN_REQUIRED => "Set --fpga-toolchain to the pinned Embedded Hardware toolchain root", "Đặt --fpga-toolchain tới thư mục gốc toolchain Embedded Hardware đã ghim";
    PROFILE_PORTS => "HTTP, Hardware RPC and Vite ports must be distinct and in 1024..=65535", "Cổng HTTP, Hardware RPC và Vite phải khác nhau và nằm trong 1024..=65535";
    PROFILE_NAME => "Use a profile name with letters, digits, hyphens or underscores", "Tên hồ sơ chỉ dùng chữ cái, chữ số, dấu gạch ngang hoặc gạch dưới";
    STATE_ABSOLUTE => "--state-dir requires an absolute external path", "--state-dir yêu cầu đường dẫn tuyệt đối nằm ngoài repo";
    STATE_LOCALAPPDATA => "Set --state-dir when LOCALAPPDATA is unavailable", "Đặt --state-dir khi không có LOCALAPPDATA";
    STATE_ANCESTOR => "State path has no existing ancestor", "Đường dẫn trạng thái không có thư mục cha đang tồn tại";
    STATE_DIRECTORY_NAMES => "State path must use ordinary directory names", "Đường dẫn trạng thái phải dùng tên thư mục thông thường";
    STATE_OUTSIDE_GIT => "State and artifacts must be outside repository and Git directories", "Trạng thái và artifact phải nằm ngoài repo và thư mục Git";
    PROFILE_DIRECTORY => "The profile directory must be an ordinary directory", "Thư mục hồ sơ phải là thư mục thông thường";
    PROFILE_IN_USE => "Another cnl operation is using this profile", "Một thao tác cnl khác đang dùng hồ sơ này";
    DOCTOR_MSVC => "Install VS 2022 with the x64 C++ tools and Windows SDK", "Cài VS 2022 với công cụ C++ x64 và Windows SDK";
    VS_DIRECTORY_MISSING => "Visual Studio installation directory unavailable", "Không tìm thấy thư mục cài đặt Visual Studio";
    VS_INSTALLER_MISSING => "Visual Studio Installer not found at", "Không tìm thấy Visual Studio Installer tại";
    BUILD_RECEIPT_REQUIRED => "Expected build receipt path", "Cần đường dẫn receipt của bản dựng";
    PLAYWRIGHT_MISSING => "The selected candidate does not include a prepared Playwright checkout", "Ứng viên đã chọn thiếu checkout đã chuẩn bị cho Playwright";
    PROCESS_EXITED => "exited with", "đã thoát với";
    #[cfg(windows)]
    STDIO_ISOLATION_FAILED => "Cannot isolate the caller's standard handles", "Không thể cô lập các handle chuẩn của tiến trình gọi";
    PATH_RESOLUTION_FAILED => "Cannot resolve", "Không thể phân giải";
    WORKSPACE_MISSING => "Could not find the CIRNETLAB workspace. Set --workspace <directory> or CNL_WORKSPACE.", "Không tìm thấy workspace CIRNETLAB. Đặt --workspace <thư-mục> hoặc CNL_WORKSPACE.";
    CHECKOUT_MISSING => "Checkout missing:", "Thiếu checkout tại";
    CHECKOUT_REQUIRED => "Expected a real checkout:", "Cần checkout Git thực tại";
    CHECKOUT_NOT_INDEPENDENT => "is not an independent Git checkout", "không phải checkout Git độc lập";
    CLONE_DESTINATION_EXISTS => "Clone destination already exists:", "Đường dẫn clone đã tồn tại";
    CHECKOUT_DIRTY => "has local changes. Commit or stash them before updating.", "có thay đổi cục bộ. Hãy commit hoặc stash trước khi cập nhật.";
    CHECKOUT_BRANCH_ORIGIN => "must be on a branch tracking origin", "phải ở nhánh theo dõi origin";
    CHECKOUT_UPSTREAM_MISSING => "has no branch upstream", "chưa có nhánh upstream";
    CHECKOUT_ORIGIN_MISMATCH => "origin is not the expected CIRNETLAB repository", "có origin khác repo CIRNETLAB đã định";
    RUNTIME_TARGET_MISMATCH => "The selected profile owns a different target. Run cnl status to identify it.", "Hồ sơ đang chọn quản lý đích khác. Chạy cnl status để xác định đích.";
    PORT_IN_USE => "is already in use. Select another port or stop its owner.", "đang được sử dụng. Chọn cổng khác hoặc dừng tiến trình đang giữ cổng.";
    PORT_BIND_FAILED => "Cannot bind", "Không thể mở cổng";
    PORT_BIND_GUIDANCE => "Check for another listener or select a different port", "Hãy kiểm tra tiến trình đang lắng nghe hoặc chọn cổng khác";
    RUNTIME_PLATFORM => "CIRNETLAB native runtime requires Windows x64", "Runtime native CIRNETLAB yêu cầu Windows x64";
    RUNTIME_ALREADY_OWNED => "This profile already owns a running process. Inspect cnl status before starting again.", "Hồ sơ này đang quản lý một tiến trình. Xem cnl status trước khi khởi động lại.";
    RUNTIME_EXITED_UNCONFIRMED => "The previous instance exited without a confirmed cnl stop. Inspect", "Phiên trước đã thoát trước khi xác nhận cnl stop. Hãy kiểm tra";
    RUNTIME_EXITED_GUIDANCE => "Use a different profile until shutdown is resolved", "Dùng hồ sơ khác cho đến khi xử lý xong việc dừng phiên";
    SQLITE_MISSING => "No initialized SQLite database in", "Chưa có cơ sở dữ liệu SQLite đã khởi tạo trong";
    SQLITE_INITIALIZE => "Use --initialize-db to create a fresh database", "Dùng --initialize-db để tạo cơ sở dữ liệu mới";
    NATIVE_READINESS_FAILED => "Native application failed HTTP, WebSocket, Interface or Hardware RPC readiness", "Ứng dụng native chưa sẵn sàng ở HTTP, WebSocket, Interface hoặc Hardware RPC";
    VITE_READINESS_FAILED => "Vite frontend, proxied API or proxied WebSocket is unhealthy", "Giao diện Vite, API qua proxy hoặc WebSocket qua proxy gặp lỗi";
    RUNTIME_START_FAILED => "Startup failed. See", "Khởi động thất bại. Xem";
    BROWSER_OPEN_GUIDANCE => "Open in your browser", "Mở bằng trình duyệt";
    VITE_CLEANUP_MISSING => "Vite exited without a cleanup receipt", "Vite đã thoát khi chưa có biên nhận dọn dẹp";
    VITE_CLEANUP => "Vite cleanup", "Dọn dẹp Vite";
    NATIVE_SHUTDOWN_UNCONFIRMED => "The owned supervisor already exited. Clean native shutdown cannot be confirmed.", "Supervisor thuộc CLI đã thoát. Không thể xác nhận hệ thống native đã dừng an toàn.";
    STOP_LAUNCHER_CHANGED => "The selected Stop launcher has changed. Restore the verified file before stopping.", "Launcher dùng để Stop đã thay đổi. Hãy khôi phục tệp đã kiểm chứng trước khi dừng.";
    RUNTIME_RUN_MISMATCH => "The launcher run identity no longer matches the cnl-owned instance.", "Định danh phiên launcher không còn khớp với phiên do cnl quản lý.";
    LAUNCHER_ID_MISSING => "Missing launcher run identity", "Thiếu định danh phiên launcher";
    LAUNCHER_ID_INVALID => "Invalid launcher run identity", "Định danh phiên launcher không hợp lệ";
    REMOTE_EXECUTABLE_CHANGED => "The selected Remote executable changed since startup", "Executable Remote đã thay đổi kể từ khi khởi động";
    CANDIDATE_VERIFIED_FILE_MISSING => "Candidate is missing the verified file", "Ứng viên thiếu tệp đã kiểm chứng";
    INTERFACE_FINGERPRINT_MISSING => "Candidate has no Interface source fingerprint", "Ứng viên thiếu mã nhận dạng mã nguồn Interface";
    SERVICE_EXITED => "Service exited with", "Dịch vụ đã thoát với";
    READINESS_BEFORE => "before reporting readiness in", "trước khi báo sẵn sàng trong";
    READY_TIMEOUT => "Timed out waiting for readiness in", "Hết thời gian chờ báo sẵn sàng trong";
    OWNED_PROCESS => "Owned process", "Tiến trình do CLI quản lý";
    SHUTDOWN_UNCONFIRMED => "shutdown could not be confirmed", "không xác nhận được việc dừng";
    LOGS_MISSING => "No runtime logs for this profile. Use cnl status to check the selected profile", "Hồ sơ này chưa có nhật ký runtime. Dùng cnl status để kiểm tra hồ sơ đã chọn";
    LOGS_SESSION_OUTSIDE_PROFILE => "Runtime log session is outside the selected profile", "Thư mục nhật ký runtime nằm ngoài hồ sơ đã chọn";
    LOG_MISSING => "Missing runtime log", "Thiếu nhật ký runtime";
    LOG_INVALID => "Expected an ordinary runtime log inside the selected profile", "Nhật ký runtime phải là tệp thường trong hồ sơ đã chọn";
    PROCESS_GONE_BEFORE_IDENTITY => "Owned process exited before identity could be recorded", "Tiến trình do CLI quản lý đã thoát trước khi ghi nhận định danh";
    #[cfg(windows)]
    PROCESS_INSPECT_FAILED => "Cannot inspect process", "Không thể kiểm tra tiến trình";
    #[cfg(not(windows))]
    PROCESS_CONTROL_PLATFORM => "Runtime process control requires Windows x64", "Điều khiển tiến trình runtime yêu cầu Windows x64";
}

messages! {
    MENU_EXIT_DETAIL => "Close the terminal menu.", "Đóng menu terminal.";
    MENU_EXIT_LABEL => "Exit", "Thoát";
    MENU_REFERENCE_DETAIL => "Show command syntax and available options.", "Hiển thị cú pháp lệnh và các tùy chọn.";
    MENU_REFERENCE_LABEL => "Command reference", "Tham chiếu lệnh";
    MENU_COMPLETION_DETAIL => "Generate a shell completion script.", "Tạo script gợi ý lệnh cho shell.";
    MENU_COMPLETION_LABEL => "Shell completion", "Gợi ý lệnh trên shell";
    MENU_SHELL_DETAIL => "Open a native shell in the current terminal.", "Mở shell native trong terminal hiện tại.";
    MENU_SHELL_LABEL => "Native shell", "Shell native";
    MENU_LOGS_DETAIL => "Read the current or latest owned service log.", "Đọc nhật ký gần nhất của dịch vụ do CLI quản lý.";
    MENU_LOGS_LABEL => "Runtime logs", "Nhật ký runtime";
    MENU_STOP_DETAIL => "Ask the supervisor to shut down the owned application.", "Yêu cầu bộ giám sát dừng ứng dụng do CLI quản lý.";
    MENU_STOP_LABEL => "Stop", "Dừng";
    MENU_VERIFY_DETAIL => "Run candidate checks or verify selected components.", "Kiểm chứng ứng viên hoặc các thành phần đã chọn.";
    MENU_VERIFY_LABEL => "Verification", "Kiểm chứng";
    MENU_STATUS_DETAIL => "Show the owned services, health, ports and Interface URL.", "Hiển thị dịch vụ thuộc CLI, tình trạng, cổng và URL Interface.";
    MENU_STATUS_LABEL => "Runtime status", "Trạng thái hệ thống";
    MENU_RUN_DETAIL => "Start Interface in development mode or choose the packaged build.", "Chạy Interface ở chế độ phát triển hoặc chọn bản đóng gói.";
    MENU_RUN_LABEL => "Run Program", "Chạy chương trình";
    MENU_BUILD_DETAIL => "Build the full native stack or selected components in dependency order.", "Build hệ thống native hoặc thành phần đã chọn theo thứ tự phụ thuộc.";
    MENU_BUILD_LABEL => "Build", "Bản dựng";
    MENU_REPOS_DETAIL => "Discover and inspect the independent CIRNETLAB repositories.", "Tìm và kiểm tra các kho mã nguồn CIRNETLAB.";
    MENU_REPOS_LABEL => "Repositories", "Kho mã nguồn";
    MENU_SETUP_DETAIL => "Prepare a development profile and optional Chromium.", "Chuẩn bị hồ sơ phát triển và Chromium nếu cần.";
    MENU_SETUP_LABEL => "Development setup", "Thiết lập phát triển";
    MENU_ENV_DETAIL => "Review the selected profile and its local ports.", "Xem hồ sơ đang chọn và các cổng cục bộ.";
    MENU_ENV_LABEL => "Environment", "Môi trường";
    MENU_DOCTOR_DETAIL => "Inspect the installed toolchains and runtime prerequisites.", "Kiểm tra toolchain và các điều kiện để chạy hệ thống.";
    MENU_DOCTOR_LABEL => "Diagnostics", "Chẩn đoán";
    COMMAND_REPO_UPDATE => "Fast-forward selected clean checkouts", "Cập nhật fast-forward các checkout sạch đã chọn";
    COMMAND_REPO_CLONE => "Clone selected missing checkouts", "Clone các checkout còn thiếu đã chọn";
    COMMAND_REPO_DISCOVER => "List expected repository locations", "Liệt kê vị trí repo dự kiến";
    COMMAND_ENV_SET => "Update local profile ports", "Cập nhật cổng của hồ sơ cục bộ";
    COMMAND_ENV_CHECK => "Check profile and toolchain prerequisites", "Kiểm tra hồ sơ và điều kiện toolchain";
    COMMAND_ENV_SHOW => "Show the current profile", "Hiển thị hồ sơ hiện tại";
    COMMAND_DEV => "Start Interface in development mode (legacy)", "Chạy Interface ở chế độ phát triển (lệnh tương thích)";
    COMMAND_TUI => "Open the CIRNET LAB terminal", "Mở terminal CIRNET LAB";
    COMMAND_ROOT => "CIRNETLAB developer tools", "Công cụ phát triển CIRNETLAB";
    COMMAND_REPO_STATUS => "Inspect repository branches, revisions and working trees", "Kiểm tra nhánh, revision và cây làm việc của các repo";
    COMMAND_COMPLETION_QUERY => "Query shell completions", "Truy vấn gợi ý lệnh cho shell";
}
