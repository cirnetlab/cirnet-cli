# CIRNETLAB CLI version 0 changelog

[Changelog index](README.md)

## Unreleased

### Changed

- Standardize English major-version changelog navigation.

## v0.3.0-dev.1 — Development

This is a development version used in the Windows x64 Native Candidate workflow. The available Git tags and GitHub Releases provide no publication date for this version.

### Added

- Added the CIRNETLAB terminal and Windows x64 toolchain, repository, candidate build, runtime and verification commands.
- Reused the Remote launcher to supervise the native stack. Added an owned Vite process for live browser development.
- Added English and Vietnamese CNL and Cargo command references.
- Add contextual shell and mouse completion, native shell delegation, localized command help and results, and refresh the terminal wordmark, context menu and activity feedback.
- Share bounded command history across views and let users save eligible CNL history for a profile.
- Add local Git and native shell suggestions, responsive layouts and a reduced-motion-aware input border.
- Add a workspace repository registry, cached GitHub discovery, parallel Git synchronization, and a dashboard with persistent repository terminals.
- Refine the terminal toolbar and session navigator. Add adjustable splits, pane maximization, scrollback search, and visible text selection.
- Detect native Bash, Git Bash and selectable WSL Bash profiles while preserving native terminal completion.
- Use checkout and shell context in the CNL palette with bounded local Git, PowerShell, CMD, Bash and WSL metadata.

### Changed

- Adapted the CNR terminal menu, orange wordmark and incremental painter. Added localized editing, command completion and bounded live output.
- Made workspace discovery follow the executable and added `--desktop interface` for verified native and Vite starts.
- Run build and verification subprocesses in their owned directories. Bind candidate selection to its workspace and profile.
- Reset menu drafts between commands, switch TUI language in place and preserve subprocess output streams.
- Default to development mode for `run`, retain `dev` compatibility and add target-aware Stop, runtime logs and multi-repository scopes.
- Localize nested reports and switch the TUI language with F8 while editing or reading output.
- Soften the copper input border sweep and pause its motion while editing.
- Show verified Git metadata and readable paths in Repository Manager. Add hierarchical navigation, shared CNL command entry, and a split terminal workspace with a session list.
- Widen Local path by removing the ahead and behind column from the repository table.
- Tighten the Git status and Branch spacing, widen Local path, and remove the path tooltip and horizontal scrolling from the repository table.
- Remove the F10 Back button from the terminal toolbar and trim the footer shortcuts.
- Show standard Windows paths in repository reports and open PowerShell in the equivalent Windows directory.
- Render Terminal sidebar session names, shell status, and filter results on the sidebar background.
- Remove the two-line session details footer from the Terminal sidebar.
- Show a terminal icon, shell, and repository on each sidebar row, with split and close actions on hover.
- Use a shell prompt and two pane outlines for the terminal icons.
- Group split terminal panes in the sidebar and show their relationship with tree connectors.
- Align sidebar terminal icons and show the Split icon when hovering over a session. The toolbar uses text labels for Split and Unsplit.
- Routed component development output through Contracts, guarded shared build caches and added explicit reuse of verified native candidates.

### Fixed

- Refresh the Repository Manager's GitHub catalog in the background and show distinct local organization checkouts over the accessible remote total.
- Preserve split terminal groups when opening or selecting an independent session.
- Recover the same virtual runtime profile after a confirmed native shutdown or Windows restart, with checks for active processes, native locks and occupied ports.

**Sources**

- [Version manifest](../../Cargo.toml)
- [Native implementation baseline](https://github.com/cirnetlab/cirnet-cli/commit/ffc482c)
- [Latest source milestone recorded here](https://github.com/cirnetlab/cirnet-cli/commit/7635722682c45b0182b1c3299eb4283e3b9ad635)
