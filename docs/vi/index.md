# Phát triển native CIRNETLAB

[English](../en/index.md)

`cnl` build và vận hành hệ thống phát triển Windows x64. Remote quản lý bộ đầu vào chính xác, manifest và trình tự dừng Hardware và Simulator. CLI dùng Interface hiện có và tuân theo [kiến trúc mục tiêu CIRNETLAB](https://github.com/cirnetlab/cirnet-remote-next/blob/233bd694f0c1005ad9aa7c9882d636f68a3e170d/docs/cirnetlab-architecture-vi.md).

## Build và chạy

Đặt bảy checkout cạnh nhau. Dùng PowerShell 7.4+, công cụ Visual Studio 2022 C++ x64 và Windows SDK, CMake, Git, Rust 1.99.1 và Node 26 có npm. Trong checkout CLI:

```powershell
cargo install --locked --path .
cnl doctor
cnl setup
cnl build --all
cnl run --initialize-db --open
cnl status
cnl stop
```

Dùng `--workspace <đường-dẫn>` khi chạy từ bên ngoài workspace. Đầu ra và dữ liệu SQLite mặc định nằm trong thư mục riêng theo workspace dưới `%LOCALAPPDATA%\CIRNETLAB`. Dùng `--state-dir <đường-dẫn-tuyệt-đối-nằm-ngoài-repo>` và `--profile <tên>` để chọn trạng thái cách ly. `cnl env set --port 8000 --hardware-port 50051 --dev-port 5173` lưu các cổng cho profile. Dùng `--dry-run` để xem trước thay đổi profile, build, thao tác launcher hoặc Git.

`--initialize-db` cho phép chạy migration để tạo cơ sở dữ liệu SQLite cục bộ mới. Dữ liệu hiện có phải qua kiểm tra schema của launcher. CLI dùng profile SQLite của launcher. Để cấu hình PostgreSQL riêng, xem [hướng dẫn triển khai Remote](https://github.com/cirnetlab/cirnet-remote-next/tree/master/docs/vi/operations). Profile phát triển của CLI chạy Hardware và Simulator ở chế độ ảo.

## Phát triển và kiểm chứng

Dùng `cnl dev --initialize-db --open` để phục vụ mã Interface đang làm việc qua Vite, với proxy API và WebSocket hiện có. Chọn cổng phát triển khác khi cổng mặc định đang được dùng. Launcher native cho phép origin loopback của Vite theo cổng đã chọn. Khi đổi ứng viên, CLI lưu cache đã kiểm chứng trước đó trong thư mục `.runtime/` do Git bỏ qua. Cần kiểm tra các nội dung cache không nhận diện được.

Chạy `cnl setup --browser` để cài Chromium tương ứng với ứng viên đã chọn. Chạy `cnl verify` để kiểm tra bản cài, API, lưu trữ, Stop an toàn và trình duyệt. Dùng `--browser-cache <đường-dẫn>` để dùng lại Chromium phù hợp đã cài. Kết quả báo rõ phạm vi kiểm tra khi thiếu cache trình duyệt. `cnl run`, `cnl dev` và `cnl verify` nhận `--receipt <đường-dẫn-assembly-result.json>` để chọn ứng viên đã build.

Dùng `cnl build --repo <khóa>` và `cnl verify --repo <khóa>` để chạy theo phạm vi. Các khóa gồm `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface` và `embedded`. Interface cần SDK được sinh cùng OpenAPI của Remote. Kiểm chứng Embedded cần bộ Icarus đã chốt và môi trường Python mô tả trong Embedded Hardware, thông qua `--fpga-toolchain` và tùy chọn `CNL_VERIFICATION_PYTHON`. Embedded chạy riêng với ứng dụng native.

Dùng `cnl repos discover` hoặc `cnl repos status` để kiểm tra các repo. Clone và update yêu cầu chỉ định repo. Update yêu cầu nhánh sạch và trỏ tới origin CIRNETLAB tương ứng. Dùng `--json` để nhận báo cáo có cấu trúc. Chạy `cnl` trong terminal tương tác để xem gợi ý lệnh, lịch sử và cuộn kết quả. Escape có thể hủy các lệnh build và kiểm tra chạy cách ly. Dùng `cnl stop` để dừng ứng dụng.

Dải phát hành hiện tại là `0.3.0-dev.1`. [Changelog](../changelogs/README.md)
