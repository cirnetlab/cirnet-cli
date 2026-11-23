use clap::{Arg, Command};
use std::{borrow::Cow, collections::HashSet, ops::Range, sync::OnceLock};

mod native;

const LIMIT: usize = 7;
const MAX_INPUT: usize = 8192;

pub(super) fn grammar() -> &'static Command {
    grammar_for(false)
}

fn grammar_for(vi: bool) -> &'static Command {
    static GRAMMAR: [OnceLock<Command>; 2] = [const { OnceLock::new() }; 2];
    GRAMMAR[usize::from(vi)].get_or_init(|| {
        let mut parser = crate::cli::app_for(vi);
        parser.build();
        parser
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Command,
    Subcommand,
    Option,
    Value,
    Path,
    History,
}

impl Kind {
    pub(super) fn label(self, vi: bool) -> &'static str {
        let labels = match self {
            Self::Command => ["command", "lệnh"],
            Self::Subcommand => ["action", "thao tác"],
            Self::Option => ["option", "tùy chọn"],
            Self::Value => ["value", "giá trị"],
            Self::Path => ["path", "đường dẫn"],
            Self::History => ["history", "lịch sử"],
        };
        labels[usize::from(vi)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Cli,
    Native,
    Menu,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Suggestion {
    pub(super) text: String,
    pub(super) insert_range: Range<usize>,
    pub(super) label: String,
    pub(super) kind: Kind,
    pub(super) description: String,
    pub(super) source: Source,
}

struct Token<'a> {
    range: Range<usize>,
    value: Cow<'a, str>,
}

fn tokens(input: &str) -> Result<Vec<Token<'_>>, &'static str> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some((start, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let mut quote = None;
        let mut decoded = None::<String>;
        let mut literal_start = start;
        while let Some((index, ch)) = chars.peek().copied() {
            if quote.is_none() && ch.is_whitespace() {
                break;
            }
            chars.next();
            if ch == '\\' && quote != Some('\'') {
                // Preserve Windows separators. Only a run immediately before
                // a double quote participates in Windows quote escaping.
                let mut count = 1;
                while chars.next_if(|(_, next)| *next == '\\').is_some() {
                    count += 1;
                }
                if let Some((quote_index, _)) = chars.next_if(|(_, next)| *next == '"') {
                    let value = decoded.get_or_insert_with(String::new);
                    value.push_str(&input[literal_start..index]);
                    value.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        value.push('"');
                    } else {
                        quote = if quote.is_some() { None } else { Some('"') };
                    }
                    literal_start = quote_index + 1;
                }
            } else if matches!(ch, '\'' | '"') && quote.is_none_or(|open| open == ch) {
                decoded
                    .get_or_insert_with(String::new)
                    .push_str(&input[literal_start..index]);
                quote = if quote.is_some() { None } else { Some(ch) };
                literal_start = index + 1;
            }
        }
        if quote.is_some() {
            return Err("Unclosed quote");
        }
        let end = chars.peek().map_or(input.len(), |(index, _)| *index);
        let value = decoded.map_or_else(
            || Cow::Borrowed(&input[start..end]),
            |mut value| {
                value.push_str(&input[literal_start..end]);
                Cow::Owned(value)
            },
        );
        tokens.push(Token {
            range: start..end,
            value,
        });
    }
    Ok(tokens)
}

pub(super) fn arguments(input: &str) -> Result<Vec<String>, &'static str> {
    Ok(tokens(input)?
        .into_iter()
        .map(|token| token.value.into_owned())
        .collect())
}

fn fuzzy_score(candidate: &str, prefix: &str) -> Option<usize> {
    if !(2..=24).contains(&prefix.len())
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return None;
    }
    let mut suffix = candidate.as_bytes();
    let mut gap = 0;
    for byte in prefix.bytes() {
        let index = suffix
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(&byte))?;
        gap += index;
        suffix = &suffix[index + 1..];
    }
    (gap <= 8).then_some(1 + gap)
}

pub(super) struct Completer {
    vi: bool,
    native: native::Completer,
    repositories: Vec<String>,
}

impl Completer {
    pub(super) fn new(vi: bool) -> Self {
        Self {
            vi,
            native: native::Completer::new(),
            repositories: crate::workspace::Repo::KEYS
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    pub(super) fn set_language(&mut self, vi: bool) {
        self.vi = vi;
    }

    pub(super) fn set_repository_values(&mut self, values: Vec<String>) {
        self.repositories = if values.is_empty() {
            crate::workspace::Repo::KEYS
                .into_iter()
                .map(str::to_owned)
                .collect()
        } else {
            values
        };
    }

    pub(super) fn metadata_ready(&self) -> bool {
        self.native.metadata_ready()
    }

    pub(super) fn metadata_pending(&self) -> bool {
        self.native.metadata_pending()
    }

    pub(super) fn suggestions(&self, input: &str, cursor: usize) -> Vec<Suggestion> {
        self.native.reset_request();
        if input.len() > MAX_INPUT || !input.is_char_boundary(cursor) {
            return Vec::new();
        }
        let first = input[..cursor]
            .split_whitespace()
            .next()
            .unwrap_or_default();
        let is_cnl = matches!(first, "cnl" | "cnl.exe" | "clear")
            || first.starts_with('-')
            || grammar().find_subcommand(first).is_some();
        if is_cnl || first.is_empty() {
            return self.cnl_suggestions(input, cursor);
        }
        let native = self.native.suggestions(input, cursor, self.vi);
        if !input[..cursor].trim_start().contains(char::is_whitespace) {
            let mut choices = self.cnl_suggestions(input, cursor);
            choices.extend(native);
            choices.sort_by_key(|item| {
                !item
                    .text
                    .starts_with(&input[item.insert_range.start..cursor])
            });
            let mut seen = HashSet::new();
            choices.retain(|item| seen.insert((item.text.clone(), item.insert_range.clone())));
            choices.truncate(LIMIT);
            choices
        } else {
            native
        }
    }

    fn cnl_suggestions(&self, input: &str, cursor: usize) -> Vec<Suggestion> {
        if input.len() > MAX_INPUT || !input.is_char_boundary(cursor) {
            return Vec::new();
        }
        let Ok(words) = tokens(input) else {
            return Vec::new();
        };
        let range = words
            .iter()
            .find(|word| word.range.start <= cursor && cursor <= word.range.end)
            .map_or(cursor..cursor, |word| word.range.clone());
        let mut previous = words
            .iter()
            .filter(|word| word.range.end <= range.start)
            .peekable();
        if previous
            .peek()
            .is_some_and(|word| matches!(word.value.as_ref(), "cnl" | "cnl.exe"))
        {
            previous.next();
        }
        let mut command = grammar_for(self.vi);
        let mut command_path = Vec::new();
        let mut value: Option<&Arg> = None;
        let mut positional = 0;
        let mut used_flags = Vec::new();
        for word in previous {
            if let Some(arg) = value.take()
                && !word.value.starts_with("--")
            {
                if arg.get_id() == "select" {
                    value = Some(arg);
                }
                continue;
            }
            if word.value == "--" {
                return Vec::new();
            }
            if let Some(flag) = word.value.strip_prefix("--") {
                used_flags.push(flag.split('=').next().unwrap_or(flag));
                let arg = command
                    .get_arguments()
                    .find(|arg| arg.get_long() == used_flags.last().copied());
                let Some(arg) = arg else { return Vec::new() };
                value = (!flag.contains('=') && arg.get_action().takes_values()).then_some(arg);
            } else if let Some(child) = command.find_subcommand(word.value.as_ref()) {
                command = child;
                command_path.push(child.get_name());
                positional = 0;
            } else {
                let found_positional = command.get_arguments().any(|arg| {
                    arg.get_index().is_some_and(|index| {
                        index == positional + 1
                            || index <= positional
                                && arg
                                    .get_num_args()
                                    .is_some_and(|range| range.max_values() > 1)
                    })
                });
                if !found_positional {
                    return Vec::new();
                }
                positional += 1;
            }
        }

        let mut candidates = Vec::new();
        let mut insert = range;
        let flag_prefix = input[insert.start..cursor].starts_with('-');
        let explicit_flag = cursor - insert.start > 2;
        let mut attached_value = false;
        if let Some((flag, _)) = input[insert.start..cursor]
            .strip_prefix("--")
            .and_then(|part| part.split_once('='))
        {
            value = command
                .get_arguments()
                .find(|arg| arg.get_long() == Some(flag));
            insert.start += flag.len() + 3;
            attached_value = true;
        }
        if let Some(arg) = value.filter(|_| !flag_prefix || attached_value) {
            self.values(
                &mut candidates,
                arg,
                &command_path,
                &input[insert.start..cursor],
            );
        } else {
            for child in command
                .get_subcommands()
                .filter(|child| !child.is_hide_set() && !flag_prefix && child.get_name() != "help")
            {
                let label = command_path
                    .iter()
                    .copied()
                    .chain([child.get_name()])
                    .collect::<Vec<_>>()
                    .join(" ");
                let description = crate::cli::catalog::detail(&label, self.vi)
                    .map(str::to_owned)
                    .or_else(|| child.get_about().map(ToString::to_string))
                    .unwrap_or_default();
                let kind = if command_path.is_empty() {
                    Kind::Command
                } else {
                    Kind::Subcommand
                };
                candidates.push((child.get_name().to_owned(), label, description, kind));
            }
            for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
                if let Some(flag) = arg.get_long() {
                    let repeatable = matches!(
                        arg.get_action(),
                        clap::ArgAction::Append | clap::ArgAction::Count
                    );
                    if (!repeatable && used_flags.contains(&flag))
                        || (arg.is_global_set() && !command_path.is_empty() && !explicit_flag)
                        || (arg.is_global_set() && !flag_prefix)
                        || (flag == "all"
                            && (used_flags.contains(&"repo")
                                || used_flags.contains(&"select")
                                || positional > 0))
                        || (matches!(flag, "repo" | "select") && used_flags.contains(&"all"))
                    {
                        continue;
                    }
                    let text = format!("--{flag}");
                    candidates.push((
                        text.clone(),
                        text,
                        arg.get_help().map(ToString::to_string).unwrap_or_default(),
                        Kind::Option,
                    ));
                } else if !flag_prefix
                    && !used_flags.contains(&"all")
                    && arg.get_index().is_some_and(|index| {
                        index == positional + 1
                            || index <= positional
                                && arg
                                    .get_num_args()
                                    .is_some_and(|range| range.max_values() > 1)
                    })
                {
                    self.values(
                        &mut candidates,
                        arg,
                        &command_path,
                        &input[insert.start..cursor],
                    );
                }
            }
        }
        let prefix = &input[insert.start..cursor];
        let original = &input[insert.clone()];
        let prefix_match = |text: &str, label: &str, kind: Kind| {
            text.starts_with(prefix)
                || kind == Kind::Value && label.starts_with(prefix.trim_start_matches('"'))
        };
        let has_prefix = candidates
            .iter()
            .any(|(candidate, label, _, kind)| prefix_match(candidate, label, *kind));
        let mut ranked = candidates
            .into_iter()
            .filter_map(|(text, label, description, kind)| {
                if text == original {
                    return None;
                }
                let score = if prefix_match(&text, &label, kind) {
                    0
                } else if !has_prefix && matches!(kind, Kind::Command | Kind::Subcommand) {
                    fuzzy_score(&text, prefix)?
                } else {
                    return None;
                };
                Some((
                    score,
                    Suggestion {
                        text,
                        label,
                        description,
                        kind,
                        source: Source::Cli,
                        insert_range: insert.clone(),
                    },
                ))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|(left_score, left), (right_score, right)| {
            fn priority(kind: Kind) -> u8 {
                u8::from(kind == Kind::Option)
            }
            left_score
                .cmp(right_score)
                .then_with(|| priority(left.kind).cmp(&priority(right.kind)))
                .then_with(|| left.text.cmp(&right.text))
        });
        ranked.dedup_by(|(_, left), (_, right)| left.text == right.text);
        ranked
            .into_iter()
            .take(LIMIT)
            .map(|(_, item)| item)
            .collect()
    }

    fn values(
        &self,
        candidates: &mut Vec<(String, String, String, Kind)>,
        arg: &Arg,
        commands: &[&str],
        prefix: &str,
    ) {
        if commands.first() == Some(&"repos")
            && matches!(arg.get_id().as_str(), "target" | "targets" | "select")
        {
            for name in self
                .repositories
                .iter()
                .filter(|name| name.starts_with(prefix.trim_start_matches('"')))
                .take(LIMIT)
            {
                let text = if name.contains(char::is_whitespace) || name.contains(['\'', '"']) {
                    format!("\"{}\"", name.replace('"', "\\\""))
                } else {
                    name.clone()
                };
                candidates.push((
                    text,
                    name.clone(),
                    arg.get_help().map(ToString::to_string).unwrap_or_default(),
                    Kind::Value,
                ));
            }
            return;
        }
        for value in arg
            .get_value_parser()
            .possible_values()
            .into_iter()
            .flatten()
            .filter(|value| !value.is_hide_set())
        {
            let name = value.get_name().to_owned();
            candidates.push((
                name.clone(),
                name,
                value
                    .get_help()
                    .map_or_else(String::new, ToString::to_string),
                Kind::Value,
            ));
        }
    }
}

pub(crate) fn command() -> Command {
    Command::new("completion").arg(
        Arg::new("shell")
            .required(true)
            .value_parser(["powershell", "bash"]),
    )
}

pub(crate) fn query_command() -> Command {
    Command::new("__complete")
        .hide(true)
        .arg(
            Arg::new("cursor")
                .long("cursor")
                .required(true)
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(Arg::new("line").last(true).required(true))
}

pub(crate) fn emit(args: &clap::ArgMatches) -> anyhow::Result<()> {
    use std::io::Write;
    let script = match args.get_one::<String>("shell").map(String::as_str) {
        Some("powershell") => {
            r#"Register-ArgumentCompleter -Native -CommandName cnl,cnl.exe -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $line = $commandAst.Extent.Text
    $cursor = [Math]::Max($cursorPosition - $commandAst.Extent.StartOffset, 0)
    $line += ' ' * [Math]::Max(0, $cursor - $line.Length)
    $offset = [Text.Encoding]::UTF8.GetByteCount($line.Substring(0, $cursor))
    $prefix = if ($wordToComplete -match '^(--[^=]+=)') { $Matches[1] } else { '' }
    & cnl __complete --cursor $offset -- $line 2>$null | ForEach-Object {
        [Management.Automation.CompletionResult]::new("$prefix$_")
    }
}
"#
        }
        Some("bash") => {
            r#"_cnl_complete() {
    mapfile -t COMPREPLY < <(cnl __complete --cursor "$COMP_POINT" -- "$COMP_LINE" 2>/dev/null)
}
complete -F _cnl_complete cnl
"#
        }
        _ => unreachable!("shell is validated by Clap"),
    };
    Ok(std::io::stdout().lock().write_all(script.as_bytes())?)
}

pub(crate) fn query(args: &clap::ArgMatches) -> anyhow::Result<()> {
    use std::io::Write;
    let line = args
        .get_one::<String>("line")
        .expect("required command line");
    let cursor = *args.get_one::<usize>("cursor").expect("required cursor");
    let Ok(words) = tokens(line) else {
        return Ok(());
    };
    let Some(executable) = words.first() else {
        return Ok(());
    };
    let executable = if executable.value == "&" {
        words.get(1).unwrap_or(executable)
    } else {
        executable
    };
    let start = executable.range.end;
    if cursor >= start {
        let offset = cursor - start;
        let mut output = std::io::stdout().lock();
        let mut completer = Completer::new(false);
        if line
            .get(start..cursor)
            .is_some_and(|input| input.split_whitespace().any(|word| word == "repos"))
            && let Ok(workspace) = crate::workspace::Workspace::discover(None)
            && let Ok(values) = crate::repositories::completion_values(&workspace, args)
        {
            completer.set_repository_values(values);
        }
        for choice in completer.cnl_suggestions(&line[start..], offset) {
            writeln!(output, "{}", choice.text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, io,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn repository_selection_completes_aliases_across_multiple_selector_values() {
        let mut completer = Completer::new(false);
        completer.set_repository_values(vec!["alpha".into(), "beta".into(), "beta tools".into()]);
        for input in ["repos update --select alpha be", "repos update alpha be"] {
            let choices = completer.suggestions(input, input.len());
            assert!(choices.iter().any(|item| item.text == "beta"), "{input}");
            assert!(
                choices.iter().any(|item| item.text == "\"beta tools\""),
                "{input}"
            );
        }
        let input = "repos update --select alpha --dr";
        assert!(
            completer
                .suggestions(input, input.len())
                .iter()
                .any(|item| item.text == "--dry-run")
        );
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "cnl-completion-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn wait_for_metadata(completer: &Completer, input: &str, cursor: usize) -> Vec<Suggestion> {
        let mut choices = completer.suggestions(input, cursor);
        let deadline = Instant::now() + Duration::from_secs(3);
        while completer.metadata_pending() && Instant::now() < deadline {
            if completer.metadata_ready() {
                choices = completer.suggestions(input, cursor);
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(!completer.metadata_pending(), "completion worker timed out");
        choices
    }

    #[test]
    fn suppresses_unrelated_globals_consumed_options_and_conflicting_scopes() {
        let completer = Completer::new(false);
        let choices = |input: &str| {
            completer
                .suggestions(input, input.len())
                .into_iter()
                .map(|item| item.text)
                .collect::<Vec<_>>()
        };
        assert_eq!(choices("stop --"), ["--desktop", "--dry-run", "--help"]);
        assert_eq!(choices("stop --pr"), ["--profile"]);
        assert!(!choices("run --desktop interface --de").contains(&"--desktop".to_owned()));
        assert!(!choices("repos clone remote --a").contains(&"--all".to_owned()));
        assert!(choices("repos clone --all re").is_empty());
        assert!(choices("run --bogus ").is_empty());
    }

    #[test]
    fn native_git_completes_at_the_cursor_without_replacing_later_tokens() {
        let completer = Completer::new(false);
        let choices = completer.suggestions("git st --short", 6);
        assert_eq!(
            choices
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["status", "stash"],
        );
        assert!(choices.iter().all(|item| item.insert_range == (4..6)));
        assert_eq!(completer.suggestions("gi", 2)[0].text, "git");
        assert!(
            completer
                .suggestions("git checkout --", 15)
                .iter()
                .any(|item| item.text == "--detach")
        );
    }

    #[test]
    fn native_windows_shell_prefixes_route_to_their_own_syntax() {
        let completer = Completer::new(true);
        let contains = |line: &str, candidate| {
            completer
                .suggestions(line, line.len())
                .iter()
                .any(|item| item.text == candidate && !item.description.is_ascii())
        };
        if cfg!(windows) {
            assert!(contains("cmd /c dir /", "/B"));
            assert!(contains("wsl --exec ls --", "--all"));
            assert!(contains("Get-Ch", "Get-ChildItem"));
            assert!(contains("Get-ChildItem -", "-Filter"));
        } else {
            assert!(contains("ls --", "--all"));
        }
    }

    #[test]
    fn native_git_reads_loose_and_packed_local_worktree_branches() -> io::Result<()> {
        let root = TestDir::new()?;
        let worktree = root.0.join("checkout with spaces");
        fs::create_dir(&worktree)?;
        fs::write(worktree.join(".git"), "gitdir: ../git/worktrees/checkout\n")?;
        fs::create_dir_all(root.0.join("git/worktrees/checkout"))?;
        fs::write(root.0.join("git/worktrees/checkout/commondir"), "../..\n")?;
        fs::create_dir_all(root.0.join("git/refs/heads/feature"))?;
        fs::write(root.0.join("git/refs/heads/feature/local"), "")?;
        fs::write(
            root.0.join("git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n\
             0123456789abcdef0123456789abcdef01234567 refs/heads/feature/packed\n\
             0123456789abcdef0123456789abcdef01234567 refs/remotes/origin/feature/unwanted\n",
        )?;
        let input = format!("git -C \"{}\" switch feature/", worktree.display());
        let choices = wait_for_metadata(&Completer::new(false), &input, input.len());
        assert_eq!(
            choices
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["feature/local", "feature/packed"],
        );
        Ok(())
    }

    #[test]
    fn native_paths_complete_unicode_inside_incomplete_quotes() -> io::Result<()> {
        let root = TestDir::new()?;
        fs::write(root.0.join("Tệp dữ liệu.txt"), "")?;
        let path = root.0.join("Tệp d");
        let input = format!("git add \"{}", path.display());
        let before_end = wait_for_metadata(&Completer::new(true), &input, input.len());
        let [choice] = before_end.as_slice() else {
            panic!("expected one path, got {before_end:?}");
        };
        assert_eq!(
            choice.label,
            root.0.join("Tệp dữ liệu.txt").to_string_lossy()
        );
        assert_eq!(choice.kind, Kind::Path);
        assert_eq!(choice.description, "Tệp");
        let completed = format!("{}{}", &input[..choice.insert_range.start], choice.text);
        assert_eq!(arguments(&completed).unwrap()[2], choice.label);

        let edited = format!("{input}raft\" --force");
        let cursor = input.len();
        let after_edit = wait_for_metadata(&Completer::new(false), &edited, cursor);
        let [choice] = after_edit.as_slice() else {
            panic!("expected one path, got {after_edit:?}");
        };
        assert_eq!(&edited[choice.insert_range.end..], " --force");
        let inside_unicode = edited.find('ệ').unwrap() + 1;
        assert!(
            Completer::new(false)
                .suggestions(&edited, inside_unicode)
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn suggests_subcommands_and_parser_values_at_the_cursor() {
        let completion = Completer::new(false);
        for (input, expected) in [
            ("cnl repos d", "discover"),
            ("dev --desktop int", "interface"),
            ("run --desktop=int", "interface"),
            ("build --repo sim", "simulator"),
            ("repos update em", "embedded"),
        ] {
            let candidates = completion.suggestions(input, input.len());
            assert!(
                candidates.iter().any(|item| item.text == expected),
                "{input}: {candidates:?}"
            );
        }
    }

    #[test]
    fn keeps_windows_paths_and_empty_quoted_arguments() {
        assert_eq!(
            arguments(r#"run --receipt "D:\CIRNET LAB\assembly-result.json" ''"#).unwrap(),
            [
                "run",
                "--receipt",
                r"D:\CIRNET LAB\assembly-result.json",
                ""
            ]
        );
    }
}
