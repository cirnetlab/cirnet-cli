// Derived from CIRNET Rules CNR, commit 646afa38516bbfd276b0aecb7a138ab427df5fa9 (GPL-3.0).
use clap::{Arg, Command};
use std::{borrow::Cow, ops::Range, sync::OnceLock};

const LIMIT: usize = 7;
const MAX_INPUT: usize = 8192;

pub(super) fn grammar() -> &'static Command {
    static GRAMMAR: OnceLock<Command> = OnceLock::new();
    GRAMMAR.get_or_init(|| {
        let mut parser = crate::cli::app();
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
    History,
}

impl Kind {
    pub(super) fn label(self, vi: bool) -> &'static str {
        let labels = match self {
            Self::Command => ["command", "lệnh"],
            Self::Subcommand => ["action", "thao tác"],
            Self::Option => ["option", "tùy chọn"],
            Self::Value => ["value", "giá trị"],
            Self::History => ["history", "lịch sử"],
        };
        labels[usize::from(vi)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Cli,
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
}

impl Completer {
    pub(super) fn new(vi: bool) -> Self {
        Self { vi }
    }

    pub(super) fn suggestions(&self, input: &str, cursor: usize) -> Vec<Suggestion> {
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
        if previous.peek().is_some_and(|word| word.value == "cnl") {
            previous.next();
        }
        let mut command = grammar();
        let mut command_path = Vec::new();
        let mut value = None;
        let mut positional = 0;
        for word in previous {
            if value.take().is_some() {
                continue;
            }
            if word.value == "--" {
                return Vec::new();
            }
            if let Some(flag) = word.value.strip_prefix("--") {
                if !flag.contains('=') {
                    value = command.get_arguments().find(|arg| {
                        arg.get_long() == Some(flag) && arg.get_action().takes_values()
                    });
                }
            } else if let Some(child) = command.find_subcommand(word.value.as_ref()) {
                command = child;
                command_path.push(child.get_name());
                positional = 0;
            } else {
                positional += 1;
            }
        }

        let mut candidates = Vec::new();
        let mut insert = range;
        if let Some((flag, _)) = input[insert.start..cursor]
            .strip_prefix("--")
            .and_then(|part| part.split_once('='))
        {
            value = command
                .get_arguments()
                .find(|arg| arg.get_long() == Some(flag));
            insert.start += flag.len() + 3;
        }
        if let Some(arg) = value {
            Self::values(&mut candidates, arg);
        } else {
            for child in command
                .get_subcommands()
                .filter(|child| !child.is_hide_set())
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
                    let text = format!("--{flag}");
                    candidates.push((
                        text.clone(),
                        text,
                        arg.get_help().map_or_else(String::new, ToString::to_string),
                        Kind::Option,
                    ));
                } else if arg.get_index().is_some_and(|index| {
                    index == positional + 1
                        || index <= positional
                            && matches!(arg.get_action(), clap::ArgAction::Append)
                }) {
                    Self::values(&mut candidates, arg);
                }
            }
        }
        let prefix = &input[insert.start..cursor];
        let original = &input[insert.clone()];
        let has_prefix = candidates
            .iter()
            .any(|(candidate, _, _, _)| candidate.starts_with(prefix));
        let mut ranked = candidates
            .into_iter()
            .filter_map(|(text, label, description, kind)| {
                if text == original {
                    return None;
                }
                let score = if text.starts_with(prefix) {
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
            left_score
                .cmp(right_score)
                .then_with(|| left.text.cmp(&right.text))
        });
        ranked.dedup_by(|(_, left), (_, right)| left.text == right.text);
        ranked
            .into_iter()
            .take(LIMIT)
            .map(|(_, item)| item)
            .collect()
    }

    fn values(candidates: &mut Vec<(String, String, String, Kind)>, arg: &Arg) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
