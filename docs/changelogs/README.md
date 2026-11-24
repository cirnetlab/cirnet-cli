# CIRNETLAB CLI changelog

## 0.3.0-dev.1

### English

- Added the CIRNETLAB terminal and Windows x64 toolchain, repository, candidate build, runtime and verification commands.
- Reused the Remote launcher to supervise the native stack. Added an owned Vite process for live browser development.
- Adapted the CNR terminal menu, orange wordmark and incremental painter. Added localized editing, command completion and bounded live output.
- Made workspace discovery follow the executable and added `--desktop interface` for verified native and Vite starts.
- Added English and Vietnamese CNL and Cargo command references.
- Run build and verification subprocesses in their owned directories. Bind candidate selection to its workspace and profile.
- Reset menu drafts between commands, switch TUI language in place and preserve subprocess output streams.
- Default to development mode for `run`, retain `dev` compatibility and add target-aware Stop, runtime logs and multi-repository scopes.
- Add contextual shell and mouse completion, native shell delegation, localized command help and results, and refresh the terminal wordmark, context menu and activity feedback.
- Localize nested reports and switch the TUI language with F8 while editing or reading output.
- Share bounded command history across views and let users save eligible CNL history for a profile.
- Add local Git and native shell suggestions, responsive layouts and a reduced-motion-aware input border.
- Soften the copper input border sweep and pause its motion while editing.
- Add a workspace repository registry, cached GitHub discovery, parallel Git synchronization, and a dashboard with persistent repository terminals.
- Show verified Git metadata and readable paths in Repository Manager. Add hierarchical navigation, shared CNL command entry, and a split terminal workspace with a session list.
- Widen Local path by removing the ahead and behind column from the repository table.
- Tighten the Git status and Branch spacing, widen Local path, and remove the path tooltip and horizontal scrolling from the repository table.
- Remove the F10 Back button from the terminal toolbar and trim the footer shortcuts.
- Show standard Windows paths in repository reports and open PowerShell in the equivalent Windows directory.
- Refine the terminal toolbar and session navigator. Add adjustable splits, pane maximization, scrollback search, and visible text selection.
- Render Terminal sidebar session names, shell status, and filter results on the sidebar background.
- Remove the two-line session details footer from the Terminal sidebar.

### Tiếng Việt

- Thêm terminal CIRNETLAB và các lệnh kiểm tra công cụ, quản lý repo, build ứng viên, vận hành và kiểm chứng trên Windows x64.
- Dùng launcher Remote để giám sát hệ thống native. Bổ sung tiến trình Vite do CLI quản lý để phát triển giao diện.
- Chuyển thể menu CNR, chữ nổi màu cam và bộ vẽ cập nhật từng vùng. Bổ sung trình nhập song ngữ, gợi ý lệnh và kết quả trực tiếp có giới hạn.
- Tìm workspace từ đường dẫn executable và thêm `--desktop interface` để mở giao diện sau khi kiểm chứng native hoặc Vite.
- Thêm tham chiếu lệnh CNL và Cargo bằng tiếng Anh và tiếng Việt.
- Chạy tiến trình build và kiểm chứng trong thư mục phù hợp. Gắn lựa chọn ứng viên với workspace và hồ sơ.
- Xóa nội dung nhập tạm khi đổi lệnh, đổi ngôn ngữ ngay trong TUI và giữ riêng các luồng đầu ra.
- Dùng chế độ phát triển làm mặc định cho `run`, giữ tương thích với `dev`, bổ sung Stop theo đích, nhật ký runtime và thao tác trên nhiều repo.
- Bổ sung gợi ý theo ngữ cảnh trên shell và bằng chuột, hỗ trợ shell native, bản địa hóa trợ giúp và kết quả, đồng thời cập nhật chữ nổi, menu ngữ cảnh và hiệu ứng chờ.
- Bản địa hóa báo cáo lồng nhau và hỗ trợ phím F8 để đổi ngôn ngữ TUI khi đang nhập hoặc đọc kết quả.
- Dùng chung lịch sử lệnh có giới hạn giữa các màn hình và cho phép lưu lệnh CNL đủ điều kiện vào hồ sơ.
- Bổ sung gợi ý Git và shell native từ dữ liệu cục bộ, bố cục linh hoạt và hiệu ứng viền ô nhập có thể tắt.
- Làm dịu dải sáng màu đồng trên viền ô nhập và tạm dừng hiệu ứng khi chỉnh sửa.
- Thêm danh bạ repo theo workspace, khám phá GitHub qua cache, đồng bộ Git song song và bảng quản lý có terminal riêng cho từng repo.
- Hiển thị dữ liệu Git đã kiểm chứng và đường dẫn dễ đọc trong trình quản lý repo. Thêm điều hướng theo cấp, ô lệnh CNL dùng chung và terminal chia khung với danh sách phiên.
- Mở rộng cột đường dẫn cục bộ bằng cách bỏ cột đi trước và đi sau khỏi bảng quản lý repo.
- Thu gọn khoảng cách giữa các cột để mở rộng đường dẫn cục bộ, đồng thời bỏ tooltip đường dẫn và cuộn ngang khỏi bảng quản lý repo.
- Bỏ nút F10 Back khỏi thanh công cụ Terminal và rút gọn gợi ý phím tắt ở thanh dưới.
- Hiển thị đường dẫn Windows thông thường trong báo cáo kho mã và mở PowerShell tại thư mục Windows tương đương.
- Tinh chỉnh thanh công cụ và danh sách phiên. Bổ sung chỉnh tỷ lệ chia, phóng to pane, tìm trong đầu ra đã lưu và chọn văn bản đang hiển thị.
- Hiển thị tên phiên, trạng thái shell và kết quả lọc trên nền bảng Phiên của Terminal.
- Bỏ hai dòng chi tiết phiên ở đáy sidebar Terminal.
