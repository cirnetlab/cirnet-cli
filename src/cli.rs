pub(crate) mod catalog;

use std::{io::IsTerminal, path::PathBuf};

use anyhow::{Result, bail};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, value_parser};
use serde_json::{Value, json};

use crate::{
    build,
    environment::{self, State},
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

fn runtime_args(name: &'static str, help: &'static str) -> Command {
    ports(
        Command::new(name).about(help).args([
            path("receipt", "Remote assembly-result.json"),
            Arg::new("desktop")
                .long("desktop")
                .value_parser(["interface"])
                .help("Open the existing Interface in the desktop browser"),
            flag("initialize-db", "Initialize a new local SQLite database"),
            flag("open", "Open the application after readiness"),
            flag("dry-run", "Preview without applying"),
        ]),
    )
}

pub fn app() -> Command {
    let repo_change = |operation, help| {
        Command::new(operation)
            .about(help)
            .arg(
                Arg::new("targets")
                    .required(true)
                    .num_args(1..)
                    .value_parser(Repo::KEYS),
            )
            .arg(flag(
                "dry-run",
                "Check the selected operations without applying",
            ))
    };
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
            runtime_args("run", "Start the verified native application"),
            runtime_args("dev", "Start native services with hot-reload Vite"),
            Command::new("status").about("Check the owned supervisor, API, RPC and UI"),
            Command::new("stop")
                .about("Stop the owned application gracefully")
                .arg(flag("dry-run", "Preview the owned Stop operation")),
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
        .mut_subcommands(|command| {
            if let Some(description) = catalog::detail(command.get_name(), false) {
                command.about(description)
            } else {
                command
            }
        })
}

pub fn run() -> Result<()> {
    let matches = app().get_matches();
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
            bail!("Select a command in non-interactive mode. See cnl --help.");
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
        );
    }
    if name == "doctor" {
        let report = environment::doctor(&workspace);
        let ok = report["ok"] == true;
        display(report, matches.get_flag("json"))?;
        if !ok {
            bail!("Toolchain prerequisites are incomplete");
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
                    bail!("Fix the prerequisites reported by cnl doctor before setup");
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
            runtime::start(&workspace, &state, arguments, name == "dev")?
        }
        "status" => runtime::status(&state)?,
        "stop" => runtime::stop(&state, arguments.get_flag("dry-run"))?,
        _ => unreachable!(),
    };
    display(report, matches.get_flag("json"))
}

fn repo_command(workspace: &Workspace, args: &ArgMatches) -> Result<Value> {
    let (action, options) = args.subcommand().expect("required repository operation");
    if matches!(action, "discover" | "status") {
        return Ok(
            json!({"workspace":workspace.root,"repositories":workspace.inventory(action == "status")}),
        );
    }
    let mut selected = Vec::new();
    for key in options.get_many::<String>("targets").unwrap() {
        let repo = Repo::parse(key);
        if !selected.contains(&repo) {
            selected.push(repo);
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

fn display(value: Value, json_output: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(url) = value["url"].as_str() {
        if let Some(state) = value["state"].as_str() {
            println!("CIRNETLAB {state} at {url}");
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for (label, item) in value.as_object().into_iter().flatten() {
            if let Some(text) = item.as_str() {
                println!("{label}: {text}");
            } else {
                println!("{label}: {}", serde_json::to_string_pretty(item)?);
            }
        }
    }
    Ok(())
}
