# CIRNETLAB native development

[Tiếng Việt](../vi/index.md) · [CNL and Cargo command reference](commands.md)

`cnl` builds and operates the Windows x64 development stack. Remote owns the exact system selection, manifests, and the Hardware and Simulator shutdown sequence. The CLI uses the existing Interface and exposes the [CIRNETLAB target architecture](https://github.com/cirnetlab/cirnet-remote/blob/master/docs/cirnetlab-architecture-en.md).

## Build and run

Place the seven checkouts next to one another. Use Windows x64, PowerShell 7.4+, the Visual Studio 2022 x64 C++ tools and Windows SDK, CMake, Git, Rust 1.99.1, and Node 26 with npm. From the CLI checkout, build with Cargo and read the executable path from its output. This honors Cargo's target-directory configuration. Add `--release` to the same command for an optimized build.

```powershell
$cnl = cargo build --locked --bin cnl --message-format=json |
    ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'cnl' -and $_.executable } |
    Select-Object -ExpandProperty executable -Last 1
if ($LASTEXITCODE -ne 0 -or !$cnl) { throw 'The cnl build did not complete.' }

& $cnl doctor
& $cnl setup
& $cnl build --all
& $cnl run --desktop interface --mode packaged --initialize-db
& $cnl status
& $cnl stop
```

Cargo builds the CLI. `cnl build --all` builds the native candidate and selects its receipt for the current profile. Reuse a verified candidate with `--receipt <absolute-path-to-assembly-result.json>`. The initial `--initialize-db` authorizes migration of a fresh SQLite database. Existing state passes the native launcher's schema check. The development profile uses virtual Hardware and Simulator. For a separately configured PostgreSQL service, see [Remote's deployment guides](https://github.com/cirnetlab/cirnet-remote/tree/master/docs/en/operations).

The CLI searches the working directory and then the executable's ancestors. For an executable installed outside the workspace, pass `--workspace <directory>` or set `CNL_WORKSPACE`. Artifacts and SQLite state default to a per-workspace directory under `%LOCALAPPDATA%\CIRNETLAB`. Use `--state-dir <absolute-external-directory>` and `--profile <name>` for isolated state. Use a short external path if MSVC cannot open a deeply nested build output. Use `cnl env set --port 8000 --hardware-port 50051 --dev-port 5173` to change the profile ports.

## Interactive terminal

Run `cnl` or `cnl tui` in a terminal. Use `--lang vi` for Vietnamese or set `NO_COLOR=1` for monochrome. Use the arrow keys to select a command, `/` to search, `:` to enter a command, and Tab to cycle completions. Use Ctrl+R for history, Page Up and Page Down to scroll results, and Ctrl+F to find text in a result. The editor supports Unicode selection, undo, redo and the Windows clipboard. Esc cancels a cancellable command. Use `cnl stop` for native and Vite services.

## Develop and verify

In the terminal, selecting a menu entry opens a fresh prompt with its base command. Recommended actions appear in the completion list. Press F8 to switch between English and Vietnamese while editing. See the [command reference](commands.md#interactive-terminal) for history and native completion. The terminal adapts the [CNR menu](https://github.com/cirnetlab/cirnet-rules/tree/646afa38516bbfd276b0aecb7a138ab427df5fa9/src/cli/menu) under GPL-3.0.

Start with `cnl run --desktop interface --initialize-db` to serve the live Interface checkout through Vite, with its existing API and WebSocket proxies. Choose a different development port when the default is occupied. The CLI checks HTTP, the served Interface and the protected WebSocket route before opening the browser. The native launcher trusts the selected Vite loopback origin. When the candidate changes, the CLI preserves the previous verified input cache under the ignored `.runtime/` directory. Unrecognized cache contents require inspection.

Run `cnl setup --browser` to install the selected candidate's Chromium locally. Run `cnl verify` for the packaged installation, API, persistence, safe Stop and browser checks, or supply `--browser-cache <path>` to use a preinstalled matching Chromium. Without a browser cache, verify reports its reduced coverage. `cnl run`, `cnl dev`, and `cnl verify` accept `--receipt <path-to-assembly-result.json>` to select a previously built candidate.

Use `cnl build --repo <key>` and `cnl verify --repo <key>` for scoped checks. Keys are `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface`, and `embedded`. The Interface needs the candidate's generated SDK and Remote OpenAPI. Embedded verification requires the pinned Icarus toolchain and Python verification environment described in Embedded Hardware, supplied via `--fpga-toolchain` and optionally `CNL_VERIFICATION_PYTHON`. Embedded runs separately from the native application.

For repository tasks, use `cnl repos discover` or `cnl repos status`. Clone and update require named repositories, and updates require a clean branch tracking its expected CIRNETLAB origin. Pass `--json` for structured reports.

Current development version: `0.3.0-dev.1`. [Changelog](../changelogs/README.md)
