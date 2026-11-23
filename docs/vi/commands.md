# Tham chiếu lệnh CIRNETLAB

[English](../en/commands.md) · [Hướng dẫn phát triển](index.md) · [README](../../README.md)

## Build và kiểm tra CLI bằng Cargo

Chạy các lệnh Cargo trong checkout `cirnet-cli`. Để build toàn bộ hệ thống native, đặt bảy checkout CIRNETLAB trong cùng thư mục workspace. Xem hướng dẫn phát triển để biết các toolchain cần thiết.

| Lệnh | Công dụng |
| --- | --- |
| `cargo --version` | Xem phiên bản Cargo đang dùng. Repo ghim Rust 1.99.1 trong `rust-toolchain.toml`. |
| `cargo check --locked --all-targets` | Kiểm tra kiểu dữ liệu của CLI và các đích kiểm thử. |
| `cargo fmt --all` | Định dạng lại mã Rust. |
| `cargo fmt --all --check` | Kiểm tra định dạng mà không sửa tệp. |
| `cargo clippy --locked --all-targets -- -D warnings` | Chạy Clippy và coi cảnh báo là lỗi. |
| `cargo test --locked` | Chạy bộ kiểm thử của CLI. |
| `cargo test --locked --test commands` | Chỉ chạy bộ kiểm thử tích hợp lệnh CLI. |
| `cargo build --locked --bin cnl` | Build bản Debug. Với thư mục đích mặc định của Cargo, tệp nằm ở `target/debug/cnl.exe`. |
| `cargo build --locked --bins` | Build mọi đích executable. Repo này hiện có một executable là `cnl`. |
| `cargo build --locked --bin cnl --release` | Build bản tối ưu hóa ở `target/release/cnl.exe` với thư mục đích mặc định. |
| `cargo run --locked --bin cnl -- tui --lang vi` | Build rồi mở TUI trực tiếp. Dấu `--` đầu tiên truyền phần đối số còn lại cho `cnl`. |
| `cargo run --locked --bin cnl -- --help` | Build và xem trợ giúp CLI. |
| `cargo tree --locked -e normal` | Xem cây phụ thuộc khi chạy CLI. |

Dùng `--offline` của Cargo với `build`, `check` hoặc `test` khi phụ thuộc cần thiết đã nằm trong cache. Cờ `--locked` giữ nguyên lockfile của repo. Nếu Cargo dùng thư mục đích tùy chỉnh, dùng [ví dụ PowerShell](index.md#build-và-chạy) để lấy đúng đường dẫn executable từ Cargo.

Các ví dụ bên dưới dùng `cnl`. Nếu CLI chưa có trong PATH, dùng `& $cnl` trong PowerShell với đường dẫn mà Cargo trả về.

## Tùy chọn toàn cục

| Tùy chọn | Công dụng |
| --- | --- |
| `--workspace <DIR>` | Chọn thư mục cha của các checkout. Tham số này ưu tiên hơn `CNL_WORKSPACE`. Khi cả hai chưa đặt, CLI tìm từ các thư mục cha của thư mục hiện hành, rồi đến các thư mục cha của executable. |
| `--state-dir <ABSOLUTE_DIR>` | Chọn thư mục tuyệt đối bên ngoài Git checkout để lưu hồ sơ, cơ sở dữ liệu SQLite cục bộ và bản dựng. Mặc định mỗi workspace có thư mục riêng dưới `%LOCALAPPDATA%\CIRNETLAB`. |
| `--profile <NAME>` | Chọn hồ sơ. Mặc định là `windows-x64`. Tên có thể chứa chữ, số, dấu gạch ngang hoặc gạch dưới. Dùng cùng workspace, thư mục trạng thái và hồ sơ trong suốt một vòng chạy. |
| `--json` | Xuất báo cáo có cấu trúc ra stdout để tự động hóa. |
| `--lang en\|vi` | Chọn ngôn ngữ terminal tương tác. Mặc định là `en`. |
| `-h`, `--help` | Hiện trợ giúp cho lệnh đã chọn. |
| `-V`, `--version` | Xem phiên bản CLI ở cấp lệnh chính. |

## Các lệnh

`<KEY>` là một trong các khóa `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface` hoặc `embedded`. `<RECEIPT>` là đường dẫn tới tệp `assembly-result.json` của ứng viên đã kiểm chứng. Dấu ngoặc vuông bao quanh đối số tùy chọn. Dấu ba chấm chỉ đối số có thể lặp lại.

| Lệnh | Hành vi |
| --- | --- |
| `cnl` · `cnl tui` | Mở terminal tương tác. Dùng `cnl tui --lang vi` để hiển thị tiếng Việt. |
| `cnl help [COMMAND ...]` | Liệt kê lệnh hoặc xem trợ giúp lệnh con, ví dụ `cnl help repos update`. |
| `cnl doctor` | Kiểm tra toolchain Windows x64 và các điều kiện của workspace. Trả mã lỗi nếu kiểm tra bắt buộc thất bại. |
| `cnl env` · `cnl env show` | Xem hồ sơ, cổng, thư mục trạng thái và loại cơ sở dữ liệu. |
| `cnl env check` | Xem hồ sơ kèm báo cáo chẩn đoán toolchain. |
| `cnl env set [--port N] [--hardware-port N] [--dev-port N] [--dry-run]` | Lưu hoặc xem trước thay đổi cổng cho hồ sơ. Các cổng không chỉ định giữ giá trị hiện tại. |
| `cnl setup [--dry-run]` | Chuẩn bị hồ sơ sau khi kiểm tra các điều kiện cần thiết. |
| `cnl setup --browser [--receipt <RECEIPT>] [--dry-run]` | Cài Chromium tương ứng với ứng viên đã chọn vào hồ sơ. |
| `cnl repos discover` | Liệt kê đường dẫn checkout dự kiến và trạng thái hiện diện. |
| `cnl repos status` | Xem nhánh, commit, thay đổi cục bộ, remote và trạng thái phụ thuộc. |
| `cnl repos clone <KEY>... [--dry-run]` | Clone các repo đã chỉ định vào những đường dẫn còn thiếu. Tài khoản Git cần quyền đọc các repo đó. |
| `cnl repos update <KEY>... [--dry-run]` | Fetch và cập nhật fast-forward các checkout đã chỉ định. Checkout phải sạch và theo dõi đúng origin CIRNETLAB. |
| `cnl build --all [--offline] [--dry-run]` | Dựng ứng viên Windows x64 qua Remote và lưu receipt cho hồ sơ. Cờ `--offline` dùng phụ thuộc từ cache. |
| `cnl build --repo <KEY> [--repo <KEY> ...] [--offline] [--dry-run] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>]` | Build các thành phần đã chọn theo thứ tự phụ thuộc. Chọn một trong hai cờ `--all` và `--repo`. Interface và Embedded dùng đầu vào từ ứng viên đã có. |
| `cnl run [RUNTIME OPTIONS]` | Khởi động hệ thống native và phục vụ Interface đã đóng gói. |
| `cnl dev [RUNTIME OPTIONS]` | Khởi động các dịch vụ native và phục vụ checkout Interface hiện tại qua Vite có hot reload. |
| `cnl status` | Xem trạng thái hồ sơ, tiến trình do CLI quản lý, cổng thực tế và URL Interface khi đang chạy. |
| `cnl stop [--dry-run]` | Yêu cầu supervisor dừng ứng dụng do hồ sơ đang chọn quản lý. |
| `cnl verify [--receipt <RECEIPT>] [--browser-cache <ABSOLUTE_DIR>] [--dry-run]` | Kiểm chứng ứng viên đóng gói. Lệnh dùng Chromium đã cài trong hồ sơ nếu có. Chỉ định cache tương thích để chạy cả phần kiểm thử trình duyệt. |
| `cnl verify --repo <KEY> [--repo <KEY> ...] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>] [--dry-run]` | Kiểm tra các thành phần đã chọn. Interface dùng đầu vào từ ứng viên. Embedded chạy kiểm thử RTL với bộ công cụ đã ghim. |

Với Embedded, biến `CIRNET_FPGA_TOOLCHAIN_ROOT` cung cấp đường dẫn khi thiếu `--fpga-toolchain`. Đặt `CNL_VERIFICATION_PYTHON` để chọn executable Python cho các kiểm tra Simulator và Embedded. Cờ `--dry-run` chỉ hoạt động ở các lệnh đã liệt kê. Dùng `cnl doctor` để kiểm tra riêng các điều kiện cần thiết.

### Tùy chọn khởi động cho `run` và `dev`

| Tùy chọn | Công dụng |
| --- | --- |
| `--desktop interface`, `--open` | Mở Interface trong trình duyệt mặc định khi kiểm tra sẵn sàng thành công. |
| `--initialize-db` | Cho phép launcher khởi tạo cơ sở dữ liệu SQLite mới. Cơ sở dữ liệu đã có phải qua kiểm tra schema. |
| `--receipt <RECEIPT>` | Chọn ứng viên đã kiểm chứng để chạy. Nếu bỏ qua, CLI dùng receipt cuối cùng mà `build --all` đã lưu cho hồ sơ. |
| `--port N` · `--hardware-port N` · `--dev-port N` | Đổi cổng HTTP, Hardware RPC và Vite cho lần chạy này. Giá trị mặc định là `8000`, `50051` và `5173`. Các cổng phải khác nhau và nằm trong `1024..=65535`. |
| `--dry-run` | Xem trước cấu hình khởi động mà không chạy dịch vụ. |

### Trình tự thường dùng

Chạy Interface đóng gói trước. Dừng phiên đó trước khi chạy Vite với cùng hồ sơ.

```powershell
cnl doctor
cnl setup
cnl build --all
cnl run --desktop interface --initialize-db
cnl status
cnl stop

cnl dev --desktop interface
cnl status
cnl stop

cnl setup --browser
cnl verify
```

Dùng `--initialize-db` ở lần chạy đầu tiên của mỗi hồ sơ mới. Nếu CLI chưa tìm thấy ứng viên, chạy `cnl build --all` với cùng hồ sơ hoặc truyền `--receipt`. Nếu đường dẫn bản dựng trên Windows quá dài, chọn thư mục ngắn bên ngoài repo bằng `--state-dir`.

Ví dụ xem trước và kiểm tra theo phạm vi

```powershell
cnl env set --port 8001 --hardware-port 50052 --dev-port 5174 --dry-run
cnl repos status --json
cnl build --repo remote --dry-run
cnl verify --repo cli
```

## Terminal tương tác

Dùng phím mũi tên để điều hướng, `/` để lọc menu, `:` để nhập lệnh, Tab hoặc Shift+Tab để duyệt gợi ý và `?` để xem trợ giúp. Ctrl+R tìm trong lịch sử lệnh. Page Up và Page Down cuộn đầu ra, Ctrl+F tìm nội dung, F3 chuyển đến kết quả tiếp theo. Nhập `clear` trong TUI để xóa phần kết quả đang hiển thị. Esc quay lại hoặc hủy lệnh hỗ trợ hủy. Dùng `cnl stop` để dừng ứng dụng. Đặt `NO_COLOR=1` trước khi mở TUI để tắt màu.
