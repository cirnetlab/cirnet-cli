use crate::localization::{Localized, message};

pub(super) fn get(value: Localized) -> &'static str {
    message(value)
}

macro_rules! labels {
    ($($name:ident: $en:literal, $vi:literal;)*) => { $(pub(super) const $name: Localized = [$en, $vi];)* };
}

labels! {
    REGISTRY_SCHEMA: "Repository registry version or workspace does not match.", "Phiên bản hoặc workspace trong danh bạ repo không khớp.";
    REGISTRY_INVALID: "Repository registry data is invalid. Inspect the saved file before retrying.", "Dữ liệu danh bạ repo không hợp lệ. Kiểm tra tệp đã lưu trước khi thử lại.";
    REGISTRY_LOCATION: "Repository state must use ordinary files outside Git checkouts.", "Trạng thái repo phải dùng tệp thông thường nằm ngoài các checkout Git.";
    AMBIGUOUS: "Ambiguous repository. Specify an owner/name, alias or path.", "Tên repo chưa xác định duy nhất. Hãy dùng owner/name, bí danh hoặc đường dẫn.";
    NOT_FOUND: "Repository not found:", "Không tìm thấy repo:";
    LOCAL_REQUIRED: "A verified local checkout is required:", "Cần checkout cục bộ đã xác minh:";
    ALREADY_LOCAL: "A checkout is already registered:", "Checkout đã được đăng ký:";
    INVALID_CONFIG: "Invalid repository setting.", "Cấu hình repo không hợp lệ.";
    INVALID_ROOT: "Select an ordinary local directory with --root.", "Chọn thư mục cục bộ thông thường bằng --root.";
    ADD_SCOPE: "Select repositories or use --all with an explicit --root.", "Chọn repo hoặc dùng --all với --root chỉ định rõ.";
    CONFIRM_ALL: "Preview the bulk operation with --dry-run, then use --yes to confirm.", "Xem trước thao tác hàng loạt với --dry-run, rồi dùng --yes để xác nhận.";
    SCOPE_REQUIRED: "Select repositories explicitly or use --all.", "Chỉ định repo hoặc dùng --all.";
    INTERACTIVE: "Repository Manager requires an interactive terminal. Use cnl repos list for a report.", "Trình quản lý repo cần terminal tương tác. Dùng cnl repos list để xem báo cáo.";
    GROUP_MISSING: "Unknown repository group:", "Không tìm thấy nhóm repo:";
    NAME_INVALID: "Use letters, digits, dots, hyphens or underscores in an alias or group name.", "Dùng chữ, số, dấu chấm, gạch ngang hoặc gạch dưới trong bí danh và tên nhóm.";
    IDENTITY_CHANGED: "Checkout remote identity differs from the registry.", "Định danh remote của checkout khác với danh bạ.";
    GH_MISSING: "GitHub CLI is unavailable. Install gh and sign in to github.com.", "Không tìm thấy GitHub CLI. Hãy cài gh và đăng nhập github.com.";
    GH_AUTH: "GitHub authentication failed. Check gh auth status --hostname github.com.", "Xác thực GitHub thất bại. Kiểm tra gh auth status --hostname github.com.";
    GH_FORBIDDEN: "GitHub refused the request. Check repository visibility and organization permissions.", "GitHub từ chối yêu cầu. Kiểm tra quyền truy cập repo và quyền trong tổ chức.";
    GH_RATE: "GitHub rate limit was reached. Retry later or use the saved catalog.", "Đã chạm giới hạn yêu cầu GitHub. Thử lại sau hoặc dùng danh mục đã lưu.";
    GH_UNAVAILABLE: "Unable to refresh GitHub repositories. Check connectivity and the gh version.", "Không làm mới được danh sách repo GitHub. Kiểm tra kết nối và phiên bản gh.";
    GH_BAD_DATA: "GitHub returned invalid repository metadata.", "GitHub trả về metadata repo không hợp lệ.";
    REMOTE_REQUIRED: "Refresh GitHub discovery to select an accessible organization repository.", "Làm mới danh sách GitHub để chọn repo tổ chức có quyền truy cập.";
    DEST_OCCUPIED: "Clone destination already exists:", "Đường dẫn clone đã tồn tại:";
    CONTEXT_REFRESHING: "Repository context is being refreshed. Retry when it is ready.", "Đang cập nhật ngữ cảnh repo. Thử lại khi đã sẵn sàng.";
    SYNC_FAILED: "One or more repository operations did not succeed. See the individual results.", "Có thao tác repo không thành công. Xem kết quả từng repo.";
    SYNC_STARTED: "Starting repository operation", "Đang chạy thao tác repo";
    SYNC_FINISHED: "Finished repository operation", "Đã kết thúc thao tác repo";
}
