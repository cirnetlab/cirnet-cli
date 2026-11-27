# CIRNETLAB CLI repository work

- Use the [English](docs/en/index.md) and [Vietnamese](docs/vi/index.md) guides. Search before creating documentation, update the authoritative page, verify links, and keep both languages and the [changelog](docs/changelogs/README.md) synchronized.
- Use [Task Continuity](https://github.com/cirnetlab/cirnet-rules/blob/master/en/docs/distribution.md#task-continuity) for temporary plans, acceptance criteria and progress. Keep independent design specifications and durable plans in this repository and reference them from the checkpoint. Mark items complete after verification.
- Remote owns the system candidate manifest and native service shutdown. The CLI owns local orchestration and uses the component repositories' public build and verification entry points. Keep physical execution outside the CLI development profile.
- Distinguish TARGET, IMPLEMENTED, TESTED, VERIFIED, RELEASED, UNAVAILABLE and BLOCKED. Keep raw verification outputs outside checkouts and Git directories. Review Git diff and status. Stage, commit, push, tag or publish only on request.
