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
| `cnl repos` | Open the interactive [Repository Manager](#repository-manager). |
| `cnl repos list` · `cnl repos status [REPO ...]` | List registrations or inspect selected worktrees and tracking information. |
| `cnl repos discover [--root DIR] [--refresh \| --offline] [--query TEXT]` | Find local checkouts and accessible GitHub repositories. |
| `cnl repos clone <REPO>...` · `cnl repos clone --all` | Clone into missing destinations and register verified checkouts. Bulk clone previews by default. |
| `cnl repos add <REPO-OR-PATH>...` · `cnl repos add --all --root DIR` | Register existing Git checkouts. Bulk registration previews by default. |
| `cnl repos register <REPO>` · `cnl repos remove <REPO>...` | Register a GitHub repository without cloning, or remove registry membership. |
| `cnl repos update` · `cnl repos update <REPO>...` · `cnl repos update --all` | Open the interactive selector, or update the selected checkouts with fast-forward only. |
| `cnl repos fetch <REPO>...` · `cnl repos fetch --all` | Fetch configured remotes. Fetch and update accept `--jobs N` and `--dry-run`. |
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

### Repository Manager

The workspace registry lives outside Git in the state directory. Use a GitHub `owner/name`, a configured alias, a local path, or an identifier such as `r:12` to select a repository. The existing native component keys are still available. `cnl build --all` continues to use the seven-component dependency graph.

GitHub discovery uses `gh` signed in to `github.com`. It paginates repositories visible to that account and keeps a timestamped workspace cache. Use `--refresh` to update the cache or `--offline` to read it without contacting GitHub. Cloning and fetching use Git credentials with access to the selected repository.

```powershell
cnl repos discover --refresh
cnl repos add --all --root . --dry-run
cnl repos add --select ./Branch/my-local-repo ./cirnet-cli
cnl repos group create tools
cnl repos group add tools my-local-repo
cnl repos switch my-local-repo
cnl repos update --select my-local-repo cli --jobs 2 --dry-run
```

Use `--yes` to confirm bulk clone, add, or removal after checking the preview. Local discovery scans the selected root and up to two directory levels. Updates require clean, tracked branches and use fast-forward only. Results are reported per repository. Noninteractive updates require explicit scope.

| Command | Behavior |
| --- | --- |
| `cnl repos config <REPO>` | Read settings. Use `--alias`, `--clear-alias`, `--name`, `--path`, `--remote`, `--preferred-branch`, or `--shell` to edit them. |
| `cnl repos group list` · `cnl repos group create <GROUP>` | List or create groups. |
| `cnl repos group add <GROUP> <REPO>...` · `cnl repos group remove <GROUP> [REPO ...]` | Manage membership. Omit repositories from group removal to remove the group. |
| `cnl repos switch <REPO>` · `cnl repos terminal <REPO> [--shell NAME]` | Set the working context or open a repository terminal. |
| `cnl repos diff <REPO> [--staged]` · `cnl repos log <REPO>` | Read changes or recent history. |
| `cnl repos branches <REPO>` · `cnl repos worktrees <REPO>` · `cnl repos remotes <REPO>` | Inspect local branches, worktrees and remotes. |
| `cnl repos branch <REPO> <BRANCH> [--switch]` · `cnl repos worktree <REPO> <PATH> --branch <BRANCH>` | Create or switch to a branch, or add a worktree for an existing branch. |
| `cnl repos stage <REPO> <PATH>...` · `cnl repos commit <REPO> -m <MESSAGE>` · `cnl repos push <REPO>` | Stage named paths, commit staged changes, or push the tracked branch. Each command requires an explicit invocation. |

The dashboard shows the branch, Git status, local path and short commit hash. Open Details to see ahead and behind counts, or History for the commit log. Use `/` to search, Space to select, and `G` to filter groups. Open Details to see the full local path, or press `C` to copy it for the focused repository. Use `A` for actions, review the targets, and confirm changes with `Y`. Esc returns to the previous view.

Press `:` to open the shared CNL command field. It uses the same history, completion and output controls as the main terminal. Press `T` to open the focused repository terminal.

Use the integrated terminal toolbar or press F1 to open a terminal. The picker searches local repositories, prioritizes recent choices during the current CNL process, and offers installed shells. The Sessions panel shows each shell and repository on one row, with terminal icons aligned across split groups and standalone sessions. Hover a row for Split Terminal and Kill Terminal. Split is available for standalone sessions, and closing a running session asks for confirmation. Press F4 to focus or hide it, then / to filter, ↑ or ↓ to navigate, and Enter to focus a session. F6 and F7 switch sessions immediately.

F3 splits or restores a single pane. New Terminal and Duplicate open independent views. Existing split groups stay together in the Sessions panel. Select either member to restore its group and focus that pane. Drag the divider or use Ctrl+Shift+Left and Ctrl+Shift+Right to change the first pane's share. Ctrl+Shift+P focuses the other pane and Ctrl+Shift+M maximizes or restores the current pane. The profile remembers sidebar visibility and split ratio.

Open **Actions** with F5, from a pane header, or by right-clicking a session. The menu lists available operations and shortcuts, including Rename, Duplicate, Move, Restart, and Close. F10 returns to Repository Manager with the sessions still open. Duplicate starts a fresh shell in the same initial directory.

Ctrl+Shift+F searches retained output in the focused pane. Enter or F3 moves to the next result, Shift+Enter or Shift+F3 moves to the previous result, and Esc returns to the shell. Drag to select visible text. Use Shift+drag if the application uses mouse tracking. Copy with Ctrl+Shift+C or **Copy selection** in Actions. Ctrl+C remains the shell interrupt. Search uses the scrollback retained by the terminal parser at the current pane width.

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

Hover over a suggestion to highlight it. Click to insert it at the cursor and press Enter to run the command. Right-click or press Shift+F10 to open the context menu. Press F8 to switch between English and Vietnamese while keeping the active draft and output. You can also enter `cnl --lang vi` or `cnl --lang en`. On compatible True Color terminals, a soft copper highlight travels along the input border and pauses while you edit. Set `CNL_REDUCED_MOTION=1` to disable the animation and spinner, or `NO_COLOR=1` for monochrome output.

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
