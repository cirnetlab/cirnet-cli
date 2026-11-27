# CIRNETLAB CLI version 0 changelog · Nhật ký thay đổi phiên bản 0

[Changelog index · Chỉ mục](README.md)

## Unreleased

### Changed

- Consolidated English and Vietnamese development history in this version file.
- Hợp nhất lịch sử phát triển tiếng Anh và tiếng Việt trong tệp theo phiên bản.

## v0.3.0-dev.1 — Development

This is a development version used in the Windows x64 Native Candidate workflow. The available Git tags and GitHub Releases provide no publication date for this version.

Đây là phiên bản phát triển dùng trong quy trình Windows x64 Native Candidate. Các Git tag và GitHub Release hiện có không cung cấp ngày phát hành cho phiên bản này.

### Added

- Added the CIRNETLAB terminal and Windows x64 toolchain, repository, candidate build, runtime and verification commands.
  - VI · Thêm terminal CIRNETLAB và các lệnh kiểm tra công cụ, quản lý repo, build ứng viên, vận hành và kiểm chứng trên Windows x64.
- Reused the Remote launcher to supervise the native stack. Added an owned Vite process for live browser development.
  - VI · Dùng launcher Remote để giám sát hệ thống native. Bổ sung tiến trình Vite do CLI quản lý để phát triển giao diện.
- Added English and Vietnamese CNL and Cargo command references.
  - VI · Thêm tham chiếu lệnh CNL và Cargo bằng tiếng Anh và tiếng Việt.
- Add contextual shell and mouse completion, native shell delegation, localized command help and results, and refresh the terminal wordmark, context menu and activity feedback.
  - VI · Bổ sung gợi ý theo ngữ cảnh trên shell và bằng chuột, hỗ trợ shell native, bản địa hóa trợ giúp và kết quả, đồng thời cập nhật chữ nổi, menu ngữ cảnh và hiệu ứng chờ.
- Share bounded command history across views and let users save eligible CNL history for a profile.
  - VI · Dùng chung lịch sử lệnh có giới hạn giữa các màn hình và cho phép lưu lệnh CNL đủ điều kiện vào hồ sơ.
- Add local Git and native shell suggestions, responsive layouts and a reduced-motion-aware input border.
  - VI · Bổ sung gợi ý Git và shell native từ dữ liệu cục bộ, bố cục linh hoạt và hiệu ứng viền ô nhập có thể tắt.
- Add a workspace repository registry, cached GitHub discovery, parallel Git synchronization, and a dashboard with persistent repository terminals.
  - VI · Thêm danh bạ repo theo workspace, khám phá GitHub qua cache, đồng bộ Git song song và bảng quản lý có terminal riêng cho từng repo.
- Refine the terminal toolbar and session navigator. Add adjustable splits, pane maximization, scrollback search, and visible text selection.
  - VI · Tinh chỉnh thanh công cụ và danh sách phiên. Bổ sung chỉnh tỷ lệ chia, phóng to pane, tìm trong đầu ra đã lưu và chọn văn bản đang hiển thị.
- Detect native Bash, Git Bash and selectable WSL Bash profiles while preserving native terminal completion.
  - VI · Phát hiện Bash native, Git Bash và hồ sơ WSL Bash theo từng bản phân phối, đồng thời giữ gợi ý Tab của shell.
- Use checkout and shell context in the CNL palette with bounded local Git, PowerShell, CMD, Bash and WSL metadata.
  - VI · Dùng ngữ cảnh checkout và shell trong bảng gợi ý CNL, với dữ liệu cục bộ có giới hạn cho Git, PowerShell, CMD, Bash và WSL.

### Changed

- Adapted the CNR terminal menu, orange wordmark and incremental painter. Added localized editing, command completion and bounded live output.
  - VI · Chuyển thể menu CNR, chữ nổi màu cam và bộ vẽ cập nhật từng vùng. Bổ sung trình nhập song ngữ, gợi ý lệnh và kết quả trực tiếp có giới hạn.
- Made workspace discovery follow the executable and added `--desktop interface` for verified native and Vite starts.
  - VI · Tìm workspace từ đường dẫn executable và thêm `--desktop interface` để mở giao diện sau khi kiểm chứng native hoặc Vite.
- Run build and verification subprocesses in their owned directories. Bind candidate selection to its workspace and profile.
  - VI · Chạy tiến trình build và kiểm chứng trong thư mục phù hợp. Gắn lựa chọn ứng viên với workspace và hồ sơ.
- Reset menu drafts between commands, switch TUI language in place and preserve subprocess output streams.
  - VI · Xóa nội dung nhập tạm khi đổi lệnh, đổi ngôn ngữ ngay trong TUI và giữ riêng các luồng đầu ra.
- Default to development mode for `run`, retain `dev` compatibility and add target-aware Stop, runtime logs and multi-repository scopes.
  - VI · Dùng chế độ phát triển làm mặc định cho `run`, giữ tương thích với `dev`, bổ sung Stop theo đích, nhật ký runtime và thao tác trên nhiều repo.
- Localize nested reports and switch the TUI language with F8 while editing or reading output.
  - VI · Bản địa hóa báo cáo lồng nhau và hỗ trợ phím F8 để đổi ngôn ngữ TUI khi đang nhập hoặc đọc kết quả.
- Soften the copper input border sweep and pause its motion while editing.
  - VI · Làm dịu dải sáng màu đồng trên viền ô nhập và tạm dừng hiệu ứng khi chỉnh sửa.
- Show verified Git metadata and readable paths in Repository Manager. Add hierarchical navigation, shared CNL command entry, and a split terminal workspace with a session list.
  - VI · Hiển thị dữ liệu Git đã kiểm chứng và đường dẫn dễ đọc trong trình quản lý repo. Thêm điều hướng theo cấp, ô lệnh CNL dùng chung và terminal chia khung với danh sách phiên.
- Widen Local path by removing the ahead and behind column from the repository table.
  - VI · Mở rộng cột đường dẫn cục bộ bằng cách bỏ cột đi trước và đi sau khỏi bảng quản lý repo.
- Tighten the Git status and Branch spacing, widen Local path, and remove the path tooltip and horizontal scrolling from the repository table.
  - VI · Thu gọn khoảng cách giữa các cột để mở rộng đường dẫn cục bộ, đồng thời bỏ tooltip đường dẫn và cuộn ngang khỏi bảng quản lý repo.
- Remove the F10 Back button from the terminal toolbar and trim the footer shortcuts.
  - VI · Bỏ nút F10 Back khỏi thanh công cụ Terminal và rút gọn gợi ý phím tắt ở thanh dưới.
- Show standard Windows paths in repository reports and open PowerShell in the equivalent Windows directory.
  - VI · Hiển thị đường dẫn Windows thông thường trong báo cáo kho mã và mở PowerShell tại thư mục Windows tương đương.
- Render Terminal sidebar session names, shell status, and filter results on the sidebar background.
  - VI · Hiển thị tên phiên, trạng thái shell và kết quả lọc trên nền bảng Phiên của Terminal.
- Remove the two-line session details footer from the Terminal sidebar.
  - VI · Bỏ hai dòng chi tiết phiên ở đáy sidebar Terminal.
- Show a terminal icon, shell, and repository on each sidebar row, with split and close actions on hover.
  - VI · Hiển thị biểu tượng terminal, shell và repo trên mỗi dòng sidebar, kèm nút chia và đóng khi rê chuột.
- Use a shell prompt and two pane outlines for the terminal icons.
  - VI · Dùng biểu tượng dấu nhắc cho shell và hai khung cho thao tác chia pane.
- Group split terminal panes in the sidebar and show their relationship with tree connectors.
  - VI · Gom các pane thuộc cùng một split trong sidebar Terminal và dùng ký tự cây để thể hiện quan hệ giữa chúng.
- Align sidebar terminal icons and show the Split icon when hovering over a session. The toolbar uses text labels for Split and Unsplit.
  - VI · Căn thẳng cột biểu tượng terminal và hiện biểu tượng Chia khi rê chuột qua phiên. Thanh công cụ dùng nhãn chữ cho Chia ô và Bỏ chia.
- Routed component development output through Contracts, guarded shared build caches and added explicit reuse of verified native candidates.
  - VI · Định tuyến đầu ra phát triển thành phần qua Contracts, bảo vệ cache dùng chung và thêm lựa chọn dùng lại ứng viên native đã kiểm chứng.

### Fixed

- Refresh the Repository Manager's GitHub catalog in the background and show distinct local organization checkouts over the accessible remote total.
  - VI · Làm mới danh mục GitHub của Trình quản lý repo ở chế độ nền và hiển thị số repo riêng biệt đã checkout cục bộ trên tổng số repo có thể truy cập.
- Preserve split terminal groups when opening or selecting an independent session.
  - VI · Giữ nhóm terminal đã chia khi mở hoặc chọn một phiên riêng.
- Recover the same virtual runtime profile after a confirmed native shutdown or Windows restart, with checks for active processes, native locks and occupied ports.
  - VI · Khôi phục cùng hồ sơ runtime mô phỏng sau khi xác nhận phiên native đã dừng hoặc Windows đã khởi động lại, đồng thời kiểm tra tiến trình còn chạy, khóa native và cổng đang sử dụng.

### Sources · Nguồn đối chiếu

- [Version manifest · Manifest phiên bản](../../Cargo.toml)
- [Native implementation baseline · Mốc triển khai native](https://github.com/cirnetlab/cirnet-cli/commit/ffc482c)
- [Latest source milestone recorded here · Mốc mã nguồn mới nhất ghi ở đây](https://github.com/cirnetlab/cirnet-cli/commit/7635722682c45b0182b1c3299eb4283e3b9ad635)
