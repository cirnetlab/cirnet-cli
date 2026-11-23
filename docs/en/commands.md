# CIRNETLAB command reference

[Tiếng Việt](../vi/commands.md) · [Development guide](index.md) · [README](../../README.md)

## Build and check the CLI with Cargo

Run the Cargo commands from the `cirnet-cli` checkout. For the full native stack, place the seven CIRNETLAB checkouts under the same workspace root. See the development guide for toolchain requirements.

| Command | Purpose |
| --- | --- |
| `cargo --version` | Show the active Cargo version. The repository pins Rust 1.99.1 in `rust-toolchain.toml`. |
| `cargo check --locked --all-targets` | Type-check the CLI and its test targets. |
| `cargo fmt --all` | Format the Rust source. |
| `cargo fmt --all --check` | Check formatting without editing files. |
| `cargo clippy --locked --all-targets -- -D warnings` | Run Clippy and treat warnings as errors. |
| `cargo test --locked` | Run the CLI test suite. |
| `cargo test --locked --test commands` | Run only the CLI command integration tests. |
| `cargo build --locked --bin cnl` | Build the Debug executable. With the default Cargo target directory, the output is `target/debug/cnl.exe`. |
| `cargo build --locked --bins` | Build all binary targets. This repository currently has one binary, `cnl`. |
| `cargo build --locked --bin cnl --release` | Build the optimized executable at `target/release/cnl.exe` with the default target directory. |
| `cargo run --locked --bin cnl -- tui --lang en` | Build and start the TUI directly. The first `--` passes the remaining arguments to `cnl`. |
| `cargo run --locked --bin cnl -- --help` | Build and print CLI help. |
| `cargo tree --locked -e normal` | Inspect the runtime dependency tree. |

Use Cargo's `--offline` with `build`, `check`, or `test` when the required dependencies are already cached. `--locked` keeps the build on the checked-in lockfile. If Cargo uses a custom target directory, follow the [PowerShell build example](index.md#build-and-run) to get the exact executable path from Cargo.

The rest of this reference uses `cnl`. If it is not on PATH, call the executable with `& $cnl` in PowerShell, using the path reported by Cargo.

## Global options

| Option | Purpose |
| --- | --- |
| `--workspace <DIR>` | Select the parent directory of the checkouts. This overrides `CNL_WORKSPACE`. Without either, the CLI searches the ancestors of the working directory and then those of the executable. |
| `--state-dir <ABSOLUTE_DIR>` | Select an absolute directory outside Git checkouts for profiles, local SQLite databases, and build artifacts. The default is a per-workspace directory under `%LOCALAPPDATA%\CIRNETLAB`. |
| `--profile <NAME>` | Select a profile. Default `windows-x64`. Use letters, digits, hyphens or underscores. Use the same workspace, state directory and profile for all steps in one lifecycle. |
| `--json` | Write the structured report to stdout. Subprocess progress on stdout is suppressed in this mode. Subprocess diagnostics stay on stderr. |
| `--lang en\|vi` | Select the CLI and interactive terminal language. Default `en`. |
| `--persist-history` | Save eligible CNL history for the selected TUI profile. History stays in memory by default. |
| `-h`, `--help` | Show help for the selected command. |
| `-V`, `--version` | Show the CLI version at the top level. |

## Commands

`<KEY>` is one of `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface`, or `embedded`. `<RECEIPT>` is the path to a verified candidate's `assembly-result.json`. Square brackets enclose optional arguments. An ellipsis means an argument can be repeated.

| Command | Behavior |
| --- | --- |
| `cnl` · `cnl tui` | Open the interactive terminal. Use `cnl tui --lang vi` for Vietnamese. |
| `cnl help [COMMAND ...]` | List commands or show nested help, for example `cnl help repos update`. |
| `cnl doctor` | Check the Windows x64 toolchain and workspace prerequisites. Exits with an error when required checks fail. |
| `cnl env` · `cnl env show` | Show the profile, ports, state directory and database type. |
| `cnl env check` | Show the profile with a toolchain diagnostic report. |
| `cnl env set [--port N] [--hardware-port N] [--dev-port N] [--dry-run]` | Save or preview port changes for the profile. Unspecified ports retain their current values. |
| `cnl setup [--dry-run]` | Prepare the profile after checking prerequisites. |
| `cnl setup --browser [--receipt <RECEIPT>] [--dry-run]` | Install the selected candidate's Chromium in the profile. |
| `cnl repos discover` | List expected checkout locations and whether each checkout is present. |
| `cnl repos status` | Show branches, revisions, working tree changes, remotes and dependency availability. |
| `cnl repos clone <KEY>... [--dry-run]` · `cnl repos clone --all [--dry-run]` | Clone repositories into missing destinations. With `--all`, verify the origin of existing checkouts and skip them. Requires Git access to the selected repositories. |
| `cnl repos update <KEY>... [--dry-run]` · `cnl repos update --all [--dry-run]` | Fetch and fast-forward the selected checkouts. Each checkout must have a clean working tree and track the expected CIRNETLAB origin. |
| `cnl build --all [--offline] [--dry-run]` | Assemble the Windows x64 candidate through Remote and save its receipt for the profile. `--offline` uses cached dependencies. |
| `cnl build --repo <KEY> [--repo <KEY> ...] [--offline] [--dry-run] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>]` | Build selected components in dependency order. `--all` and `--repo` are mutually exclusive. Interface and Embedded use inputs from an existing candidate. |
| `cnl run [RUNTIME OPTIONS]` | Start the native stack with Vite in development mode by default. Select `--mode packaged` for the verified packaged Interface. |
| `cnl dev [RUNTIME OPTIONS]` | Compatibility command for `run --mode development`. |
| `cnl status` | Report the target, mode, owned services, actual ports and Interface URL for the selected profile. |
| `cnl stop [--desktop interface] [--dry-run]` | Ask the supervisor to stop the selected profile's owned session. An explicit target must match that session. |
| `cnl logs [--service native\|vite] [--lines N]` | Read the tail of the current or most recent runtime log in the selected profile. Defaults to 60 lines of `native`. |
| `cnl shell [powershell\|cmd\|bash\|wsl]` | Open an interactive native shell for the current working directory. Defaults to PowerShell on Windows and Bash on Linux. Windows can also select CMD or WSL. |
| `cnl completion <powershell\|bash>` | Write a shell completion script to stdout. |
| `cnl verify [--receipt <RECEIPT>] [--browser-cache <ABSOLUTE_DIR>] [--dry-run]` | Verify the packaged candidate. Uses the profile's installed Chromium if available. Supply a compatible cache explicitly to include browser coverage. |
| `cnl verify --repo <KEY> [--repo <KEY> ...] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>] [--dry-run]` | Run selected component checks. Interface tests use staged candidate inputs. Embedded runs its RTL checks with a pinned toolchain. |

For Embedded, `CIRNET_FPGA_TOOLCHAIN_ROOT` is the fallback for `--fpga-toolchain`. Set `CNL_VERIFICATION_PYTHON` to select the Python executable for Simulator and Embedded checks. `--dry-run` is command-specific. Use `cnl doctor` to check prerequisites separately.

### Runtime options for `run` and `dev`

| Option | Purpose |
| --- | --- |
| `--desktop interface`, `--open` | Open the Interface in the default browser after readiness checks. |
| `--mode development\|packaged` | Select live Vite or the packaged Interface. `dev` accepts development mode only. |
| `--initialize-db` | Allow the launcher to initialize a fresh SQLite database. Existing databases must pass schema checks. |
| `--receipt <RECEIPT>` | Start a specific verified candidate. Without it, the CLI uses the last receipt saved by `build --all` for the selected profile. |
| `--port N` · `--hardware-port N` · `--dev-port N` | Override the HTTP, Hardware RPC and Vite ports for this launch. Profile defaults are `8000`, `50051` and `5173`. Ports must be distinct and in `1024..=65535`. |
| `--dry-run` | Preview launch settings without starting services. |

### Common sequences

Run the packaged Interface first. Stop that session before using the same profile for Vite.

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

Use `--initialize-db` on the first launch of each fresh profile. A successful `build --all` replaces the selected receipt only after verifying the new candidate. The selection records the workspace, state directory, profile and manifest digest. The `receipt` field in the build report is the path to pass to `--receipt`. This option selects a candidate for the current invocation and keeps the saved selection. If the receipt is missing or invalid, build again in the same scope or use another verified receipt. For a long Windows build path, select a short external directory with `--state-dir`.

Useful previews and focused checks

```powershell
cnl env set --port 8001 --hardware-port 50052 --dev-port 5174 --dry-run
cnl repos status --json
cnl build --repo remote --dry-run
cnl verify --repo cli
```

## Interactive terminal

Use the arrows to navigate, `/` to filter the menu, `:` to enter a command, Tab or Shift+Tab to cycle completions, and `?` to show help. In the command field, Up and Down recall eligible commands. Ctrl+R finds earlier entries matching the current prefix. Esc leaves history and restores the draft. Page Up and Page Down scroll output, Ctrl+F searches it, and F3 advances to the next match. Type `clear` in the TUI to clear the output view. Use `cnl stop` for runtime shutdown.

Hover over a suggestion to highlight it. Click to insert it at the cursor and press Enter to run the command. Right-click or press Shift+F10 to open the context menu. Press F8 to switch between English and Vietnamese while keeping the active draft and output. You can also enter `cnl --lang vi` or `cnl --lang en`. The input border animates on compatible True Color terminals. Set `CNL_REDUCED_MOTION=1` to disable the animation and spinner, or `NO_COLOR=1` for monochrome output.

Enter native commands directly, such as `git status` or `Get-ChildItem | Select-Object -First 5` on Windows. Each prompt command runs in a separate process. Use `cnl shell` when you want working directory or environment changes to persist through multiple native commands. Type `exit` in that shell to return to the TUI.

The TUI suggests local Git branches, known Git and shell options, and paths where supported. On Windows it follows PowerShell by default. Use an explicit `cmd /c` or `wsl --exec` prefix for CMD or Bash command suggestions. On Linux the default shell context is Bash. Suggestions use bounded local metadata.

To reuse eligible CNL history across launches, start with `cnl tui --persist-history`. The CLI saves up to 100 eligible commands in `tui-history.json` under the selected profile when the terminal exits. Safe native commands can be recalled during the session. Arbitrary arguments, paths and credentials are excluded from stored history.

## Shell completion

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
