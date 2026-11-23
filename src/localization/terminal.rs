//! Stable identifiers for application-owned terminal labels.

use super::Localized;

pub(crate) const COMMAND_TOO_LONG: Localized = ["Command exceeds 8 KiB.", "Lệnh vượt quá 8 KiB."];
pub(crate) const INPUT_RECEIVED: Localized = [
    "Input received. Press Enter to run.",
    "Đã nhận nội dung. Nhấn Enter để chạy.",
];
pub(crate) const RUN_IN_PROGRESS: Localized =
    ["A command is still running.", "Một lệnh đang chạy."];
pub(crate) const CANCELLING: Localized = ["Cancelling…", "Đang hủy…"];
pub(crate) const CANNOT_CANCEL: Localized = [
    "Wait for this command to finish. Use stop for the application.",
    "Đợi lệnh hoàn tất. Dùng stop để dừng ứng dụng.",
];
pub(crate) const OUTPUT_OMITTED: Localized = [
    "Earlier output was omitted.",
    "Đã lược bớt phần đầu của kết quả.",
];
pub(crate) const HISTORY_READ_FAILED: Localized = [
    "Could not read profile history. Check tui-history.json or start without --persist-history.",
    "Không đọc được lịch sử hồ sơ. Kiểm tra tui-history.json hoặc mở lại khi bỏ --persist-history.",
];
pub(crate) const HISTORY_WRITE_FAILED: Localized = [
    "Could not save profile history. Check permissions for the selected state directory.",
    "Không lưu được lịch sử hồ sơ. Kiểm tra quyền của thư mục trạng thái đã chọn.",
];
pub(crate) const RECENT_CNL: Localized = ["Recent CNL command", "Lệnh CNL gần đây"];
pub(crate) const RECENT_NATIVE: Localized = ["Recent native command", "Lệnh native gần đây"];
pub(crate) const RECOMMENDED: Localized = ["Recommended action", "Thao tác gợi ý"];
pub(crate) const CLOSE_QUOTE: Localized = [
    "Close the quoted argument.",
    "Hãy đóng dấu nháy của đối số.",
];
pub(crate) const ENTER_CNL_COMMAND: Localized =
    ["Enter a CIRNETLAB command.", "Nhập một lệnh CIRNETLAB."];
pub(crate) const INTERACTIVE_REQUIRED: Localized = [
    "cnl tui requires an interactive terminal",
    "cnl tui yêu cầu terminal tương tác",
];
pub(crate) const TERMINAL_ALREADY_OPEN: Localized = [
    "The CIRNETLAB terminal is already open.",
    "Terminal CIRNETLAB đang mở.",
];
#[cfg(not(windows))]
pub(crate) const CANCELLATION_WINDOWS_ONLY: Localized = [
    "Process tree cancellation is available on Windows",
    "Hủy cây tiến trình được hỗ trợ trên Windows",
];

pub(crate) const MENU_HINT: Localized = [
    "Tab complete · : command · / search · Esc back · Ctrl+C exit · ? help · F8 EN/VI",
    "Tab gợi ý · : lệnh · / tìm · Esc quay lại · Ctrl+C thoát · ? trợ giúp · F8 EN/VI",
];
pub(crate) const MENU_HINT_SHORT: Localized = [
    "Tab · : cmd · / find · Esc · Ctrl+C · ? · F8 EN/VI",
    "Tab · : lệnh · / tìm · Esc · Ctrl+C · ? · F8 EN/VI",
];
pub(crate) const NATIVE_SHELL: Localized = ["Native shell", "Shell native"];
pub(crate) const NO_FIND_MATCHES: Localized = ["  (no matches)", "  (không có kết quả)"];
pub(crate) const FIND_PROMPT: Localized = ["Find  › ", "Tìm  › "];
pub(crate) const MENU_COMMANDS: Localized = ["Commands", "Lệnh"];
pub(crate) const SEARCH_PROMPT: Localized = ["Search  › ", "Tìm kiếm  › "];
pub(crate) const COMMAND_REFERENCE: Localized = ["Command reference", "Tham chiếu lệnh"];
pub(crate) const NO_MATCHING_COMMANDS: Localized =
    ["No matching commands", "Không có lệnh phù hợp"];
pub(crate) const COPY: Localized = ["Copy", "Sao chép"];
pub(crate) const CUT: Localized = ["Cut", "Cắt"];
pub(crate) const PASTE: Localized = ["Paste", "Dán"];
pub(crate) const DELETE: Localized = ["Delete", "Xóa"];
pub(crate) const SELECT_ALL: Localized = ["Select all", "Chọn tất cả"];
pub(crate) const UNDO: Localized = ["Undo", "Hoàn tác"];
pub(crate) const REDO: Localized = ["Redo", "Làm lại"];
pub(crate) const COPY_LINE: Localized = ["Copy line", "Sao chép dòng"];
pub(crate) const SEARCH_TOO_LONG: Localized = [
    "Search text exceeds its limit.",
    "Văn bản tìm kiếm vượt giới hạn.",
];
pub(crate) const CLIPBOARD_SHORTCUT: Localized = [
    "Use the terminal's copy or paste shortcut.",
    "Dùng phím tắt sao chép hoặc dán của terminal.",
];
pub(crate) const COPIED: Localized = ["Copied", "Đã sao chép"];
pub(crate) const SENSITIVE_TEXT: Localized = [
    "Sensitive text was withheld.",
    "Đã giữ lại văn bản nhạy cảm.",
];
