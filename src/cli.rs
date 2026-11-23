pub(crate) mod catalog;
pub(crate) mod native;

use std::{fmt::Write as _, io::IsTerminal, path::PathBuf};

use anyhow::{Result, bail};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, value_parser};
use serde_json::{Value, json};

use crate::{
    build,
    environment::{self, State},
    localization::{self, message, set_language},
    repositories, runtime, tui,
    workspace::{Repo, Workspace},
};

fn flag(name: &'static str) -> Arg {
    Arg::new(name).long(name).action(ArgAction::SetTrue)
}

fn path(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_parser(value_parser!(PathBuf))
}

fn port(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_parser(value_parser!(u16).range(1024..=65535))
}

fn ports(command: Command) -> Command {
    command.args([port("port"), port("hardware-port"), port("dev-port")])
}

fn selected() -> Arg {
    Arg::new("repo")
        .long("repo")
        .action(ArgAction::Append)
        .value_parser(Repo::KEYS)
}

fn desktop() -> Arg {
    Arg::new("desktop")
        .long("desktop")
        .value_parser(runtime::Target::VALUES)
}

fn runtime_args(name: &'static str, modes: &'static [&'static str]) -> Command {
    ports(
        Command::new(name).args([
            path("receipt"),
            desktop(),
            Arg::new("mode")
                .long("mode")
                .value_parser(clap::builder::PossibleValuesParser::new(
                    modes.iter().copied(),
                ))
                .default_value("development"),
            flag("initialize-db"),
            flag("open"),
            flag("dry-run"),
        ]),
    )
}

pub fn app() -> Command {
    app_for(false)
}

pub(crate) fn app_for(vi: bool) -> Command {
    catalog::localize(
        Command::new("cnl")
            .version(env!("CARGO_PKG_VERSION"))
            .args([
                path("workspace").global(true),
                path("state-dir").global(true),
                Arg::new("profile")
                    .long("profile")
                    .global(true)
                    .default_value("windows-x64"),
                flag("json").global(true),
                flag("persist-history").global(true),
                Arg::new("lang")
                    .long("lang")
                    .value_parser(["en", "vi"])
                    .default_value("en")
                    .global(true),
            ])
            .subcommands([
                Command::new("tui"),
                Command::new("doctor"),
                Command::new("env").subcommands([
                    Command::new("show"),
                    Command::new("check"),
                    ports(Command::new("set").arg(flag("dry-run"))),
                ]),
                Command::new("setup").args([flag("browser"), path("receipt"), flag("dry-run")]),
                repositories::command(),
                Command::new("build")
                    .group(ArgGroup::new("scope").args(["all", "repo"]).required(true))
                    .args([
                        flag("all"),
                        selected(),
                        flag("offline"),
                        flag("dry-run"),
                        path("receipt"),
                        path("fpga-toolchain"),
                    ]),
                runtime_args("run", &["development", "packaged"]),
                runtime_args("dev", &["development"]).hide(true),
                Command::new("status"),
                Command::new("stop").arg(desktop()).arg(flag("dry-run")),
                Command::new("logs").args([
                    Arg::new("service")
                        .long("service")
                        .value_parser(["native", "vite"])
                        .default_value("native"),
                    Arg::new("lines")
                        .long("lines")
                        .value_parser(value_parser!(u16).range(1..=500))
                        .default_value("60"),
                ]),
                Command::new("shell").arg(Arg::new("shell").value_parser(native::SHELLS)),
                Command::new("verify").args([
                    path("receipt"),
                    selected(),
                    path("browser-cache"),
                    path("fpga-toolchain"),
                    flag("dry-run"),
                ]),
            ])
            .subcommand(super::tui::completion::command())
            .subcommand(super::tui::completion::query_command()),
        vi,
    )
}

pub fn run() -> Result<()> {
    let vi = crate::localization::requested_language(std::env::args_os());
    set_language(vi);
    let matches = match app_for(vi).try_get_matches() {
        Ok(matches) => matches,
        Err(error) if !vi => error.exit(),
        Err(error) => {
            let output = parse_error(&error, vi);
            if error.use_stderr() {
                eprint!("{output}");
            } else {
                print!("{output}");
            }
            std::process::exit(error.exit_code());
        }
    };
    if let Some((name, args)) = matches.subcommand() {
        match name {
            "completion" => return tui::completion::emit(args),
            "__complete" => return tui::completion::query(args),
            "shell" => {
                if matches.get_flag("json")
                    || !std::io::stdin().is_terminal()
                    || !std::io::stdout().is_terminal()
                {
                    bail!("{}", catalog::text(localization::SHELL_INTERACTIVE, vi));
                }
                let selected = args.get_one::<String>("shell").map(String::as_str);
                let workspace = Workspace::discover(
                    matches
                        .get_one::<PathBuf>("workspace")
                        .map(PathBuf::as_path),
                )
                .ok();
                let active = workspace
                    .as_ref()
                    .map(|workspace| repositories::active_context(workspace, &matches))
                    .transpose()?
                    .flatten();
                let (directory, preferred) = active
                    .as_ref()
                    .map_or((None, None), |(directory, preferred)| {
                        (Some(directory.as_path()), preferred.as_deref())
                    });
                let status = native::interactive_in(selected.or(preferred), directory)?;
                std::process::exit(status.code().unwrap_or(1));
            }
            _ => {}
        }
    }
    crate::process::set_json_output(matches.get_flag("json"));
    let workspace = Workspace::discover(
        matches
            .get_one::<PathBuf>("workspace")
            .map(PathBuf::as_path),
    )?;
    let Some((name, arguments)) = matches.subcommand() else {
        if matches.get_flag("json")
            || !std::io::stdin().is_terminal()
            || !std::io::stdout().is_terminal()
        {
            bail!("{}", message(localization::COMMAND_REQUIRED));
        }
        return tui::open(&workspace, &matches);
    };
    if name == "tui" {
        return tui::open(&workspace, &matches);
    }
    if name == "repos" {
        if let Some(update) = repositories::wants_dashboard(arguments) {
            if matches.get_flag("json")
                || !std::io::stdin().is_terminal()
                || !std::io::stdout().is_terminal()
            {
                eprintln!("{}", repositories::interactive_required(update));
                std::process::exit(2);
            }
            return tui::open_repositories(&workspace, &matches, update);
        }
        if let Some(terminal) = arguments.subcommand_matches("terminal") {
            if matches.get_flag("json")
                || !std::io::stdin().is_terminal()
                || !std::io::stdout().is_terminal()
            {
                bail!("{}", repositories::interactive_required(false));
            }
            return tui::open_repository_terminal(&workspace, &matches, terminal);
        }
        let report = repositories::execute(&workspace, &matches, arguments)?;
        let failed = report["ok"] == false;
        display(report, matches.get_flag("json"), vi)?;
        if failed {
            bail!("{}", repositories::operation_failed());
        }
        return Ok(());
    }
    if name == "doctor" {
        let report = environment::doctor(&workspace);
        let ok = report["ok"] == true;
        display(report, matches.get_flag("json"), vi)?;
        if !ok {
            bail!("{}", message(localization::TOOLCHAIN_INCOMPLETE));
        }
        return Ok(());
    }
    let mut state = State::resolve(
        &workspace,
        matches
            .get_one::<PathBuf>("state-dir")
            .map(PathBuf::as_path),
        matches.get_one::<String>("profile").unwrap(),
    )?;
    let report = match name {
        "env" => {
            let (action, options) = arguments.subcommand().unwrap_or(("show", arguments));
            if action == "set" {
                update_ports(&mut state, options)?;
                if !options.get_flag("dry-run") {
                    state.save_profile()?;
                }
            }
            let mut value =
                json!({"stateDirectory":state.dir, "profile":state.profile,"database":"sqlite"});
            if action == "check" {
                value["doctor"] = environment::doctor(&workspace);
            }
            value
        }
        "setup" => {
            let browser = arguments.get_flag("browser");
            let cache = state.dir.join("chromium");
            if !arguments.get_flag("dry-run") {
                let checks = environment::doctor(&workspace);
                if checks["ok"] != true {
                    bail!("{}", message(localization::SETUP_PREREQUISITES));
                }
                let _lock = state.lock()?;
                if browser {
                    let candidate =
                        build::candidate(&workspace, &state, arguments.get_one("receipt"))?;
                    environment::install_browser(&candidate.receipt, &cache)?;
                }
            }
            json!({"stateDirectory":state.dir,"browserCache":browser.then_some(cache),"dryRun":arguments.get_flag("dry-run")})
        }
        "build" => build::build(&workspace, &state, arguments)?,
        "verify" => build::verify(&workspace, &state, arguments)?,
        "run" | "dev" => {
            update_ports(&mut state, arguments)?;
            runtime::start(&workspace, &state, arguments)?
        }
        "status" => runtime::status(&state)?,
        "stop" => runtime::stop(
            &state,
            runtime::Target::requested(arguments),
            arguments.get_flag("dry-run"),
        )?,
        "logs" => runtime::logs(&state, arguments)?,
        _ => unreachable!(),
    };
    display(report, matches.get_flag("json"), vi)
}

pub(crate) fn local_terminal_language(args: &[String]) -> Option<bool> {
    if !args
        .iter()
        .any(|arg| arg == "--lang" || arg.starts_with("--lang="))
    {
        return None;
    }
    let matches = app()
        .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)))
        .ok()?;
    if !matches!(matches.subcommand_name(), None | Some("tui"))
        || matches.get_flag("json")
        || ["workspace", "state-dir", "profile"]
            .iter()
            .any(|id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
    {
        return None;
    }
    matches.get_one::<String>("lang").map(|lang| lang == "vi")
}

fn update_ports(state: &mut State, input: &ArgMatches) -> Result<()> {
    for (name, value) in [
        ("port", &mut state.profile.port),
        ("hardware-port", &mut state.profile.hardware_port),
        ("dev-port", &mut state.profile.dev_port),
    ] {
        if let Some(port) = input.get_one::<u16>(name) {
            *value = *port;
        }
    }
    state.profile.validate()
}

pub(crate) const TUI_REPORT_MARKER: &str = "CNL_TUI_REPORT ";

fn display(value: Value, json_output: bool, vi: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(content) = value["content"].as_str() {
        println!("{content}");
    } else if let Some(nonce) = localization::tui_report_nonce() {
        println!(
            "{TUI_REPORT_MARKER}{nonce} {}",
            serde_json::to_string(&value)?
        );
    } else {
        println!("{}", human_report(&value, vi));
    }
    Ok(())
}

pub(crate) fn human_report(value: &Value, vi: bool) -> String {
    let mut output = String::new();
    render_report(&mut output, value, "", 0, vi);
    output.pop();
    output
}

fn render_report(output: &mut String, value: &Value, key: &str, indent: usize, vi: bool) {
    let lookup = |label| localization::text(label, vi);
    if let Some(detail) = localization::human_detail(value, vi) {
        let _ = writeln!(output, "{detail}");
        return;
    }
    match value {
        Value::Object(fields) if !fields.is_empty() => {
            for (key, value) in fields {
                let label = localization::report_field(key).map_or(key.as_str(), lookup);
                let _ = write!(output, "{:indent$}{label}:", "");
                if report_section(value) {
                    output.push('\n');
                } else {
                    output.push(' ');
                }
                render_report(output, value, key, indent + 2, vi);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                if report_section(item) {
                    if index > 0 {
                        output.push('\n');
                    }
                } else {
                    let _ = write!(output, "{:indent$}- ", "");
                }
                render_report(output, item, key, indent, vi);
            }
        }
        Value::String(value) => {
            if matches!(
                key,
                "path"
                    | "workspace"
                    | "commonDir"
                    | "selector"
                    | "stateDirectory"
                    | "logDirectory"
                    | "dataDirectory"
                    | "bundle"
                    | "receipt"
            ) {
                let _ = writeln!(
                    output,
                    "{}",
                    crate::workspace::user_path(std::path::Path::new(value))
                );
            } else {
                let value = localization::report_value(key, value).map_or(value.as_str(), lookup);
                let _ = writeln!(output, "{value}");
            }
        }
        Value::Bool(value) => {
            let _ = writeln!(
                output,
                "{}",
                lookup(localization::report_boolean(key, *value))
            );
        }
        Value::Number(value) => {
            let _ = writeln!(output, "{value}");
        }
        Value::Null | Value::Array(_) | Value::Object(_) => output.push_str("—\n"),
    }
}

fn report_section(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            !fields.is_empty() && localization::human_detail(value, false).is_none()
        }
        Value::Array(items) => !items.is_empty(),
        _ => false,
    }
}

pub(crate) fn parse_error(error: &clap::Error, vi: bool) -> String {
    if vi {
        localization::clap_diagnostic(error)
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_displays_friendly_paths_without_rewriting_json_or_raw_output() {
        let drive = r"\\?\E:\repo\alpha";
        let network = r"\\?\UNC\server\share\beta";
        let report = json!({"path":drive,"commonDir":network,"detail":drive});
        for vi in [false, true] {
            let output = human_report(&report, vi);
            assert!(output.contains(r": E:\repo\alpha"));
            assert!(output.contains(r": \\server\share\beta"));
            assert!(output.contains(drive), "raw Git diagnostics are preserved");
        }
        assert_eq!(report["path"], drive);
        assert_eq!(report["commonDir"], network);
    }

    #[test]
    fn human_discovery_report_normalizes_only_path_shaped_selectors() {
        let drive = r"\\?\E:\repo\alpha";
        let share = r"\\?\UNC\server\share\beta";
        let report = json!({
            "repositories": [
                { "selector": drive, "path": drive, "commitMessage": drive },
                { "selector": share, "path": share },
                { "selector": "r:42" },
                { "selector": "cirnetlab/example" }
            ]
        });
        for (vi, label) in [(false, "Identifier"), (true, "Định danh")] {
            let output = human_report(&report, vi);
            for selector in [
                r"E:\repo\alpha",
                r"\\server\share\beta",
                "r:42",
                "cirnetlab/example",
            ] {
                assert!(
                    output
                        .lines()
                        .any(|line| line.trim() == format!("{label}: {selector}")),
                    "{output}"
                );
            }
            assert!(output.lines().any(|line| line.ends_with(drive)));
        }
        assert_eq!(report["repositories"][0]["selector"], drive);
        assert_eq!(report["repositories"][1]["selector"], share);
    }

    #[test]
    fn human_report_translates_only_owned_report_values() {
        let value = json!({
            "repositories": [
                {
                    "branch": "detached",
                    "detail": "Could not start a tool",
                    "changes": ["A  input development.log"],
                    "state": "checkout"
                }
            ],
            "mode": "development",
            "native": {
                "healthy": false,
                "hardwareRpc": true,
                "hardwareDetail": "upstream detail: ready"
            },
            "url": "http://127.0.0.1:8000/"
        });
        let vi = human_report(&value, true);
        for unchanged in [
            "detached",
            "Could not start a tool",
            "A  input development.log",
            "upstream detail: ready",
            "http://127.0.0.1:8000/",
        ] {
            assert!(vi.contains(unchanged));
        }
        for localized in [
            "Có checkout",
            "Phát triển",
            "Dịch vụ gặp lỗi",
            "Hardware RPC: ĐẠT",
        ] {
            assert!(vi.contains(localized));
        }
    }
}
