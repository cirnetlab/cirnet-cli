# CIRNETLAB native development

[Tiếng Việt](../vi/index.md)

`cnl` builds and operates the Windows x64 development stack. Remote owns the exact system selection, manifests, and the Hardware and Simulator shutdown sequence. The CLI uses the existing Interface and exposes the [CIRNETLAB target architecture](https://github.com/cirnetlab/cirnet-remote-next/blob/233bd694f0c1005ad9aa7c9882d636f68a3e170d/docs/cirnetlab-architecture-en.md).

## Build and run

Put the seven checkouts beside one another. Use PowerShell 7.4+, Visual Studio 2022 x64 C++ tools and Windows SDK, CMake, Git, Rust 1.99.1, and Node 26 with npm. From the CLI checkout:

```powershell
cargo install --locked --path .
cnl doctor
cnl setup
cnl build --all
cnl run --initialize-db --open
cnl status
cnl stop
```

Use `--workspace <path>` from outside the workspace. By default, outputs and SQLite data go in a workspace-specific directory under `%LOCALAPPDATA%\CIRNETLAB`. Use `--state-dir <absolute-external-directory>` and `--profile <name>` to choose isolated state. `cnl env set --port 8000 --hardware-port 50051 --dev-port 5173` saves port choices for the profile. Use `--dry-run` to preview profile changes, builds, launcher actions, or Git mutations.

`--initialize-db` authorizes migration of a fresh local SQLite database. Existing state must pass the native launcher's schema check. The CLI uses the launcher's SQLite profile. For separately configured PostgreSQL, follow [Remote's deployment guides](https://github.com/cirnetlab/cirnet-remote-next/tree/master/docs/en/operations). The CLI's development profile runs virtual Hardware and Simulator.

## Develop and verify

Start with `cnl dev --initialize-db --open` to serve the live Interface checkout through Vite, with its existing API and WebSocket proxies. Choose a different development port when the default is occupied. The native launcher trusts the selected Vite loopback origin. When the candidate changes, the CLI preserves the previous verified input cache under the ignored `.runtime/` directory. Unrecognized cache contents require inspection.

Run `cnl setup --browser` to install the selected candidate's Chromium locally. Run `cnl verify` for the packaged installation, API, persistence, safe Stop and browser checks, or supply `--browser-cache <path>` to use a preinstalled matching Chromium. Without a browser cache, verify reports its reduced coverage. `cnl run`, `cnl dev`, and `cnl verify` accept `--receipt <path-to-assembly-result.json>` to select a previously built candidate.

Use `cnl build --repo <key>` and `cnl verify --repo <key>` for scoped checks. Keys are `cli`, `contracts`, `simulator`, `hardware`, `remote`, `interface`, and `embedded`. The Interface needs the candidate's generated SDK and Remote OpenAPI. Embedded verification requires the pinned Icarus toolchain and Python verification environment described in Embedded Hardware, supplied via `--fpga-toolchain` and optionally `CNL_VERIFICATION_PYTHON`. Embedded runs separately from the native application.

For repository tasks, use `cnl repos discover` or `cnl repos status`. Clone and update require named repositories, and updates require a clean branch tracking its expected CIRNETLAB origin. Pass `--json` for structured reports. Running `cnl` in an interactive terminal opens command suggestions, history and scrollback. Escape can cancel isolated build and check commands; use `cnl stop` for an application stack.

Current release line: `0.3.0-dev.1`. [Changelog](../changelogs/README.md)
