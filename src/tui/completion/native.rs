mod catalog;
mod local;

use super::{Kind, LIMIT, Source, Suggestion};
use local::{Cache, Key};
use std::{cell::RefCell, env, ops::Range, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shell {
    Pwsh,
    Cmd,
    Bash,
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

    fn separator(self, ch: char) -> bool {
        ch == '/' || (self.windows_syntax() && ch == '\\')
    }

    fn escaping(self, ch: char, quote: Option<char>) -> bool {
        match self {
            Self::Pwsh | Self::Wsl => ch == '\u{60}' && quote != Some('\''),
            Self::Cmd => ch == '^' && quote.is_none(),
            Self::Bash => ch == '\\' && quote != Some('\''),
        }
    }
}

struct Word {
    range: Range<usize>,
    value: String,
    boundary: bool,
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
            });
            continue;
        }
        let mut value = String::new();
        let mut quote = None;
        while let Some((_, ch)) = chars.peek().copied() {
            if quote.is_none() && (ch.is_whitespace() || boundary(ch)) {
                break;
            }
            chars.next();
            if shell.escaping(ch, quote) {
                if let Some((_, next)) = chars.next() {
                    if shell == Shell::Bash
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
                } else if matches!(shell, Shell::Pwsh | Shell::Wsl)
                    && chars.next_if(|(_, next)| *next == ch).is_some()
                {
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
        });
    }
    words
}

struct Line<'a> {
    original: &'a str,
    range: Range<usize>,
    prefix: String,
    previous: Vec<&'a Word>,
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
        let prefix = self::words(&input[..cursor], range.start, shell)
            .pop()
            .map_or_else(String::new, |word| word.value);
        Self {
            original: &input[range.clone()],
            range,
            prefix,
            previous,
        }
    }

    fn options_ended(&self) -> bool {
        self.previous.iter().any(|word| word.value == "--")
    }

    fn explicit_path(&self, shell: Shell) -> bool {
        self.prefix.chars().any(|ch| shell.separator(ch))
            || (shell.windows_syntax() && self.prefix.as_bytes().get(1) == Some(&b':'))
    }
}

struct Candidate {
    matched: String,
    item: Suggestion,
    ignore_case: bool,
}

struct Candidates<'a> {
    line: &'a Line<'a>,
    vi: bool,
    entries: Vec<Candidate>,
}

impl<'a> Candidates<'a> {
    fn add(
        &mut self,
        matched: &str,
        text: String,
        description: [&str; 2],
        kind: Kind,
        ignore_case: bool,
    ) {
        let exact = matched.starts_with(&self.line.prefix);
        if text == self.line.original
            || !(exact
                || ignore_case
                    && matched
                        .to_lowercase()
                        .starts_with(&self.line.prefix.to_lowercase()))
        {
            return;
        }
        self.entries.push(Candidate {
            matched: matched.into(),
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
        if let Some(text) = literal(shell, name, self.line.original) {
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
        self.entries
            .sort_by_key(|candidate| !candidate.matched.starts_with(&self.line.prefix));
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

    pub(super) fn reset_request(&self) {
        self.cache.borrow_mut().reset_request();
    }

    pub(super) fn metadata_ready(&self) -> bool {
        self.cache.borrow_mut().metadata_ready()
    }

    pub(super) fn metadata_pending(&self) -> bool {
        self.cache.borrow().metadata_pending()
    }

    pub(super) fn suggestions(&self, input: &str, cursor: usize, vi: bool) -> Vec<Suggestion> {
        let host = Shell::host();
        let host_words = words(input, 0, host);
        let host_line = Line::parse(input, cursor, &host_words, host);
        if let Some((start, shell)) = nested_shell(&host_line, host) {
            let nested_words = words(input, start, shell);
            let line = Line::parse(input, cursor, &nested_words, shell);
            self.complete(&line, shell, vi)
        } else {
            self.complete(&host_line, host, vi)
        }
    }

    fn complete(&self, line: &Line<'_>, shell: Shell, vi: bool) -> Vec<Suggestion> {
        let mut choices = Candidates {
            line,
            vi,
            entries: Vec::new(),
        };
        let Some(first) = line.previous.first() else {
            self.commands(&mut choices, shell);
            return choices.ranked();
        };
        let command = match shell {
            Shell::Pwsh | Shell::Cmd => first.value.to_ascii_lowercase(),
            Shell::Bash | Shell::Wsl => first.value.clone(),
        };
        if matches!(command.as_str(), "git" | "git.exe") {
            self.git(&mut choices, shell);
            return choices.ranked();
        }

        let line = choices.line;
        let prefix = &line.prefix;
        match shell {
            Shell::Pwsh if matches!(command.as_str(), "cmd" | "cmd.exe") => {
                choices.add_index(catalog::CMD_FLAGS, Kind::Option, true);
            }
            Shell::Pwsh if matches!(command.as_str(), "wsl" | "wsl.exe") => {
                choices.add_index(catalog::WSL_FLAGS, Kind::Option, false);
            }
            Shell::Pwsh
                if matches!(command.as_str(), "findstr" | "findstr.exe")
                    && prefix.starts_with('/') =>
            {
                choices.add_index(catalog::cmd_options("findstr"), Kind::Option, true);
            }
            Shell::Pwsh => {
                let command = catalog::ps_command(&command);
                if prefix.starts_with('-') {
                    choices.add_index(catalog::powershell_options(command), Kind::Option, true);
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
            Shell::Bash | Shell::Wsl => {
                if prefix.starts_with('-') && !line.options_ended() {
                    choices.add_index(catalog::bash_options(&command), Kind::Option, false);
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
            Shell::Cmd => choices.add_index(catalog::CMD, Kind::Command, true),
            Shell::Bash | Shell::Wsl => {
                choices.add_index(catalog::BASH, Kind::Command, false);
                if shell == Shell::Bash
                    && !choices.line.prefix.is_empty()
                    && let (Some(path), Ok(cwd)) = (env::var_os("PATH"), env::current_dir())
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

    fn git(&self, choices: &mut Candidates<'_>, shell: Shell) {
        let line = choices.line;
        let mut arguments = line.previous.iter().skip(1);
        let mut cwd = env::current_dir().ok();
        let mut explicit_git_dir = false;
        let subcommand = loop {
            let Some(word) = arguments.next().map(|word| word.value.as_str()) else {
                if line.prefix.starts_with('-') {
                    choices.add_index(catalog::GIT_FLAGS, Kind::Option, false);
                } else {
                    choices.add_index(catalog::GIT_ACTIONS, Kind::Subcommand, false);
                }
                return;
            };
            match word {
                "-C" => {
                    let Some(path) = arguments.next() else {
                        self.path_from(choices, shell, true, true, cwd);
                        return;
                    };
                    cwd = filesystem_path(shell, &path.value, cwd);
                }
                "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                    explicit_git_dir |= word == "--git-dir";
                    if arguments.next().is_none() {
                        if matches!(word, "--git-dir" | "--work-tree") {
                            self.path_from(choices, shell, true, true, cwd);
                        }
                        return;
                    }
                }
                flag if flag.starts_with('-') => {
                    explicit_git_dir |= flag.starts_with("--git-dir=");
                }
                command => break command,
            }
        };
        let arguments = arguments
            .map(|word| word.value.as_str())
            .collect::<Vec<_>>();
        if line.prefix.starts_with('-') && !line.options_ended() {
            choices.add_index(catalog::git_options(subcommand), Kind::Option, false);
            return;
        }
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
        );
        let accepts_branches = matches!(
            subcommand,
            "switch"
                | "checkout"
                | "branch"
                | "merge"
                | "rebase"
                | "log"
                | "show"
                | "diff"
                | "reset"
        );
        let creating_branch = arguments.last().is_some_and(|word| {
            matches!(
                (subcommand, *word),
                ("switch", "-c" | "-C" | "--create" | "--force-create") | ("checkout", "-b" | "-B")
            )
        });
        if !line.options_ended() && arguments.iter().all(|word| word.starts_with('-')) {
            choices.add_index(catalog::git_actions(subcommand), Kind::Subcommand, false);
        }
        let explicit_path = ["/", "./", "../", ".\\", "..\\"]
            .iter()
            .any(|start| line.prefix.starts_with(start))
            || shell.windows_syntax() && line.prefix.as_bytes().get(1) == Some(&b':');
        if accepts_paths
            && (!accepts_branches || line.options_ended() || explicit_path)
            && !creating_branch
        {
            self.path_from(choices, shell, false, true, cwd);
        } else if accepts_branches
            && !line.options_ended()
            && !creating_branch
            && !explicit_git_dir
            && !arguments
                .last()
                .is_some_and(|word| matches!(*word, "--format" | "--sort"))
            && let Some(cwd) = cwd
        {
            for entry in self.cache.borrow_mut().get(Key::Branches(cwd)).iter() {
                choices.add_literal(
                    &entry.name,
                    ["Local Git branch", "Nhánh Git cục bộ"],
                    shell,
                    Kind::Value,
                );
            }
        }
    }

    fn paths(&self, choices: &mut Candidates<'_>, shell: Shell, dirs_only: bool, git: bool) {
        self.path_from(choices, shell, dirs_only, git, env::current_dir().ok());
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
            if let Some(text) = literal(shell, &insert, choices.line.original) {
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

fn nested_shell(line: &Line<'_>, host: Shell) -> Option<(usize, Shell)> {
    let wrapper = line.previous.first()?;
    let base = wrapper.value.to_ascii_lowercase();
    match (host, base.as_str()) {
        (Shell::Pwsh, "cmd" | "cmd.exe") => line
            .previous
            .iter()
            .skip(1)
            .find(|word| {
                word.value.eq_ignore_ascii_case("/c") || word.value.eq_ignore_ascii_case("/k")
            })
            .map(|word| (word.range.end, Shell::Cmd)),
        (Shell::Pwsh, "wsl" | "wsl.exe") => {
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
        _ => None,
    }
}

fn filesystem_path(shell: Shell, path: &str, cwd: Option<PathBuf>) -> Option<PathBuf> {
    if !path.is_empty() && !local::safe_text(path) {
        return None;
    }
    if shell == Shell::Wsl && cfg!(windows) {
        if let Some(mount) = path.strip_prefix("/mnt/") {
            let (drive, rest) = mount.split_once('/')?;
            if drive.len() == 1
                && drive.as_bytes()[0].is_ascii_alphabetic()
                && !rest.contains(['\\', ':'])
            {
                return Some(PathBuf::from(format!("{drive}:/")).join(rest));
            }
            return None;
        }
        if path.starts_with('/') || path.contains(['\\', ':']) {
            return None;
        }
    } else if shell.windows_syntax()
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
    // WSL options belong to the guest command. The TUI still sends the line
    // through the host PowerShell parser before launching wsl.exe.
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
        Shell::Pwsh | Shell::Wsl
            if quote == Some('"') || value.contains(['\u{2018}', '\u{2019}']) =>
        {
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
        Shell::Pwsh | Shell::Wsl => format!("'{}'", value.replace('\'', "''")),
        Shell::Cmd => format!("\"{value}\""),
        Shell::Bash => format!("'{}'", value.replace('\'', "'\\''")),
    })
}
