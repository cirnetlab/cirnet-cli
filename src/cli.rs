pub(crate) mod catalog;
pub(crate) mod native;

use std::{io::IsTerminal, path::PathBuf};

use anyhow::{Result, bail};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, value_parser};
use serde_json::{Value, json};

use crate::{
    build,
    environment::{self, State},
    localization::{message, report_field, set_language},
    runtime, tui,
    workspace::{Repo, Workspace},
};

fn flag(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .action(ArgAction::SetTrue)
        .help(help)
}

fn path(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_parser(value_parser!(PathBuf))
        .help(help)
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
        .help("Select the Interface desktop target")
}

fn runtime_args(name: &'static str, modes: &'static [&'static str]) -> Command {
    ports(
        Command::new(name).args([
            path("receipt", "Remote assembly-result.json"),
            desktop(),
            Arg::new("mode")
                .long("mode")
                .value_parser(clap::builder::PossibleValuesParser::new(
                    modes.iter().copied(),
                ))
                .default_value("development")
                .help("Choose the Interface runtime mode"),
            flag("initialize-db", "Initialize a new local SQLite database"),
            flag("open", "Open the application after readiness"),
            flag("dry-run", "Preview without applying"),
        ]),
    )
}

pub fn app() -> Command {
    app_for(false)
}

pub(crate) fn app_for(vi: bool) -> Command {
    let repo_change = |operation, help| {
        Command::new(operation)
            .about(help)
            .arg(Arg::new("targets").num_args(1..).value_parser(Repo::KEYS))
            .arg(flag("all", "Select every repository in the workspace"))
            .group(
                ArgGroup::new("scope")
                    .args(["targets", "all"])
                    .required(true),
            )
            .arg(flag(
                "dry-run",
                "Check the selected operations without applying",
            ))
    };
    catalog::localize(
        Command::new("cnl")
            .version(env!("CARGO_PKG_VERSION"))
            .about("CIRNETLAB native development tools")
            .args([
                path(
                    "workspace",
                    "Root containing the independent CIRNETLAB repositories",
                )
                .global(true),
                path(
                    "state-dir",
                    "Absolute external directory for state and artifacts",
                )
                .global(true),
                Arg::new("profile")
                    .long("profile")
                    .global(true)
                    .default_value("windows-x64"),
                flag("json", "Emit a JSON report on stdout").global(true),
                Arg::new("lang")
                    .long("lang")
                    .value_parser(["en", "vi"])
                    .default_value("en")
                    .help("Terminal language")
                    .global(true),
            ])
            .subcommands([
                Command::new("tui").about("Open the CIRNETLAB terminal"),
                Command::new("doctor").about("Inspect the Windows x64 toolchain"),
                Command::new("env")
                    .about("Show, validate or update local profiles")
                    .subcommands([
                        Command::new("show"),
                        Command::new("check"),
                        ports(Command::new("set").arg(flag("dry-run", "Preview profile changes"))),
                    ]),
                Command::new("setup")
                    .about("Prepare profile directories and optional Chromium")
                    .args([
                        flag(
                            "browser",
                            "Install the selected candidate's Chromium for testing",
                        ),
                        path("receipt", "Remote assembly-result.json"),
                        flag("dry-run", "Preview workspace preparation"),
                    ]),
                Command::new("repos")
                    .about("Manage the independent checkouts")
                    .subcommand_required(true)
                    .subcommands([
                        Command::new("discover").about("List expected repository locations"),
                        Command::new("status")
                            .about("Report branches, changes, remotes and dependencies"),
                        repo_change("clone", "Clone explicitly selected missing checkouts"),
                        repo_change("update", "Fast-forward explicitly selected clean checkouts"),
                    ]),
                Command::new("build")
                    .about("Build selected components or the complete native system")
                    .group(ArgGroup::new("scope").args(["all", "repo"]).required(true))
                    .args([
                        flag("all", "Build the full Windows x64 candidate"),
                        selected(),
                        flag("offline", "Use cached dependency inputs"),
                        flag("dry-run", "Preview build order"),
                        path("receipt", "Existing candidate inputs for Interface or RTL"),
                        path("fpga-toolchain", "Pinned RTL toolchain root"),
                    ]),
                runtime_args("run", &["development", "packaged"]),
                runtime_args("dev", &["development"]).hide(true),
                Command::new("status").about("Check the owned supervisor, API, RPC and UI"),
                Command::new("stop")
                    .about("Stop the owned application gracefully")
                    .arg(desktop())
                    .arg(flag("dry-run", "Preview the owned Stop operation")),
                Command::new("logs").args([
                    Arg::new("service")
                        .long("service")
                        .value_parser(["native", "vite"])
                        .default_value("native")
                        .help("Select the runtime log"),
                    Arg::new("lines")
                        .long("lines")
                        .value_parser(value_parser!(u16).range(1..=500))
                        .default_value("60")
                        .help("Maximum number of lines to show"),
                ]),
                Command::new("shell").arg(
                    Arg::new("shell")
                        .value_parser(native::SHELLS)
                        .help("Select a native shell"),
                ),
                Command::new("verify")
                    .about("Run native integration checks or selected component tests")
                    .args([
                        path("receipt", "Remote assembly-result.json"),
                        selected(),
                        path("browser-cache", "Absolute Playwright Chromium directory"),
                        path("fpga-toolchain", "Pinned RTL toolchain root"),
                        flag("dry-run", "Preview verification"),
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
    let matches = app_for(vi).get_matches();
    if let Some((name, args)) = matches.subcommand() {
        match name {
            "completion" => return tui::completion::emit(args),
            "__complete" => return tui::completion::query(args),
            "shell" => {
                if matches.get_flag("json")
                    || !std::io::stdin().is_terminal()
                    || !std::io::stdout().is_terminal()
                {
                    bail!(
                        "{}",
                        catalog::text(
                            [
                                "Open a native shell from an interactive terminal.",
                                "Mở shell native từ terminal tương tác."
                            ],
                            vi
                        )
                    );
                }
                let status =
                    native::interactive(args.get_one::<String>("shell").map(String::as_str))?;
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
            bail!(
                "{}",
                message([
                    "Select a command in non-interactive mode. See cnl --help.",
                    "Hãy chọn một lệnh khi chạy ở chế độ không tương tác. Xem cnl --help."
                ])
            );
        }
        return tui::open(&workspace, &matches);
    };
    if name == "tui" {
        return tui::open(&workspace, &matches);
    }
    if name == "repos" {
        return display(
            repo_command(&workspace, arguments)?,
            matches.get_flag("json"),
            vi,
        );
    }
    if name == "doctor" {
        let report = environment::doctor(&workspace);
        let ok = report["ok"] == true;
        display(report, matches.get_flag("json"), vi)?;
        if !ok {
            bail!(
                "{}",
                message([
                    "Toolchain prerequisites are incomplete",
                    "Chưa đủ điều kiện toolchain"
                ])
            );
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
                    bail!(
                        "{}",
                        message([
                            "Fix the prerequisites reported by cnl doctor before setup",
                            "Hãy xử lý các điều kiện do cnl doctor báo trước khi chạy setup"
                        ])
                    );
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

fn repo_command(workspace: &Workspace, args: &ArgMatches) -> Result<Value> {
    let (action, options) = args.subcommand().expect("required repository operation");
    if matches!(action, "discover" | "status") {
        return Ok(
            json!({"workspace":workspace.root,"repositories":workspace.inventory(action == "status")}),
        );
    }
    let mut selected = Vec::new();
    if options.get_flag("all") {
        selected = if action == "clone" {
            workspace.missing_checkouts()?
        } else {
            Repo::ALL.to_vec()
        };
    } else {
        for key in options.get_many::<String>("targets").into_iter().flatten() {
            let repo = Repo::parse(key);
            if !selected.contains(&repo) {
                selected.push(repo);
            }
        }
    }
    Ok(json!({"operations":workspace.mutate(action,&selected,options.get_flag("dry-run"))?}))
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

fn display(value: Value, json_output: bool, vi: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(content) = value["content"].as_str() {
        println!("{content}");
    } else if let Some(url) = value["url"].as_str() {
        let state = value["state"].as_str().unwrap_or("unknown");
        let status = match state {
            "ready" => catalog::text(["Ready", "Sẵn sàng"], vi),
            "unhealthy" => catalog::text(["Unhealthy", "Dịch vụ gặp lỗi"], vi),
            "exited-without-confirmed-stop" => catalog::text(
                [
                    "Exited without confirmed Stop",
                    "Đã thoát trước khi xác nhận Stop",
                ],
                vi,
            ),
            _ => state,
        };
        let mode = match value["mode"].as_str() {
            Some("development") => catalog::text(["development", "phát triển"], vi),
            Some("packaged") => catalog::text(["packaged", "đóng gói"], vi),
            Some(mode) => mode,
            None => "",
        };
        println!("{status} · Interface ({mode}) · {url}");
        if let Some(port) = value["ports"]["hardware_port"].as_u64() {
            println!(
                "{} · 127.0.0.1:{port}",
                catalog::text(["Hardware RPC", "Hardware RPC"], vi)
            );
        }
        if let Some(path) = value["logDirectory"].as_str() {
            println!("{} · {path}", catalog::text(["Logs", "Nhật ký"], vi));
        }
    } else if value["state"] == "stopped" {
        println!("{}", catalog::text(["Stopped", "Đã dừng"], vi));
        if let Some(path) = value["logDirectory"].as_str() {
            println!("{} · {path}", catalog::text(["Logs", "Nhật ký"], vi));
        }
    } else {
        for (label, item) in value.as_object().into_iter().flatten() {
            let label = report_field(label, vi);
            if let Some(text) = item.as_str() {
                println!("{label}: {text}");
            } else {
                println!("{label}: {}", serde_json::to_string_pretty(item)?);
            }
        }
    }
    Ok(())
}
