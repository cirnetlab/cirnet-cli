mod catalog;
mod local;

use super::{Kind, LIMIT, Source, Suggestion, fuzzy_score};
use local::{Cache, Key};
use std::{
    cell::RefCell,
    env,
    ops::Range,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    Pwsh,
    Cmd,
    Bash,
    GitBash,
    Wsl,
}

impl Shell {
    fn host() -> Self {
        if cfg!(windows) {
            Self::Pwsh
        } else {
            Self::Bash
        }
    }

    fn windows_syntax(self) -> bool {
        matches!(self, Self::Pwsh | Self::Cmd)
    }

    fn bash_syntax(self) -> bool {
        matches!(self, Self::Bash | Self::GitBash | Self::Wsl)
    }

    fn separator(self, ch: char) -> bool {
        ch == '/' || (self.windows_syntax() && ch == '\\')
    }

    fn escaping(self, ch: char, quote: Option<char>) -> bool {
        match self {
            Self::Pwsh => ch == '\u{60}' && quote != Some('\''),
            Self::Cmd => ch == '^' && quote.is_none(),
            Self::Bash | Self::GitBash | Self::Wsl => ch == '\\' && quote != Some('\''),
        }
    }
}

pub(super) struct Context {
    host_directory: Option<PathBuf>,
    directory: Option<PathBuf>,
    shell: Shell,
    distribution: Option<String>,
}

impl Default for Context {
    fn default() -> Self {
        let host_directory = env::current_dir().ok();
        Self {
            directory: host_directory.clone(),
            host_directory,
            shell: Shell::host(),
            distribution: None,
        }
    }
}

impl Context {
    pub(super) fn routes_env_wrapper(&self, line: &str) -> bool {
        if !self.shell.bash_syntax() {
            return false;
        }
        let mut words = line.split_whitespace();
        words.next() == Some("env")
            && words.next().is_some_and(|word| {
                word == "--"
                    || word.split_once('=').is_some_and(|(name, _)| {
                        !name.is_empty()
                            && name.bytes().enumerate().all(|(i, byte)| {
                                byte == b'_'
                                    || byte.is_ascii_alphabetic()
                                    || i > 0 && byte.is_ascii_digit()
                            })
                    })
            })
    }

    pub(super) fn set(&mut self, directory: Option<&Path>, profile: Option<&str>) {
        self.directory = directory
            .map(Path::to_path_buf)
            .or_else(|| self.host_directory.clone());
        self.distribution = profile
            .and_then(|profile| profile.strip_prefix("wsl:"))
            .map(str::to_owned);
        self.shell = match profile {
            Some("cmd") => Shell::Cmd,
            Some("powershell" | "pwsh") => Shell::Pwsh,
            Some("git-bash") => Shell::GitBash,
            Some("bash") => Shell::Bash,
            Some("wsl") => Shell::Wsl,
            _ if self.distribution.is_some() => Shell::Wsl,
            _ => Shell::host(),
        };
    }
}

struct Word {
    range: Range<usize>,
    value: String,
    boundary: bool,
    redirect_path: bool,
}

fn words(input: &str, start: usize, shell: Shell) -> Vec<Word> {
    let mut words = Vec::new();
    let mut chars = input[start..]
        .char_indices()
        .map(|(index, ch)| (start + index, ch))
        .peekable();
    let boundary = |ch| matches!(ch, '|' | '&') || (ch == ';' && shell != Shell::Cmd);
    while let Some((start, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if boundary(ch) {
            chars.next();
            words.push(Word {
                range: start..start + ch.len_utf8(),
                value: String::new(),
                boundary: true,
                redirect_path: false,
            });
            continue;
        }
        if matches!(ch, '<' | '>') {
            chars.next();
            let mut redirect_path = true;
            if chars.next_if(|(_, next)| *next == ch).is_some() && ch == '<' {
                // Here-documents and here-strings take text, not a file.
                let _ = chars.next_if(|(_, next)| *next == '<');
                redirect_path = false;
            }
            if chars.next_if(|(_, next)| *next == '&').is_some() {
                redirect_path = false;
            } else if ch == '>' {
                let _ = chars.next_if(|(_, next)| *next == '|');
            }
            let end = chars.peek().map_or(input.len(), |(index, _)| *index);
            words.push(Word {
                range: start..end,
                value: input[start..end].into(),
                boundary: false,
                redirect_path,
            });
            continue;
        }
        let mut value = String::new();
        let mut quote = None;
        while let Some((_, ch)) = chars.peek().copied() {
            if quote.is_none() && (ch.is_whitespace() || boundary(ch) || matches!(ch, '<' | '>')) {
                break;
            }
            chars.next();
            if shell.escaping(ch, quote) {
                if let Some((_, next)) = chars.next() {
                    if shell.bash_syntax()
                        && quote == Some('"')
                        && !matches!(next, '$' | '\u{60}' | '"' | '\\' | '\n')
                    {
                        value.push(ch);
                    }
                    if next != '\n' {
                        value.push(next);
                    }
                } else {
                    value.push(ch);
                }
            } else if (ch == '"' || (ch == '\'' && shell != Shell::Cmd))
                && quote.is_none_or(|open| open == ch)
            {
                if quote.is_none() {
                    quote = Some(ch);
                } else if shell == Shell::Pwsh && chars.next_if(|(_, next)| *next == ch).is_some() {
                    value.push(ch);
                } else {
                    quote = None;
                }
            } else {
                value.push(ch);
            }
        }
        words.push(Word {
            range: start..chars.peek().map_or(input.len(), |(index, _)| *index),
            value,
            boundary: false,
            redirect_path: false,
        });
    }
    words
}

struct Line<'a> {
    original: &'a str,
    range: Range<usize>,
    prefix: String,
    previous: Vec<&'a Word>,
    following: Vec<&'a Word>,
}

impl<'a> Line<'a> {
    fn parse(input: &'a str, cursor: usize, words: &'a [Word], shell: Shell) -> Self {
        let start = words
            .iter()
            .rposition(|word| word.boundary && word.range.end <= cursor)
            .map_or(0, |index| index + 1);
        let segment = &words[start..];
        let range = segment
            .iter()
            .take_while(|word| !word.boundary)
            .find(|word| word.range.start <= cursor && cursor <= word.range.end)
            .map_or(cursor..cursor, |word| word.range.clone());
        let previous = segment
            .iter()
            .take_while(|word| !word.boundary && word.range.end <= range.start)
            .collect();
        let following = segment
            .iter()
            .take_while(|word| !word.boundary)
            .filter(|word| word.range.start >= range.end && word.range.start > cursor)
            .collect();
        let prefix = self::words(&input[..cursor], range.start, shell)
            .pop()
            .map_or_else(String::new, |word| word.value);
        Self {
            original: &input[range.clone()],
            range,
            prefix,
            previous,
            following,
        }
    }

    fn options_ended(&self) -> bool {
        self.previous.iter().any(|word| word.value == "--")
    }

    fn attached_value(&self) -> Option<(&str, Self)> {
        let (flag, value) = self.original.split_once('=')?;
        let (typed_flag, prefix) = self.prefix.split_once('=')?;
        if !flag.starts_with("--") || flag != typed_flag {
            return None;
        }
        Some((
            flag,
            Self {
                original: value,
                range: self.range.start + flag.len() + 1..self.range.end,
                prefix: prefix.into(),
                previous: self.previous.clone(),
                following: self.following.clone(),
            },
        ))
    }

    fn used_option(&self, name: &str, ignore_case: bool) -> bool {
        // Config overrides and verbosity can be specified repeatedly.
        if matches!(name, "-c" | "-C" | "-v" | "--verbose" | "-q" | "--quiet") {
            return false;
        }
        let name = name.trim_end_matches('=');
        self.previous
            .iter()
            .chain(&self.following)
            .take_while(|word| word.value != "--")
            .filter(|word| word.value.starts_with(['-', '/']))
            .any(|word| {
                let given = word
                    .value
                    .split_once('=')
                    .map_or(word.value.as_str(), |(flag, _)| flag);
                given == name || ignore_case && given.eq_ignore_ascii_case(name)
            })
    }

    fn explicit_path(&self, shell: Shell) -> bool {
        self.prefix.chars().any(|ch| shell.separator(ch))
            || (shell.windows_syntax() && self.prefix.as_bytes().get(1) == Some(&b':'))
    }
}

struct Candidate {
    item: Suggestion,
    ignore_case: bool,
    rank: usize,
}

struct Candidates<'a> {
    line: &'a Line<'a>,
    context: &'a Context,
    insertion: Shell,
    vi: bool,
    entries: Vec<Candidate>,
}

impl<'a> Candidates<'a> {
    fn for_line<'b>(&self, line: &'b Line<'b>) -> Candidates<'b>
    where
        'a: 'b,
    {
        Candidates {
            line,
            context: self.context,
            insertion: self.insertion,
            vi: self.vi,
            entries: Vec::new(),
        }
    }

    fn add(
        &mut self,
        matched: &str,
        text: String,
        description: [&str; 2],
        kind: Kind,
        ignore_case: bool,
    ) {
        if text == self.line.original
            || kind == Kind::Option && self.line.used_option(matched, ignore_case)
        {
            return;
        }
        let rank = if matched.starts_with(&self.line.prefix) {
            0
        } else if ignore_case
            && matched
                .to_lowercase()
                .starts_with(&self.line.prefix.to_lowercase())
        {
            1
        } else if matches!(kind, Kind::Command | Kind::Subcommand) {
            let Some(score) = fuzzy_score(matched, &self.line.prefix) else {
                return;
            };
            2 + score
        } else {
            return;
        };
        self.entries.push(Candidate {
            rank,
            item: Suggestion {
                text,
                label: matched.into(),
                kind,
                description: description[usize::from(self.vi)].into(),
                source: Source::Native,
                insert_range: self.line.range.clone(),
            },
            ignore_case,
        });
    }

    fn add_index(&mut self, entries: &[catalog::Item], kind: Kind, ignore_case: bool) {
        for (name, detail) in entries {
            self.add(name, (*name).into(), *detail, kind, ignore_case);
        }
    }

    fn add_literal(&mut self, name: &str, detail: [&str; 2], shell: Shell, kind: Kind) {
        if let Some(text) = literal(self.insertion, name, self.line.original) {
            self.add(
                name,
                text,
                detail,
                kind,
                shell.windows_syntax() && kind == Kind::Path,
            );
        }
    }

    fn ranked(mut self) -> Vec<Suggestion> {
        if self.entries.iter().any(|candidate| candidate.rank < 2) {
            self.entries.retain(|candidate| candidate.rank < 2);
        }
        self.entries.sort_by_key(|candidate| candidate.rank);
        let mut results = Vec::<Candidate>::new();
        for candidate in self.entries {
            if results.iter().any(|item| {
                if candidate.ignore_case || item.ignore_case {
                    item.item.text.eq_ignore_ascii_case(&candidate.item.text)
                } else {
                    item.item.text == candidate.item.text
                }
            }) {
                continue;
            }
            results.push(candidate);
            if results.len() == LIMIT {
                break;
            }
        }
        results
            .into_iter()
            .map(|candidate| candidate.item)
            .collect()
    }
}

pub(super) struct Completer {
    cache: RefCell<Cache>,
}

impl Completer {
    pub(super) fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
        }
    }

    pub(super) fn wsl_profiles(&self) -> Vec<String> {
        if !cfg!(windows) {
            return Vec::new();
        }
        self.cache
            .borrow_mut()
            .get(Key::WslDistributions)
            .iter()
            .filter(|entry| crate::cli::native::is_interactive_distribution(&entry.name))
            .map(|entry| format!("wsl:{}", entry.name))
            .collect()
    }

    pub(super) fn reset_request(&self) {
        self.cache.borrow_mut().reset_request();
    }

    pub(super) fn metadata_ready(&self) -> bool {
        self.cache.borrow_mut().metadata_ready()
    }

    pub(super) fn metadata_pending(&self) -> bool {
        self.cache.borrow().metadata_pending()
    }

    pub(super) fn suggestions(
        &self,
        input: &str,
        cursor: usize,
        vi: bool,
        context: &Context,
    ) -> Vec<Suggestion> {
        let host = context.shell;
        let host_words = words(input, 0, host);
        let host_line = Line::parse(input, cursor, &host_words, host);
        if let Some(payload) = bash_payload(&host_line, cursor, host) {
            let shell = if cfg!(windows) {
                Shell::GitBash
            } else {
                Shell::Bash
            };
            let inner = &input[..payload.end];
            let words = words(inner, payload.start, shell);
            let line = Line::parse(inner, cursor, &words, shell);
            let mut choices = self.complete(&line, shell, shell, vi, context);
            for item in &mut choices {
                item.text = if host == Shell::Pwsh {
                    item.text.replace('\'', "''")
                } else {
                    item.text.replace('\'', "'\\''")
                };
            }
            return choices;
        }
        if let Some((start, shell)) = nested_shell(&host_line, host) {
            let insertion = if shell == Shell::Wsl { host } else { shell };
            let nested_words = words(input, start, insertion);
            let line = Line::parse(input, cursor, &nested_words, insertion);
            self.complete(&line, shell, insertion, vi, context)
        } else {
            self.complete(&host_line, host, host, vi, context)
        }
    }

    fn complete(
        &self,
        line: &Line<'_>,
        shell: Shell,
        insertion: Shell,
        vi: bool,
        context: &Context,
    ) -> Vec<Suggestion> {
        let mut choices = Candidates {
            line,
            context,
            insertion,
            vi,
            entries: Vec::new(),
        };
        let Some(first) = line.previous.first() else {
            self.commands(&mut choices, shell);
            return choices.ranked();
        };
        let executable = first
            .value
            .rsplit(|ch| shell.separator(ch))
            .next()
            .unwrap_or(&first.value);
        let command = if shell.windows_syntax() {
            executable.to_ascii_lowercase()
        } else {
            executable.into()
        };
        if line.previous.last().is_some_and(|word| word.redirect_path) {
            self.paths(&mut choices, shell, false, false);
            return choices.ranked();
        }
        if matches!(command.as_str(), "git" | "git.exe") {
            self.git(&mut choices, shell);
            return choices.ranked();
        }

        let line = choices.line;
        let prefix = &line.prefix;
        if matches!(shell, Shell::Bash | Shell::GitBash)
            && prefix.starts_with('$')
            && !line.original.starts_with('\'')
            && line.original.trim_matches('"').bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'{' | b'}')
            })
        {
            let braced = prefix.as_bytes().get(1) == Some(&b'{');
            for (name, _) in env::vars_os().take(512) {
                if let Some(name) = name.to_str()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    let value = if braced {
                        format!("{}{{{name}}}", '$')
                    } else {
                        format!("{}{name}", '$')
                    };
                    let text = if line.original.starts_with('"') {
                        format!("\"{value}\"")
                    } else {
                        value.clone()
                    };
                    choices.add(
                        &value,
                        text,
                        ["Environment variable", "Biến môi trường"],
                        Kind::Value,
                        false,
                    );
                }
            }
            return choices.ranked();
        }
        match shell {
            Shell::Pwsh | Shell::Cmd if matches!(command.as_str(), "cmd" | "cmd.exe") => {
                choices.add_index(catalog::CMD_FLAGS, Kind::Option, true);
            }
            Shell::Pwsh | Shell::Cmd
                if matches!(command.as_str(), "wsl" | "wsl.exe") && cfg!(windows) =>
            {
                self.wsl_host(&mut choices, shell);
            }
            Shell::Pwsh
                if matches!(command.as_str(), "findstr" | "findstr.exe")
                    && prefix.starts_with('/') =>
            {
                choices.add_index(catalog::cmd_options("findstr"), Kind::Option, true);
            }
            Shell::Pwsh => {
                let command = catalog::ps_command(&command);
                if prefix.starts_with('-') && !line.options_ended() {
                    choices.add_index(catalog::powershell_options(command), Kind::Option, true);
                    self.metadata(
                        &mut choices,
                        shell,
                        Key::PowerShellParameters(command.into()),
                        Kind::Option,
                        ["PowerShell parameter", "Tham số PowerShell"],
                    );
                } else if !line.options_ended()
                    && let Some(parameter) = line.previous.last().map(|word| word.value.as_str())
                    && parameter.starts_with('-')
                    && !matches!(
                        parameter.to_ascii_lowercase().as_str(),
                        "-path"
                            | "-literalpath"
                            | "-destination"
                            | "-recurse"
                            | "-force"
                            | "-file"
                            | "-directory"
                    )
                {
                    self.metadata(
                        &mut choices,
                        shell,
                        Key::PowerShellValues {
                            command: command.into(),
                            parameter: parameter.into(),
                        },
                        Kind::Value,
                        ["PowerShell parameter value", "Giá trị tham số PowerShell"],
                    );
                } else if line.explicit_path(shell)
                    || (matches!(
                        command,
                        "get-childitem"
                            | "get-content"
                            | "set-location"
                            | "test-path"
                            | "resolve-path"
                            | "copy-item"
                            | "move-item"
                            | "remove-item"
                            | "new-item"
                            | "rename-item"
                    ) && !line.previous.last().is_some_and(|word| {
                        word.value.starts_with('-')
                            && !matches!(
                                word.value.to_ascii_lowercase().as_str(),
                                "-path"
                                    | "-literalpath"
                                    | "-destination"
                                    | "-recurse"
                                    | "-force"
                                    | "-file"
                                    | "-directory"
                            )
                    }))
                {
                    self.paths(&mut choices, shell, command == "set-location", false);
                }
            }
            Shell::Cmd => {
                let command = command.strip_suffix(".exe").unwrap_or(&command);
                if prefix.starts_with('/') && !prefix[1..].contains(['/', '\\']) {
                    choices.add_index(catalog::cmd_options(command), Kind::Option, true);
                } else if line.explicit_path(shell)
                    || matches!(
                        command,
                        "dir"
                            | "cd"
                            | "chdir"
                            | "type"
                            | "copy"
                            | "move"
                            | "del"
                            | "findstr"
                            | "mkdir"
                            | "rmdir"
                    )
                {
                    self.paths(
                        &mut choices,
                        shell,
                        matches!(command, "cd" | "chdir" | "rmdir"),
                        false,
                    );
                }
            }
            Shell::Bash | Shell::GitBash | Shell::Wsl => {
                if prefix.starts_with('-') && !line.options_ended() {
                    let options = if shell == Shell::GitBash {
                        catalog::bash_options(&command)
                    } else {
                        catalog::bash_portable_options(&command)
                    };
                    choices.add_index(options, Kind::Option, false);
                } else if line.explicit_path(shell)
                    || matches!(
                        command.as_str(),
                        "ls" | "cd"
                            | "cat"
                            | "cp"
                            | "mv"
                            | "rm"
                            | "mkdir"
                            | "rmdir"
                            | "head"
                            | "tail"
                            | "less"
                            | "source"
                            | "find"
                    )
                    || line.previous.len() >= 2 && command == "grep"
                {
                    self.paths(
                        &mut choices,
                        shell,
                        matches!(command.as_str(), "cd" | "rmdir"),
                        false,
                    );
                }
            }
        }
        choices.ranked()
    }

    fn commands(&self, choices: &mut Candidates<'_>, shell: Shell) {
        if choices.line.explicit_path(shell) {
            self.paths(choices, shell, false, false);
            return;
        }
        choices.add_index(&[catalog::GIT], Kind::Command, shell.windows_syntax());
        match shell {
            Shell::Pwsh => {
                choices.add_index(catalog::POWERSHELL, Kind::Command, true);
                self.metadata(
                    choices,
                    shell,
                    Key::PowerShellCommands,
                    Kind::Command,
                    ["Installed PowerShell command", "Lệnh PowerShell đã cài"],
                );
                self.windows_executables(choices, shell);
                choices.add_index(
                    &[
                        ("cmd", ["Windows Command Prompt", "Dấu nhắc lệnh Windows"]),
                        (
                            "wsl",
                            [
                                "Windows Subsystem for Linux",
                                "Hệ thống con Windows dành cho Linux",
                            ],
                        ),
                        ("pwsh", ["Start PowerShell", "Mở PowerShell"]),
                        (
                            "findstr",
                            ["Search text for patterns", "Tìm mẫu trong văn bản"],
                        ),
                        (
                            "where.exe",
                            ["Locate matching files", "Tìm các tệp phù hợp"],
                        ),
                    ],
                    Kind::Command,
                    true,
                );
            }
            Shell::Cmd => {
                choices.add_index(catalog::CMD, Kind::Command, true);
                self.windows_executables(choices, shell);
            }
            Shell::Bash | Shell::GitBash | Shell::Wsl => {
                choices.add_index(catalog::BASH, Kind::Command, false);
                if shell != Shell::Wsl {
                    self.metadata(
                        choices,
                        shell,
                        Key::BashCommands,
                        Kind::Command,
                        ["Bash command", "Lệnh Bash"],
                    );
                }
                if shell == Shell::Bash
                    && !choices.line.prefix.is_empty()
                    && let (Some(path), Some(cwd)) =
                        (env::var_os("PATH"), choices.context.directory.clone())
                {
                    for entry in self
                        .cache
                        .borrow_mut()
                        .get(Key::Executables { path, cwd })
                        .iter()
                    {
                        choices.add_literal(
                            &entry.name,
                            ["Executable on PATH", "Tệp thực thi trong PATH"],
                            shell,
                            Kind::Command,
                        );
                    }
                }
            }
        }
    }

    fn windows_executables(&self, choices: &mut Candidates<'_>, shell: Shell) {
        if !choices.line.prefix.is_empty()
            && let (Some(path), Some(cwd)) =
                (env::var_os("PATH"), choices.context.directory.clone())
        {
            self.metadata(
                choices,
                shell,
                Key::CmdExecutables { path, cwd },
                Kind::Command,
                ["Executable on PATH", "Tệp thực thi trong PATH"],
            );
        }
    }

    fn metadata(
        &self,
        choices: &mut Candidates<'_>,
        shell: Shell,
        key: Key,
        kind: Kind,
        detail: [&str; 2],
    ) {
        for entry in self.cache.borrow_mut().get(key).iter() {
            if kind == Kind::Value {
                choices.add_literal(&entry.name, detail, shell, kind);
            } else {
                choices.add(
                    &entry.name,
                    entry.name.clone(),
                    detail,
                    kind,
                    shell.windows_syntax(),
                );
            }
        }
    }

    fn wsl_host(&self, choices: &mut Candidates<'_>, shell: Shell) {
        let previous = choices.line.previous.last().map(|word| word.value.as_str());
        if matches!(
            previous,
            Some(
                "-d" | "--distribution"
                    | "-s"
                    | "--set-default"
                    | "-t"
                    | "--terminate"
                    | "--unregister"
                    | "--set-version"
                    | "--export"
            )
        ) {
            self.metadata(
                choices,
                shell,
                Key::WslDistributions,
                Kind::Value,
                ["Installed WSL distribution", "Bản phân phối WSL đã cài"],
            );
        } else if !matches!(previous, Some("-u" | "--user" | "--cd")) {
            choices.add_index(catalog::WSL_FLAGS, Kind::Option, false);
        }
    }

    fn git(&self, choices: &mut Candidates<'_>, shell: Shell) {
        let line = choices.line;
        let mut arguments = line.previous.iter().skip(1);
        let mut cwd = choices.context.directory.clone();
        let mut explicit_git_dir = false;
        let subcommand = loop {
            let Some(word) = arguments.next().map(|word| word.value.as_str()) else {
                if line.prefix.starts_with('-') {
                    choices.add_index(catalog::GIT_FLAGS, Kind::Option, false);
                } else {
                    choices.add_index(catalog::GIT_ACTIONS, Kind::Subcommand, false);
                    self.git_options(choices, shell, Key::GitCommands);
                }
                return;
            };
            match word {
                "-C" => {
                    let Some(path) = arguments.next() else {
                        self.path_from(choices, shell, true, true, cwd);
                        return;
                    };
                    if !path.value.is_empty() {
                        cwd = filesystem_path(shell, &path.value, cwd);
                    }
                }
                "-c" | "--config-env" | "--git-dir" | "--work-tree" | "--namespace" => {
                    explicit_git_dir |= matches!(word, "--git-dir" | "--work-tree" | "--namespace");
                    if arguments.next().is_none() {
                        match word {
                            "--git-dir" | "--work-tree" => {
                                self.path_from(choices, shell, true, true, cwd)
                            }
                            "-c" | "--config-env" => self.git_config_keys(choices, shell),
                            _ => {}
                        }
                        return;
                    }
                }
                flag if flag.starts_with('-') => {
                    explicit_git_dir |= ["--git-dir=", "--work-tree=", "--namespace="]
                        .iter()
                        .any(|prefix| flag.starts_with(prefix));
                }
                command => break command,
            }
        };
        let arguments = arguments
            .map(|word| word.value.as_str())
            .collect::<Vec<_>>();
        let action = arguments
            .iter()
            .copied()
            .find(|word| !word.starts_with('-'));
        if !line.options_ended() {
            if let Some((flag, value)) = line.attached_value() {
                let mut values = choices.for_line(&value);
                self.git_values(&mut values, shell, subcommand, flag, cwd, explicit_git_dir);
                choices.entries.extend(values.entries);
                return;
            }
            if let Some(flag) = arguments.last().copied().filter(|flag| {
                matches!(
                    *flag,
                    "--color"
                        | "--format"
                        | "--sort"
                        | "--author"
                        | "--date"
                        | "--source"
                        | "--onto"
                        | "--strategy"
                        | "-s"
                        | "-m"
                        | "--message"
                        | "--output"
                        | "--prefix"
                )
            }) {
                self.git_values(choices, shell, subcommand, flag, cwd, explicit_git_dir);
                return;
            }
            if line.prefix.starts_with('-') {
                choices.add_index(catalog::git_options(subcommand), Kind::Option, false);
                if shell != Shell::Wsl {
                    let subject = if matches!(
                        subcommand,
                        "stash"
                            | "worktree"
                            | "remote"
                            | "config"
                            | "submodule"
                            | "maintenance"
                            | "bisect"
                    ) && let Some(action) = action
                    {
                        format!("{subcommand} {action}")
                    } else {
                        subcommand.to_owned()
                    };
                    self.git_options(choices, shell, Key::GitOptions(subject));
                }
                return;
            }
        }

        if !line.options_ended() && action.is_none() {
            choices.add_index(catalog::git_actions(subcommand), Kind::Subcommand, false);
            if shell != Shell::Wsl
                && matches!(
                    subcommand,
                    "stash"
                        | "worktree"
                        | "remote"
                        | "config"
                        | "submodule"
                        | "maintenance"
                        | "bisect"
                )
            {
                self.git_options(choices, shell, Key::GitOptions(subcommand.to_owned()));
            }
        }
        let creating_branch = arguments.last().is_some_and(|word| {
            matches!(
                (subcommand, *word),
                ("switch", "-c" | "-C" | "--create" | "--force-create") | ("checkout", "-b" | "-B")
            )
        });
        let explicit_path = ["/", "./", "../", ".\\", "..\\"]
            .iter()
            .any(|start| line.prefix.starts_with(start))
            || shell.windows_syntax() && line.prefix.as_bytes().get(1) == Some(&b':');
        let accepts_paths = matches!(
            subcommand,
            "add"
                | "status"
                | "diff"
                | "restore"
                | "reset"
                | "log"
                | "show"
                | "checkout"
                | "grep"
                | "mv"
                | "rm"
                | "clone"
                | "blame"
                | "clean"
                | "ls-files"
                | "archive"
        );
        let accepts_refs = matches!(
            subcommand,
            "branch"
                | "switch"
                | "checkout"
                | "merge"
                | "rebase"
                | "cherry-pick"
                | "revert"
                | "reset"
                | "log"
                | "show"
                | "diff"
                | "blame"
                | "describe"
                | "tag"
                | "bisect"
                | "reflog"
                | "archive"
                | "rev-parse"
                | "rev-list"
                | "merge-base"
                | "show-ref"
                | "cat-file"
        );
        if creating_branch {
            return;
        }
        if accepts_paths && (!accepts_refs || line.options_ended() || explicit_path)
            || subcommand == "worktree" && action == Some("add") && arguments.last() == Some(&"add")
            || subcommand == "submodule" && action.is_some()
        {
            self.path_from(choices, shell, false, true, cwd.clone());
        }
        if shell == Shell::Wsl || line.options_ended() || explicit_path {
            return;
        }
        if subcommand == "config" && !matches!(action, Some("edit" | "list")) {
            self.git_config_keys(choices, shell);
        }
        if !explicit_git_dir && let Some(cwd) = cwd {
            use local::GitData;
            let kind = match (subcommand, action) {
                ("stash", Some("show" | "apply" | "pop" | "drop" | "branch")) => GitData::Stashes,
                ("worktree", Some("remove" | "move" | "lock" | "unlock" | "repair")) => {
                    GitData::Worktrees
                }
                ("worktree", Some("add")) if arguments.last() != Some(&"add") => GitData::Refs,
                ("fetch" | "pull" | "push", None) => GitData::Remotes,
                ("fetch" | "pull" | "push", Some(_)) => GitData::Refs,
                (
                    "remote",
                    Some(
                        "show" | "prune" | "rename" | "remove" | "get-url" | "set-url" | "set-head"
                        | "set-branches",
                    ),
                ) => GitData::Remotes,
                ("tag", _) => GitData::Tags,
                ("config", _) => GitData::Config,
                ("branch" | "switch", _) => GitData::Branches,
                _ if accepts_refs => GitData::Refs,
                _ => return,
            };
            self.git_repo_values(choices, shell, cwd, kind);
        }
    }

    fn git_options(&self, choices: &mut Candidates<'_>, shell: Shell, key: Key) {
        if shell == Shell::Wsl {
            return;
        }
        for entry in self.cache.borrow_mut().get(key).iter() {
            let name = entry.name.trim_end_matches('=');
            let (kind, detail) = if name.starts_with('-') {
                (
                    Kind::Option,
                    ["Installed Git option", "Tùy chọn của Git đã cài"],
                )
            } else {
                (
                    Kind::Subcommand,
                    ["Installed Git command", "Lệnh của Git đã cài"],
                )
            };
            choices.add(name, name.into(), detail, kind, false);
        }
    }

    fn git_values(
        &self,
        choices: &mut Candidates<'_>,
        shell: Shell,
        subcommand: &str,
        flag: &str,
        cwd: Option<PathBuf>,
        explicit_git_dir: bool,
    ) {
        if flag == "--output" {
            self.path_from(choices, shell, false, true, cwd);
        } else if matches!(
            (subcommand, flag),
            ("restore", "--source") | ("rebase", "--onto")
        ) && shell != Shell::Wsl
            && !explicit_git_dir
            && let Some(cwd) = cwd
        {
            self.git_repo_values(choices, shell, cwd, local::GitData::Refs);
        } else {
            let values: &[&str] = match flag {
                "--color" if subcommand != "config" => &["always", "auto", "never"],
                "--date" if matches!(subcommand, "log" | "show" | "reflog" | "blame") => &[
                    "default",
                    "iso",
                    "iso-strict",
                    "local",
                    "raw",
                    "relative",
                    "rfc",
                    "short",
                    "unix",
                ],
                "--strategy" | "-s"
                    if matches!(subcommand, "merge" | "rebase" | "cherry-pick" | "revert") =>
                {
                    &["octopus", "ort", "ours", "subtree"]
                }
                _ => &[],
            };
            for name in values {
                choices.add(
                    name,
                    (*name).into(),
                    ["Git option value", "Giá trị tùy chọn Git"],
                    Kind::Value,
                    false,
                );
            }
        }
    }

    fn git_config_keys(&self, choices: &mut Candidates<'_>, shell: Shell) {
        if shell == Shell::Wsl {
            return;
        }
        for entry in self.cache.borrow_mut().get(Key::GitConfigKeys).iter() {
            choices.add(
                &entry.name,
                entry.name.clone(),
                ["Git configuration key", "Khóa cấu hình Git"],
                Kind::Value,
                false,
            );
        }
    }

    fn git_repo_values(
        &self,
        choices: &mut Candidates<'_>,
        shell: Shell,
        cwd: PathBuf,
        kind: local::GitData,
    ) {
        use local::GitData;
        let detail = match kind {
            GitData::Branches => ["Git branch", "Nhánh Git"],
            GitData::Refs => ["Git reference", "Tham chiếu Git"],
            GitData::Tags => ["Git tag", "Thẻ Git"],
            GitData::Remotes => ["Git remote", "Remote Git"],
            GitData::Stashes => ["Git stash reference", "Tham chiếu stash Git"],
            GitData::Worktrees => ["Git worktree", "Cây làm việc Git"],
            GitData::Config => ["Repository configuration key", "Khóa cấu hình repo"],
        };
        for entry in self
            .cache
            .borrow_mut()
            .get(Key::GitRepository { cwd, kind })
            .iter()
        {
            choices.add_literal(&entry.name, detail, shell, Kind::Value);
        }
    }

    fn paths(&self, choices: &mut Candidates<'_>, shell: Shell, dirs_only: bool, git: bool) {
        self.path_from(
            choices,
            shell,
            dirs_only,
            git,
            choices.context.directory.clone(),
        );
    }

    fn path_from(
        &self,
        choices: &mut Candidates<'_>,
        shell: Shell,
        dirs_only: bool,
        git: bool,
        cwd: Option<PathBuf>,
    ) {
        let prefix = &choices.line.prefix;
        let split = prefix
            .rfind(|ch| shell.separator(ch))
            .map_or(0, |index| index + 1);
        if prefix.contains(['$', '*', '?']) || prefix.starts_with('~') {
            return;
        }
        let base = &prefix[..split];
        let separator = base
            .chars()
            .last()
            .filter(|ch| shell.separator(*ch))
            .unwrap_or(if shell.windows_syntax() { '\\' } else { '/' });
        let Some(parent) = filesystem_path(shell, base, cwd) else {
            return;
        };
        let literal_path = choices
            .line
            .previous
            .last()
            .is_some_and(|word| word.value.eq_ignore_ascii_case("-LiteralPath"));
        for entry in self.cache.borrow_mut().get(Key::Paths(parent)).iter() {
            if (dirs_only && !entry.directory)
                || (entry.name.starts_with('.') && !prefix[split..].starts_with('.'))
                || (git && (entry.name == ".git" || entry.name.contains(['*', '?', '[', ']'])))
                || (shell == Shell::Pwsh && !literal_path && entry.name.contains(['[', ']']))
            {
                continue;
            }
            let mut name = format!("{base}{}", entry.name);
            let insert = if base.is_empty() && entry.name.starts_with('-') {
                format!(".{separator}{name}")
            } else {
                name.clone()
            };
            if entry.directory {
                name.push(separator);
            }
            let insert = if entry.directory {
                format!("{insert}{separator}")
            } else {
                insert
            };
            if let Some(text) = literal(choices.insertion, &insert, choices.line.original) {
                choices.add(
                    &name,
                    text,
                    if entry.directory {
                        ["Directory", "Thư mục"]
                    } else {
                        ["File", "Tệp"]
                    },
                    Kind::Path,
                    shell.windows_syntax(),
                );
            }
        }
    }
}

// A simple outer single quote preserves raw byte offsets inside the script.
fn bash_payload(line: &Line<'_>, cursor: usize, host: Shell) -> Option<Range<usize>> {
    if host == Shell::Cmd
        || !matches!(
            line.previous.last().map(|word| word.value.as_str()),
            Some("-c" | "-lc" | "-ic" | "-ilc")
        )
    {
        return None;
    }
    let command = line
        .previous
        .first()?
        .value
        .rsplit(|ch| host.separator(ch))
        .next()?;
    if !matches!(command, "bash" | "bash.exe") {
        return None;
    }
    let body = line.original.strip_prefix('\'')?;
    let body = body.strip_suffix('\'').unwrap_or(body);
    let start = line.range.start + 1;
    let end = start + body.len();
    (!body.contains('\'') && (start..=end).contains(&cursor)).then_some(start..end)
}

fn nested_shell(line: &Line<'_>, host: Shell) -> Option<(usize, Shell)> {
    let wrapper = line.previous.first()?;
    let base = wrapper.value.to_ascii_lowercase();
    match (host, base.as_str()) {
        (Shell::Pwsh | Shell::Cmd, "cmd" | "cmd.exe") => line
            .previous
            .iter()
            .skip(1)
            .find(|word| {
                word.value.eq_ignore_ascii_case("/c") || word.value.eq_ignore_ascii_case("/k")
            })
            .map(|word| (word.range.end, Shell::Cmd)),
        (Shell::Pwsh | Shell::Cmd, "wsl" | "wsl.exe") if cfg!(windows) => {
            let mut args = line.previous.iter().skip(1);
            let mut offset = wrapper.range.end;
            while let Some(word) = args.next() {
                match word.value.as_str() {
                    "-e" | "--exec" | "--" => return Some((word.range.end, Shell::Wsl)),
                    "-d" | "--distribution" | "-u" | "--user" | "--cd" => {
                        offset = args.next()?.range.end;
                    }
                    value if value.starts_with('-') => return None,
                    _ => return Some((offset, Shell::Wsl)),
                }
            }
            (!line.prefix.starts_with('-')).then_some((offset, Shell::Wsl))
        }
        (shell, "command" | "exec" | "env") if shell.bash_syntax() => {
            let mut offset = wrapper.range.end;
            for word in line.previous.iter().skip(1) {
                if word.value == "--" || base == "env" && word.value.contains('=') {
                    offset = word.range.end;
                } else if word.value.starts_with('-') {
                    return None;
                } else {
                    break;
                }
            }
            Some((offset, shell))
        }
        _ => None,
    }
}

fn filesystem_path(shell: Shell, path: &str, cwd: Option<PathBuf>) -> Option<PathBuf> {
    if !path.is_empty() && !local::safe_text(path) {
        return None;
    }
    if shell == Shell::Wsl {
        return None;
    }
    if shell == Shell::GitBash && cfg!(windows) && path.starts_with('/') {
        let (drive, rest) = path[1..].split_once('/')?;
        if drive.len() == 1
            && drive.as_bytes()[0].is_ascii_alphabetic()
            && !rest.contains(['\\', ':'])
        {
            return Some(PathBuf::from(format!("{drive}:/")).join(rest));
        }
        return None;
    }
    if shell.windows_syntax()
        && path.as_bytes().get(1) == Some(&b':')
        && !path
            .as_bytes()
            .get(2)
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
    {
        return None;
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(cwd?.join(path))
    }
}

fn literal(shell: Shell, value: &str, original: &str) -> Option<String> {
    if !local::safe_text(value) {
        return None;
    }
    if shell == Shell::Cmd && value.contains(['%', '!', '"', '^', '&', '|', '<', '>']) {
        return None;
    }
    let quote = original
        .chars()
        .next()
        .filter(|ch| matches!(ch, '\'' | '"'));
    let simple = value.chars().all(|ch| {
        ch.is_alphanumeric()
            || matches!(ch, '_' | '-' | '.' | '/')
            || (shell.windows_syntax() && matches!(ch, '\\' | ':'))
    });
    if simple && quote.is_none() {
        return Some(value.into());
    }
    Some(match shell {
        Shell::Pwsh if quote == Some('"') || value.contains(['\u{2018}', '\u{2019}']) => {
            let mut escaped = String::from("\"");
            for ch in value.chars() {
                if matches!(ch, '$' | '"' | '\u{60}' | '\u{201c}' | '\u{201d}') {
                    escaped.push('\u{60}');
                }
                escaped.push(ch);
            }
            escaped.push('"');
            escaped
        }
        Shell::Pwsh => format!("'{}'", value.replace('\'', "''")),
        Shell::Cmd => format!("\"{value}\""),
        Shell::Bash | Shell::GitBash | Shell::Wsl => format!("'{}'", value.replace('\'', "'\\''")),
    })
}
