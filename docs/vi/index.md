# Phát triển native CIRNETLAB

[English](../en/index.md)

`cnl` build và vận hành hệ thống phát triển Windows x64. Remote quản lý bộ đầu vào chính xác, manifest và trình tự dừng Hardware và Simulator. CLI dùng Interface hiện có và tuân theo [kiến trúc mục tiêu CIRNETLAB](https://github.com/cirnetlab/cirnet-remote-next/blob/233bd694f0c1005ad9aa7c9882d636f68a3e170d/docs/cirnetlab-architecture-vi.md).

## Build và chạy

Đặt bảy checkout cạnh nhau. Dùng Windows x64, PowerShell 7.4+, công cụ Visual Studio 2022 C++ x64 và Windows SDK, CMake, Git, Rust 1.99.1 và Node 26 có npm. Trong checkout CLI, build bằng Cargo và đọc đường dẫn executable từ đầu ra của Cargo. Cách này dùng đúng thư mục đích đã cấu hình. Thêm `--release` vào cùng lệnh để build bản tối ưu hóa.

```powershell
$cnl = cargo build --locked --bin cnl --message-format=json |
    ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'cnl' -and $_.executable } |
    Select-Object -ExpandProperty executable -Last 1
if ($LASTEXITCODE -ne 0 -or !$cnl) { throw 'Bản dựng cnl chưa hoàn tất.' }

& $cnl doctor
& $cnl setup
& $cnl build --all
& $cnl run --desktop interface --initialize-db
& $cnl status
& $cnl stop
```

Cargo build CLI. `cnl build --all` build ứng viên native và chọn receipt của ứng viên cho hồ sơ hiện tại. Dùng `--receipt <đường-dẫn-tuyệt-đối-đến-assembly-result.json>` để dùng lại ứng viên đã kiểm chứng. Cờ `--initialize-db` cho phép chạy migration để tạo cơ sở dữ liệu SQLite mới. Launcher native kiểm tra schema của dữ liệu hiện có. Hồ sơ phát triển dùng Hardware và Simulator ảo. Để cấu hình PostgreSQL riêng, xem [hướng dẫn triển khai Remote](https://github.com/cirnetlab/cirnet-remote-next/tree/master/docs/vi/operations).

CLI tìm workspace từ thư mục hiện hành, sau đó từ các thư mục cha của executable. Khi executable được cài bên ngoài workspace, dùng `--workspace <thư-mục>` hoặc đặt `CNL_WORKSPACE`. Artifact và SQLite mặc định nằm trong thư mục riêng theo workspace dưới `%LOCALAPPDATA%\CIRNETLAB`. Dùng `--state-dir <thư-mục-tuyệt-đối-nằm-ngoài-repo>` và `--profile <tên>` để chọn trạng thái cách ly. Chọn đường dẫn ngoài repo ngắn hơn nếu MSVC không mở được tệp build nằm quá sâu. Dùng `cnl env set --port 8000 --hardware-port 50051 --dev-port 5173` để đổi cổng của hồ sơ.

## Terminal tương tác

Chạy `cnl` hoặc `cnl tui` trong terminal. Dùng `--lang vi` để chọn tiếng Việt hoặc đặt `NO_COLOR=1` để tắt màu. Dùng phím mũi tên để chọn lệnh, `/` để tìm, `:` để nhập và Tab để duyệt gợi ý. Dùng Ctrl+R để xem lịch sử, Page Up và Page Down để cuộn kết quả và Ctrl+F để tìm nội dung. Trình nhập hỗ trợ chọn văn bản Unicode, hoàn tác, làm lại và clipboard Windows. Esc hủy lệnh có thể hủy. Dùng `cnl stop` để dừng dịch vụ native và Vite.

Terminal chuyển thể menu CNR từ [CIRNET Rules](https://github.com/cirnetlab/cirnet-rules/tree/646afa38516bbfd276b0aecb7a138ab427df5fa9/src/cli/menu) theo giấy phép GPL-3.0. Các tệp được tái sử dụng ghi rõ commit nguồn.

## Phát triển và kiểm chứng

Dùng `cnl dev --desktop interface --initialize-db` để phục vụ mã Interface đang làm việc qua Vite, với proxy API và WebSocket hiện có. Chọn cổng phát triển khác khi cổng mặc định đang được dùng. CLI kiểm tra HTTP, trang Interface và tuyến WebSocket có xác thực trước khi mở trình duyệt. Launcher native cho phép origin loopback của Vite theo cổng đã chọn. Khi đổi ứng viên, CLI lưu cache đã kiểm chứng trước đó trong thư mục `.runtime/` do Git bỏ qua. Cần kiểm tra các nội dung cache không nhận diện được.

Chạy `cnl setup --browser` để cài Chromium tương ứng với ứng viên đã chọn. Chạy `cnl verify` để kiểm tra bản cài, API, lưu trữ, Stop an toàn và trình duyệt. Dùng `--browser-cache <đường-dẫn>` để dùng lại Chromium phù hợp đã cài. Kết quả báo rõ phạm vi kiểm tra khi thiếu cache trình duyệt. `cnl run`, `cnl dev` và `cnl verify` nhận `--receipt <đường-dẫn-assembly-result.json>` để chọn ứng viên đã build.

Dùng `cnl build --repo <khóa>` và `cnl verify --repo <khóa>` để chạy theo phạm vi. Các khóa gồm `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface` và `embedded`. Interface cần SDK được sinh cùng OpenAPI của Remote. Kiểm chứng Embedded cần bộ Icarus đã chốt và môi trường Python mô tả trong Embedded Hardware, thông qua `--fpga-toolchain` và tùy chọn `CNL_VERIFICATION_PYTHON`. Embedded chạy riêng với ứng dụng native.

Dùng `cnl repos discover` hoặc `cnl repos status` để kiểm tra các repo. Clone và update yêu cầu chỉ định repo. Update yêu cầu nhánh sạch và trỏ tới origin CIRNETLAB tương ứng. Dùng `--json` để nhận báo cáo có cấu trúc.

Dải phát hành hiện tại là `0.3.0-dev.1`. [Changelog](../changelogs/README.md)
