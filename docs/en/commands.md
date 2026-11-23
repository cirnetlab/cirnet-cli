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
| `--lang en\|vi` | Set the interactive terminal language. Default `en`. |
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
| `cnl repos clone <KEY>... [--dry-run]` | Clone the named repositories into missing destinations. Requires Git access to the selected repositories. |
| `cnl repos update <KEY>... [--dry-run]` | Fetch and fast-forward the named checkouts. Each checkout must have a clean working tree and track the expected CIRNETLAB origin. |
| `cnl build --all [--offline] [--dry-run]` | Assemble the Windows x64 candidate through Remote and save its receipt for the profile. `--offline` uses cached dependencies. |
| `cnl build --repo <KEY> [--repo <KEY> ...] [--offline] [--dry-run] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>]` | Build selected components in dependency order. `--all` and `--repo` are mutually exclusive. Interface and Embedded use inputs from an existing candidate. |
| `cnl run [RUNTIME OPTIONS]` | Start the native stack and serve the packaged Interface. |
| `cnl dev [RUNTIME OPTIONS]` | Start the native services and serve the working Interface checkout through Vite with hot reload. |
| `cnl status` | Report the selected profile's runtime status, owned processes, actual ports and Interface URL when running. |
| `cnl stop [--dry-run]` | Ask the supervisor to stop the application owned by the selected profile. |
| `cnl verify [--receipt <RECEIPT>] [--browser-cache <ABSOLUTE_DIR>] [--dry-run]` | Verify the packaged candidate. Uses the profile's installed Chromium if available. Supply a compatible cache explicitly to include browser coverage. |
| `cnl verify --repo <KEY> [--repo <KEY> ...] [--receipt <RECEIPT>] [--fpga-toolchain <DIR>] [--dry-run]` | Run selected component checks. Interface tests use staged candidate inputs. Embedded runs its RTL checks with a pinned toolchain. |

For Embedded, `CIRNET_FPGA_TOOLCHAIN_ROOT` is the fallback for `--fpga-toolchain`. Set `CNL_VERIFICATION_PYTHON` to select the Python executable for Simulator and Embedded checks. `--dry-run` is command-specific. Use `cnl doctor` to check prerequisites separately.

### Runtime options for `run` and `dev`

| Option | Purpose |
| --- | --- |
| `--desktop interface`, `--open` | Open the Interface in the default browser after readiness checks. |
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
cnl run --desktop interface --initialize-db
cnl status
cnl stop

cnl dev --desktop interface
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

Use the arrows to navigate, `/` to filter the menu, `:` to enter a command, Tab or Shift+Tab to cycle completions, and `?` to show help. Ctrl+R searches command history. Page Up and Page Down scroll output, Ctrl+F searches it, and F3 advances to the next match. Type `clear` in the TUI to clear the output view. Esc goes back or cancels a cancellable command. Use `cnl stop` for runtime shutdown. Set `NO_COLOR=1` before starting the TUI for monochrome output.
