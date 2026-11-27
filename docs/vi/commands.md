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
| `--json` | Ghi báo cáo có cấu trúc ra stdout. Chế độ này ẩn tiến độ trên stdout của tiến trình con. Thông báo chẩn đoán vẫn đi qua stderr. |
| `--lang en\|vi` | Chọn ngôn ngữ CLI và terminal tương tác. Mặc định là `en`. |
| `--persist-history` | Lưu lịch sử lệnh CNL đủ điều kiện vào hồ sơ TUI đã chọn. Mặc định lịch sử chỉ nằm trong bộ nhớ. |
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
| `cnl repos` | Mở [Trình quản lý repo](#trình-quản-lý-repo) tương tác. |
| `cnl repos list` · `cnl repos status [REPO ...]` | Liệt kê mục đã đăng ký hoặc kiểm tra checkout và nhánh theo dõi của các repo đã chọn. |
| `cnl repos discover [--root DIR] [--refresh \| --offline] [--query TEXT]` | Tìm checkout cục bộ và repo GitHub mà tài khoản có quyền truy cập. |
| `cnl repos clone <REPO>...` · `cnl repos clone --all` | Clone vào đường dẫn còn trống và đăng ký checkout đã xác minh. Clone hàng loạt mặc định hiển thị bản xem trước. |
| `cnl repos add <REPO-OR-PATH>...` · `cnl repos add --all --root DIR` | Đăng ký checkout Git hiện có. Thêm hàng loạt mặc định hiển thị bản xem trước. |
| `cnl repos register <REPO>` · `cnl repos remove <REPO>...` | Đăng ký repo GitHub chưa có checkout hoặc gỡ mục khỏi danh bạ. |
| `cnl repos update` · `cnl repos update <REPO>...` · `cnl repos update --all` | Mở màn hình chọn hoặc cập nhật các checkout đã chọn theo fast-forward. |
| `cnl repos fetch <REPO>...` · `cnl repos fetch --all` | Fetch từ remote đã cấu hình. Fetch và update hỗ trợ `--jobs N` và `--dry-run`. |
| `cnl build --all [--offline] [--dry-run]` | Dựng ứng viên Windows x64 qua Remote và lưu receipt cho hồ sơ. Cờ `--offline` dùng phụ thuộc từ cache. |
| `cnl build --repo <KEY> [--repo <KEY> ...] [--offline] [--dry-run] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>]` | Build các thành phần đã chọn theo thứ tự phụ thuộc. Chọn một trong hai cờ `--all` và `--repo`. Interface và Embedded dùng đầu vào từ ứng viên đã có. |
| `cnl run [RUNTIME OPTIONS]` | Khởi động hệ thống native với Vite ở chế độ phát triển theo mặc định. Dùng `--mode packaged` để chạy Interface đóng gói đã kiểm chứng. |
| `cnl dev [RUNTIME OPTIONS]` | Lệnh tương thích cho `run --mode development`. |
| `cnl status` | Xem đích, chế độ, dịch vụ do CLI quản lý, cổng thực tế và URL Interface của hồ sơ đã chọn. |
| `cnl stop [--desktop interface] [--dry-run]` | Yêu cầu supervisor dừng phiên do hồ sơ đã chọn quản lý. Đích được chỉ định phải khớp với phiên đó. |
| `cnl logs [--service native\|vite] [--lines N]` | Đọc phần cuối nhật ký runtime đang chạy hoặc gần nhất trong hồ sơ đã chọn. Mặc định đọc 60 dòng của `native`. |
| `cnl shell [PROFILE]` | Mở shell tương tác tại thư mục hiện tại. Có thể chọn PowerShell 7, CMD, Bash native, Git Bash hoặc WSL Bash tùy nền tảng và môi trường đã cài. |
| `cnl completion <powershell\|bash>` | Xuất script gợi ý lệnh cho shell qua stdout. |
| `cnl verify [--receipt <RECEIPT>] [--browser-cache <ABSOLUTE_DIR>] [--dry-run]` | Kiểm chứng ứng viên đóng gói. Lệnh dùng Chromium đã cài trong hồ sơ nếu có. Chỉ định cache tương thích để chạy cả phần kiểm thử trình duyệt. |
| `cnl verify --repo <KEY> [--repo <KEY> ...] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>] [--dry-run]` | Kiểm tra các thành phần đã chọn. Interface dùng đầu vào từ ứng viên. Embedded chạy kiểm thử RTL với bộ công cụ đã ghim. |

Với Embedded, biến `CIRNET_FPGA_TOOLCHAIN_ROOT` cung cấp đường dẫn khi thiếu `--fpga-toolchain`. Đặt `CNL_VERIFICATION_PYTHON` để chọn executable Python cho các kiểm tra Simulator và Embedded. Cờ `--dry-run` chỉ hoạt động ở các lệnh đã liệt kê. Dùng `cnl doctor` để kiểm tra riêng các điều kiện cần thiết.

### Trình quản lý repo

Danh bạ của workspace nằm trong thư mục trạng thái ngoài Git. Chọn repo bằng `owner/name` trên GitHub, bí danh, đường dẫn cục bộ hoặc định danh như `r:12`. Các khóa component native hiện có vẫn dùng được. `cnl build --all` tiếp tục dùng đồ thị phụ thuộc bảy component.

Tính năng khám phá GitHub dùng `gh` đã đăng nhập vào `github.com`. CLI đọc tất cả các trang repo mà tài khoản có quyền xem và lưu thời điểm làm mới danh mục theo workspace. Dùng `--refresh` để làm mới hoặc `--offline` để đọc cache ngoại tuyến. Clone và fetch dùng thông tin xác thực Git có quyền truy cập repo đã chọn.

Khi mở, Trình quản lý repo làm mới danh mục GitHub ở chế độ nền nếu chưa có danh mục hoặc cache đã quá một ngày. Nhấn F5 để làm mới thủ công, hoặc mở bằng `cnl repos --offline` để chỉ dùng cache. Bộ đếm hiển thị số repo riêng biệt của tổ chức đã xác minh cục bộ trên tổng số repo GitHub có thể truy cập. Công cụ dùng ID repo GitHub để xác minh các đường dẫn chuyển hướng sau khi repo đổi tên. Giao diện phân biệt dữ liệu vừa tải, từ cache và đang tải, đồng thời hiển thị `?/?` khi chưa xác định được tổng số. Tìm kiếm, lọc nhóm, chọn repo và worktree bổ sung không ảnh hưởng đến bộ đếm.

```powershell
cnl repos discover --refresh
cnl repos add --all --root . --dry-run
cnl repos add --select ./Branch/my-local-repo ./cirnet-cli
cnl repos group create tools
cnl repos group add tools my-local-repo
cnl repos switch my-local-repo
cnl repos update --select my-local-repo cli --jobs 2 --dry-run
```

Dùng `--yes` để xác nhận clone, thêm hoặc gỡ hàng loạt sau khi xem trước. CLI quét thư mục đã chọn và tối đa hai cấp thư mục con. Update yêu cầu nhánh sạch có upstream và chỉ cập nhật fast-forward. Kết quả được báo theo từng repo. Khi chạy không tương tác, cần chỉ định phạm vi update.

| Lệnh | Hành vi |
| --- | --- |
| `cnl repos config <REPO>` | Xem cấu hình. Dùng `--alias`, `--clear-alias`, `--name`, `--path`, `--remote`, `--preferred-branch` hoặc `--shell` để sửa. |
| `cnl repos group list` · `cnl repos group create <GROUP>` | Liệt kê hoặc tạo nhóm. |
| `cnl repos group add <GROUP> <REPO>...` · `cnl repos group remove <GROUP> [REPO ...]` | Quản lý thành viên. Bỏ danh sách repo trong lệnh gỡ để gỡ cả nhóm. |
| `cnl repos switch <REPO>` · `cnl repos terminal <REPO> [--shell NAME]` | Đặt ngữ cảnh làm việc hoặc mở terminal cho repo. |
| `cnl repos diff <REPO> [--staged]` · `cnl repos log <REPO>` | Xem thay đổi hoặc lịch sử gần đây. |
| `cnl repos branches <REPO>` · `cnl repos worktrees <REPO>` · `cnl repos remotes <REPO>` | Xem nhánh cục bộ, worktree và remote. |
| `cnl repos branch <REPO> <BRANCH> [--switch]` · `cnl repos worktree <REPO> <PATH> --branch <BRANCH>` | Tạo hoặc chuyển nhánh, hoặc thêm worktree cho nhánh có sẵn. |
| `cnl repos stage <REPO> <PATH>...` · `cnl repos commit <REPO> -m <MESSAGE>` · `cnl repos push <REPO>` | Stage đường dẫn chỉ định, commit phần đã stage hoặc push nhánh theo dõi. Bạn phải gọi rõ từng lệnh. |

Bảng quản lý hiển thị nhánh, trạng thái Git, đường dẫn cục bộ và mã commit rút gọn. Mở Chi tiết để xem số commit đi trước và đi sau, hoặc mở Lịch sử để xem nhật ký commit. Dùng `/` để tìm, Space để chọn và `G` để lọc nhóm. Mở Chi tiết để xem đường dẫn đầy đủ hoặc nhấn `C` để sao chép đường dẫn của repo đang chọn. Dùng `A` để mở thao tác, kiểm tra danh sách rồi nhấn `Y` để xác nhận. Esc quay về màn hình trước.

Nhấn `:` để mở ô lệnh CNL cho checkout đang có tiêu điểm. Gợi ý và lệnh native dùng cùng thư mục checkout và shell ưu tiên của repo. Nhấn `T` để mở terminal của repo đang có tiêu điểm.

Dùng thanh công cụ hoặc nhấn F1 để mở terminal. Bộ chọn tìm trong các kho mã cục bộ, ưu tiên kho vừa dùng trong lần chạy CNL hiện tại và liệt kê các shell đã cài. Bảng Phiên hiển thị shell và repo trên cùng một dòng, với biểu tượng terminal thẳng cột giữa các nhóm Split và phiên độc lập. Rê chuột lên dòng để hiện nút Chia terminal và Đóng terminal. Nút Chia dùng cho phiên độc lập. Đóng phiên đang chạy cần xác nhận. Nhấn F4 để lấy tiêu điểm hoặc ẩn bảng, / để lọc, ↑ hoặc ↓ để di chuyển và Enter để lấy tiêu điểm phiên. F6 và F7 chuyển phiên trực tiếp.

F3 chia pane hoặc quay về một pane. Terminal mới và Nhân bản mở chế độ xem riêng. Các nhóm split cũ vẫn hiển thị liền nhau trong bảng Phiên. Chọn một thành viên để khôi phục nhóm và lấy tiêu điểm pane tương ứng. Kéo vạch chia hoặc dùng Ctrl+Shift+Trái và Ctrl+Shift+Phải để đổi tỷ lệ pane đầu. Ctrl+Shift+P chuyển tiêu điểm sang pane còn lại, Ctrl+Shift+M phóng to hoặc khôi phục pane. Hồ sơ lưu chế độ hiển thị bảng Phiên và tỷ lệ chia.

Nhấn F5, dùng nút trên tiêu đề pane hoặc bấm chuột phải vào phiên để mở **Thao tác**. Menu liệt kê thao tác khả dụng cùng phím tắt, gồm Đổi tên, Nhân bản, Di chuyển, Khởi động lại và Đóng. F10 quay về Trình quản lý repo và giữ các phiên đang mở. Nhân bản mở shell mới tại cùng thư mục ban đầu.

Ctrl+Shift+F tìm trong đầu ra còn lưu của pane hiện tại. Enter hoặc F3 chuyển đến kết quả sau, Shift+Enter hoặc Shift+F3 chuyển về kết quả trước và Esc trở lại shell. Kéo chuột để chọn văn bản đang hiển thị. Giữ Shift khi kéo nếu ứng dụng terminal đang bắt sự kiện chuột. Sao chép bằng Ctrl+Shift+C hoặc mục **Sao chép vùng chọn** trong Thao tác. Ctrl+C vẫn gửi tín hiệu ngắt tới shell. Tìm kiếm dùng phần đầu ra mà bộ phân tích terminal còn giữ ở độ rộng hiện tại của pane.

### Tùy chọn khởi động cho `run` và `dev`

| Tùy chọn | Công dụng |
| --- | --- |
| `--desktop interface`, `--open` | Mở Interface trong trình duyệt mặc định khi kiểm tra sẵn sàng thành công. |
| `--mode development\|packaged` | Chọn Vite hoặc Interface đóng gói. Lệnh `dev` chỉ nhận chế độ phát triển. |
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
cnl run --desktop interface --mode packaged --initialize-db
cnl status
cnl stop

cnl run --desktop interface
cnl status
cnl stop

cnl setup --browser
cnl verify
```

Dùng `--initialize-db` ở lần chạy đầu tiên của mỗi hồ sơ mới. `build --all` chỉ cập nhật receipt đã chọn sau khi kiểm chứng ứng viên mới. Thông tin lựa chọn lưu workspace, thư mục trạng thái, hồ sơ và mã băm manifest. Trường `receipt` trong báo cáo bản dựng là đường dẫn truyền vào `--receipt`. Cờ này chọn ứng viên cho lần gọi hiện tại và giữ nguyên lựa chọn đã lưu. Nếu receipt thiếu hoặc không hợp lệ, build lại trong cùng phạm vi hoặc chọn receipt đã kiểm chứng khác. Nếu đường dẫn bản dựng trên Windows quá dài, chọn thư mục ngắn bên ngoài repo bằng `--state-dir`.

Nếu Windows khởi động lại trong lúc chạy phiên phát triển mô phỏng, hãy chạy `cnl stop --desktop interface` với cùng hồ sơ. Stop và Run có thể xóa bản ghi CLI cũ khi các tiến trình thuộc phiên đã thoát, khóa dữ liệu native và các cổng của phiên đã được giải phóng, đồng thời launcher đã ghi nhận phiên đó dừng an toàn. Nếu lần khởi động lại làm launcher chưa kịp ghi kết quả, CLI cần metadata khớp với phiên, các tệp ứng viên đúng mã băm đã lưu và bằng chứng Windows khởi động sau thời điểm bắt đầu phiên. Nếu Stop vẫn báo lỗi, hãy giữ hồ sơ runtime và kiểm tra `cnl status` cùng `cnl logs --service native`.

Ví dụ xem trước và kiểm tra theo phạm vi

```powershell
cnl env set --port 8001 --hardware-port 50052 --dev-port 5174 --dry-run
cnl repos status --json
cnl build --repo remote --dry-run
cnl verify --repo cli
```

## Terminal tương tác

Dùng phím mũi tên để điều hướng, `/` để lọc menu, `:` để nhập lệnh, Tab hoặc Shift+Tab để duyệt gợi ý và `?` để xem trợ giúp. Trong ô nhập lệnh, phím lên và xuống gọi lại các lệnh đủ điều kiện. Ctrl+R tìm lệnh cũ theo phần đã nhập. Esc thoát lịch sử và khôi phục bản nháp. Page Up và Page Down cuộn đầu ra, Ctrl+F tìm nội dung, F3 chuyển đến kết quả tiếp theo. Nhập `clear` trong TUI để xóa phần kết quả đang hiển thị. Dùng `cnl stop` để dừng ứng dụng.

Rê chuột lên mục gợi ý để tô sáng. Bấm để chèn mục vào vị trí nhập và nhấn Enter để chạy lệnh. Nhấp chuột phải hoặc nhấn Shift+F10 để mở menu ngữ cảnh. Nhấn F8 để đổi giữa tiếng Anh và tiếng Việt mà vẫn giữ bản nháp và kết quả hiện tại. Bạn cũng có thể nhập `cnl --lang vi` hoặc `cnl --lang en`. Trên terminal hỗ trợ True Color, dải sáng màu đồng lướt nhẹ dọc viền ô nhập và tạm dừng khi bạn chỉnh sửa. Đặt `CNL_REDUCED_MOTION=1` để tắt hiệu ứng viền và hiệu ứng chờ, hoặc `NO_COLOR=1` để dùng chế độ đơn sắc.

Nhập lệnh native trực tiếp, ví dụ `git status` hoặc `Get-ChildItem | Select-Object -First 5` trên Windows. Mỗi lệnh chạy trong tiến trình riêng. Dùng `cnl shell` nếu cần giữ thay đổi thư mục làm việc hoặc biến môi trường qua nhiều lệnh native. Nhập `exit` trong shell để trở lại TUI.

Bảng gợi ý kết hợp cú pháp CNL với dữ liệu tra cứu nền có giới hạn. Git gợi ý lệnh và tùy chọn tích hợp trong bản đã cài, nhánh cục bộ, nhánh theo dõi từ xa, tag, remote, stash, worktree, khóa cấu hình và đường dẫn tùy theo đối số. PowerShell gợi ý cmdlet, bí danh, tham số và giá trị liệt kê từ nguồn tin cậy. CMD và Bash kết hợp lệnh tích hợp, tệp thực thi tìm thấy và các tùy chọn phổ biến. Dùng phím mũi tên hoặc Tab để duyệt thêm gợi ý ngoài vùng hiển thị.

Hồ sơ của repo đang hoạt động cung cấp shell và thư mục làm việc. Windows mặc định dùng PowerShell 7 và có thể chuyển sang CMD nếu cần. Linux mặc định dùng Bash. Dùng `cmd /c` hoặc `wsl --exec` để đổi cú pháp gợi ý cho một lệnh. Ô lệnh CNL gợi ý bản phân phối WSL đã đăng ký và cú pháp Linux phổ biến. Trong Terminal WSL đang chạy, Bash tra cứu lệnh, đường dẫn và người dùng của bản phân phối đó.

Dùng `cnl shell git-bash` để mở Git Bash trên Windows, `cnl shell bash` trên Linux, hoặc `cnl shell "wsl:Ubuntu 24.04"` để mở bản phân phối WSL đã cài. Hồ sơ `wsl` mở bản phân phối mặc định. Dùng cùng tên hồ sơ với `cnl repos terminal <REPO> --shell PROFILE` hoặc lưu vào cấu hình bằng `cnl repos config <REPO> --shell PROFILE`. Git Bash dùng môi trường MSYS. Mỗi Terminal tích hợp giữ shell riêng và xử lý Tab bằng chính shell đó, kể cả sau khi chia hoặc nhân bản phiên.

Để dùng lại lịch sử lệnh CNL đủ điều kiện sau khi khởi động lại, chạy `cnl tui --persist-history`. Khi thoát terminal, CLI lưu tối đa 100 lệnh đủ điều kiện trong `tui-history.json` của hồ sơ đang chọn. Lệnh native an toàn có thể gọi lại trong phiên. Lịch sử lưu trên đĩa loại bỏ đối số tùy ý, đường dẫn và thông tin xác thực.

## Gợi ý lệnh trên shell

PowerShell 7

```powershell
cnl completion powershell > cnl-completion.ps1
. ./cnl-completion.ps1
```

Bash

```bash
cnl completion bash > cnl-completion.bash
source ./cnl-completion.bash
```

## Lưu trữ phát triển cục bộ

Đặt `CIRNET_LOCAL_STORAGE_ROOT` trong tiến trình gọi lệnh. CLI dùng bộ cấp phát Contracts của workspace cho đầu ra phát triển thành phần. Giữ `CIRNET_ARTIFACTS_SESSION` khi lặp lệnh trên cùng worktree. Đặt `CARGO_TARGET_DIR` trong phiên đó nếu cần ghi đè. Lắp ráp hệ thống dùng namespace Remote. Lựa chọn candidate và trạng thái runtime giữ cơ chế sở hữu hiện có.

Từ shell MSVC x64, dùng `cnl build --all --reuse-candidate --receipt <ASSEMBLY_RECEIPT> --verification-evidence <RESULT> --verification-sha256 <HASH>` để dùng lại candidate đã review. Lệnh kiểm tra nguồn hiện tại, manifest và bằng chứng trình duyệt đóng gói trước khi cập nhật con trỏ profile. `cnl build --all` yêu cầu candidate mới. Khi kế thừa phiên build thành phần, mọi repo được chọn phải thuộc ngữ cảnh đã xác minh của phiên. Các lệnh build và test CLI liên quan lấy khóa độc quyền trên phiên đó.
