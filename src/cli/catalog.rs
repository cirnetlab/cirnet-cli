use crate::localization;

pub(crate) use crate::localization::{Localized, text};

pub(crate) struct Entry {
    pub label: Localized,
    pub detail: Localized,
    pub command: &'static str,
    pub recommendation: Option<&'static str>,
}

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        label: localization::MENU_DOCTOR_LABEL,
        detail: localization::MENU_DOCTOR_DETAIL,
        command: "doctor",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_ENV_LABEL,
        detail: localization::MENU_ENV_DETAIL,
        command: "env",
        recommendation: Some("show"),
    },
    Entry {
        label: localization::MENU_SETUP_LABEL,
        detail: localization::MENU_SETUP_DETAIL,
        command: "setup",
        recommendation: Some("--browser"),
    },
    Entry {
        label: localization::MENU_REPOS_LABEL,
        detail: localization::MENU_REPOS_DETAIL,
        command: "repos",
        recommendation: Some("status"),
    },
    Entry {
        label: localization::MENU_BUILD_LABEL,
        detail: localization::MENU_BUILD_DETAIL,
        command: "build",
        recommendation: Some("--all"),
    },
    Entry {
        label: localization::MENU_RUN_LABEL,
        detail: localization::MENU_RUN_DETAIL,
        command: "run",
        recommendation: Some("--desktop interface"),
    },
    Entry {
        label: localization::MENU_STATUS_LABEL,
        detail: localization::MENU_STATUS_DETAIL,
        command: "status",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_VERIFY_LABEL,
        detail: localization::MENU_VERIFY_DETAIL,
        command: "verify",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_STOP_LABEL,
        detail: localization::MENU_STOP_DETAIL,
        command: "stop",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_LOGS_LABEL,
        detail: localization::MENU_LOGS_DETAIL,
        command: "logs",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_SHELL_LABEL,
        detail: localization::MENU_SHELL_DETAIL,
        command: "shell",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_COMPLETION_LABEL,
        detail: localization::MENU_COMPLETION_DETAIL,
        command: "completion",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_REFERENCE_LABEL,
        detail: localization::MENU_REFERENCE_DETAIL,
        command: "--help",
        recommendation: None,
    },
    Entry {
        label: localization::MENU_EXIT_LABEL,
        detail: localization::MENU_EXIT_DETAIL,
        command: "",
        recommendation: None,
    },
];

pub(crate) fn detail(words: &str, vi: bool) -> Option<&'static str> {
    ENTRIES
        .iter()
        .find(|entry| {
            entry.command == words || entry.command.split_whitespace().next() == Some(words)
        })
        .map(|entry| text(entry.detail, vi))
}

pub(crate) fn localize(command: clap::Command, vi: bool) -> clap::Command {
    localize_command(command, "", vi)
}

fn localize_command(mut command: clap::Command, parent: &str, vi: bool) -> clap::Command {
    let name = command.get_name();
    let description = match (parent, name) {
        ("repos", "status") => Some(localization::COMMAND_REPO_STATUS),
        (_, name) => match name {
            "cnl" => Some(localization::COMMAND_ROOT),
            "tui" => Some(localization::COMMAND_TUI),
            "dev" => Some(localization::COMMAND_DEV),
            "show" => Some(localization::COMMAND_ENV_SHOW),
            "check" => Some(localization::COMMAND_ENV_CHECK),
            "set" => Some(localization::COMMAND_ENV_SET),
            "discover" => Some(localization::COMMAND_REPO_DISCOVER),
            "clone" => Some(localization::COMMAND_REPO_CLONE),
            "update" => Some(localization::COMMAND_REPO_UPDATE),
            "__complete" => Some(localization::COMMAND_COMPLETION_QUERY),
            _ => None,
        },
    }
    .map(|value| text(value, vi))
    .or_else(|| detail(name, vi));
    let description =
        description.unwrap_or_else(|| panic!("missing command description for {name}"));
    command = command.about(description);
    let parent = command.get_name().to_owned();
    if vi {
        command = command
            .disable_help_flag(true)
            .arg(
                clap::Arg::new("help")
                    .short('h')
                    .long("help")
                    .action(clap::ArgAction::Help),
            )
            .subcommand_help_heading("Các lệnh")
            .help_template(
                "{before-help}{about-with-newline}\nCách dùng: {usage}\n\n{all-args}{after-help}",
            );
        if parent == "cnl" {
            command = command.disable_version_flag(true).arg(
                clap::Arg::new("version")
                    .short('V')
                    .long("version")
                    .action(clap::ArgAction::Version),
            );
        }
    }
    command
        .mut_args(|arg| {
            let id = arg.get_id().as_str();
            let label = localization::option(id)
                .unwrap_or_else(|| panic!("missing option description for {parent}::{id}"));
            let arg = arg.help(text(label, vi));
            if vi {
                let heading = if arg.get_long().is_some() {
                    "Tùy chọn"
                } else {
                    "Đối số"
                };
                arg.help_heading(heading)
            } else {
                arg
            }
        })
        .mut_subcommands(|child| localize_command(child, &parent, vi))
}
