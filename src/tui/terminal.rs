#[cfg(not(windows))]
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute, queue,
    style::{
        Attribute, Attributes, Color, Print, SetAttribute, SetAttributes, SetBackgroundColor,
        SetForegroundColor,
    },
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    env,
    io::{self, Write},
    ops::Range,
    rc::Rc,
    time::Duration,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::animation::{SWEEP_LEVELS, Sweep};

const INPUT_RIM_RGB: (u8, u8, u8) = (78, 80, 86);
const COPPER_RGB: (u8, u8, u8) = (196, 145, 117);

const ORANGE: Color = Color::Rgb {
    r: 211,
    g: 133,
    b: 103,
};

const SHADOW: Color = Color::Rgb {
    r: 97,
    g: 57,
    b: 39,
};

const SHADOW_SOFT: Color = Color::Rgb {
    r: 44,
    g: 34,
    b: 33,
};

const BANNER_COLORS: [Color; 4] = [
    Color::Rgb {
        r: 255,
        g: 214,
        b: 168,
    },
    Color::Rgb {
        r: 246,
        g: 189,
        b: 142,
    },
    Color::Rgb {
        r: 235,
        g: 164,
        b: 114,
    },
    Color::Rgb {
        r: 209,
        g: 131,
        b: 88,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Plain,
    Muted,
    Heading,
    Accent,
    Shadow,
    ShadowSoft,
    Banner(usize),
    BannerOnShadow(usize, bool),
    ShadowOverSoft,
    SoftOverShadow,
    Key,
    String,
    Number,
    Literal,
    Punctuation,
    Keyword,
    Option,
    Path,
    Success,
    Warning,
    Error,
    Stderr,
    Match,
    Selection,
    Palette(PalettePart),
    Pty {
        foreground: Option<Color>,
        background: Option<Color>,
        attributes: Attributes,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PalettePart {
    InputRim,
    InputEdge,
    Input,
    InputGlow(u8),
    SuggestionEdge,
    Detail,
    Suggestion,
    Selected,
    ContextEdge,
    ContextItem,
    ContextSelected,
    SessionActive,
    SessionHover,
}

pub(super) fn color_enabled() -> bool {
    env::var_os("NO_COLOR").is_none() && env::var("TERM").as_deref() != Ok("dumb")
}

#[derive(Clone, Copy)]
enum SemanticDepth {
    Rgb,
    Indexed,
    Basic,
}

impl SemanticDepth {
    fn detect() -> Self {
        let term = env::var("TERM").unwrap_or_default();
        if env::var("COLORTERM").is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit"))
            || term.ends_with("-direct")
            || env::var_os("WT_SESSION").is_some_and(|value| !value.is_empty())
        {
            Self::Rgb
        } else if term.contains("256color") {
            Self::Indexed
        } else {
            #[cfg(windows)]
            if term.is_empty() && crossterm::ansi_support::supports_ansi() {
                return Self::Rgb;
            }
            Self::Basic
        }
    }

    fn color(self, rgb: (u8, u8, u8), index: u8, basic: Color) -> Color {
        match self {
            Self::Rgb => Color::Rgb {
                r: rgb.0,
                g: rgb.1,
                b: rgb.2,
            },
            Self::Indexed => Color::AnsiValue(index),
            Self::Basic => basic,
        }
    }

    fn allows_input_animation(self, color: bool, interactive: bool, reduced_motion: bool) -> bool {
        matches!(self, Self::Rgb) && color && interactive && !reduced_motion
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    color: Option<Color>,
    background: Option<Color>,
    attribute: Option<Attribute>,
    terminal_attributes: Attributes,
}

impl Style {
    fn for_tone(tone: Tone, depth: SemanticDepth) -> Self {
        use Color::*;
        let color = match tone {
            Tone::Plain => return Self::default(),
            Tone::Muted | Tone::Heading | Tone::Match | Tone::Selection => {
                return Self {
                    attribute: Some(match tone {
                        Tone::Muted => Attribute::Dim,
                        Tone::Heading => Attribute::Bold,
                        _ => Attribute::Reverse,
                    }),
                    ..Self::default()
                };
            }
            Tone::Accent => ORANGE,
            Tone::Shadow => SHADOW,
            Tone::ShadowSoft => SHADOW_SOFT,
            Tone::Banner(index) => BANNER_COLORS[index],
            Tone::BannerOnShadow(index, near) => {
                return Self {
                    color: Some(BANNER_COLORS[index]),
                    background: Some(if near { SHADOW } else { SHADOW_SOFT }),
                    ..Self::default()
                };
            }
            Tone::ShadowOverSoft | Tone::SoftOverShadow => {
                let (foreground, background) = if tone == Tone::ShadowOverSoft {
                    (SHADOW, SHADOW_SOFT)
                } else {
                    (SHADOW_SOFT, SHADOW)
                };
                return Self {
                    color: Some(foreground),
                    background: Some(background),
                    ..Self::default()
                };
            }
            Tone::Key => depth.color((125, 207, 222), 117, Cyan),
            Tone::String => depth.color((223, 198, 145), 222, DarkYellow),
            Tone::Number => depth.color((238, 170, 117), 216, Yellow),
            Tone::Literal => depth.color((130, 204, 181), 115, Green),
            Tone::Punctuation => depth.color((167, 166, 177), 248, Grey),
            Tone::Keyword => depth.color((202, 170, 234), 183, Magenta),
            Tone::Option => depth.color((231, 159, 126), 216, DarkYellow),
            Tone::Path => depth.color((154, 189, 237), 111, Blue),
            Tone::Success => depth.color((143, 207, 149), 114, Green),
            Tone::Warning => depth.color((242, 199, 105), 221, Yellow),
            Tone::Error => depth.color((244, 137, 130), 210, Red),
            Tone::Stderr => depth.color((178, 176, 187), 249, Grey),
            Tone::Palette(part) => return Self::palette(part, depth),
            Tone::Pty {
                foreground,
                background,
                attributes,
            } => {
                return Self {
                    color: foreground,
                    background,
                    terminal_attributes: attributes,
                    ..Self::default()
                };
            }
        };
        Self {
            color: Some(color),
            ..Self::default()
        }
    }

    fn palette(part: PalettePart, depth: SemanticDepth) -> Self {
        use Color::*;
        use PalettePart::*;
        let suggestions = depth.color((31, 33, 37), 235, DarkGrey);
        let input_rim = depth.color(INPUT_RIM_RGB, 240, DarkGrey);
        let (color, background) = match part {
            InputRim => (input_rim, None),
            InputEdge => (input_rim, None),
            InputGlow(strength) => {
                let amount = u16::from(strength.min(SWEEP_LEVELS));
                let levels = u16::from(SWEEP_LEVELS);
                let blend = |base: u8, copper: u8| {
                    ((u16::from(base) * (levels - amount)
                        + u16::from(copper) * amount
                        + levels / 2)
                        / levels) as u8
                };
                let rgb = (
                    blend(INPUT_RIM_RGB.0, COPPER_RGB.0),
                    blend(INPUT_RIM_RGB.1, COPPER_RGB.1),
                    blend(INPUT_RIM_RGB.2, COPPER_RGB.2),
                );
                (depth.color(rgb, 240, DarkGrey), None)
            }
            // A box-drawing cell cannot combine a third color outside its
            // stroke with a distinct interior background. Inherit the
            // terminal surface for the complete input to keep its rounded
            // border seamless without painting beyond the outer edge.
            Input => return Self::default(),
            SuggestionEdge => (suggestions, None),
            Detail => (depth.color((159, 161, 166), 248, Grey), Some(suggestions)),
            Suggestion => (depth.color((211, 212, 215), 252, White), Some(suggestions)),
            Selected => (depth.color((237, 237, 239), 255, White), Some(suggestions)),
            ContextEdge => (depth.color((100, 108, 118), 242, Grey), None),
            ContextItem => (depth.color((232, 233, 236), 254, White), Some(suggestions)),
            SessionActive => (
                depth.color((255, 224, 199), 223, White),
                Some(depth.color((66, 48, 41), 237, DarkGrey)),
            ),
            SessionHover => (
                depth.color((242, 242, 243), 255, White),
                Some(depth.color((44, 46, 50), 236, DarkGrey)),
            ),
            ContextSelected => (
                depth.color((248, 249, 251), 255, White),
                Some(depth.color((55, 70, 86), 238, DarkBlue)),
            ),
        };
        Self {
            color: Some(color),
            background,
            attribute: matches!(part, Selected | ContextSelected | SessionActive)
                .then_some(Attribute::Bold),
            ..Self::default()
        }
    }

    fn write(self, out: &mut impl Write, mut previous: Self) -> io::Result<()> {
        if self.terminal_attributes != previous.terminal_attributes
            && !previous.terminal_attributes.is_empty()
        {
            queue!(out, SetAttribute(Attribute::Reset))?;
            previous = Self::default();
        }
        if self.attribute != previous.attribute {
            if let Some(attribute) = previous.attribute {
                let reset = if attribute == Attribute::Reverse {
                    Attribute::NoReverse
                } else {
                    Attribute::NormalIntensity
                };
                queue!(out, SetAttribute(reset))?;
            }
            if let Some(attribute) = self.attribute {
                queue!(out, SetAttribute(attribute))?;
            }
        }
        if self.terminal_attributes != previous.terminal_attributes {
            queue!(out, SetAttributes(self.terminal_attributes))?;
        }
        if self.color != previous.color {
            queue!(out, SetForegroundColor(self.color.unwrap_or(Color::Reset)))?;
        }
        if self.background != previous.background {
            queue!(
                out,
                SetBackgroundColor(self.background.unwrap_or(Color::Reset))
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Line(Option<Rc<LineData>>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LineData {
    text: String,
    ends: Vec<(Tone, usize)>,
}

impl Line {
    pub(super) fn push(&mut self, tone: Tone, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        let data = Rc::make_mut(self.0.get_or_insert_with(Rc::default));
        data.text.push_str(text);
        let end = data.text.len();
        match data.ends.last_mut() {
            Some((last, previous_end)) if *last == tone => *previous_end = end,
            _ => data.ends.push((tone, end)),
        }
    }

    pub(super) fn contents(&self) -> &str {
        self.0.as_ref().map_or("", |data| data.text.as_str())
    }

    pub(super) fn select(&mut self, range: Range<usize>) {
        if range.is_empty() || self.contents().get(range.clone()).is_none() {
            return;
        }
        let mut selected = Self::default();
        let mut start = 0;
        for (tone, segment) in self.segments() {
            let end = start + segment.len();
            let left = range.start.clamp(start, end) - start;
            let right = range.end.clamp(start, end) - start;
            selected.push(tone, &segment[..left]);
            selected.push(Tone::Selection, &segment[left..right]);
            selected.push(tone, &segment[right..]);
            start = end;
        }
        *self = selected;
    }

    pub(super) fn segments(&self) -> impl DoubleEndedIterator<Item = (Tone, &str)> {
        self.0.iter().flat_map(|data| {
            data.ends.iter().enumerate().map(|(index, &(tone, end))| {
                let start = index.checked_sub(1).map_or(0, |i| data.ends[i].1);
                (tone, &data.text[start..end])
            })
        })
    }

    fn damage(&self, previous: Option<&Self>) -> (u16, Range<usize>) {
        let text = self.contents();
        let len = text.len();
        let Some(previous) = previous else {
            return (0, 0..len);
        };
        let old = previous.contents();
        let mut start = matching_edge(self.segments().zip(previous.segments()), false);
        let mut end = if old.len() == len {
            (len - matching_edge(self.segments().rev().zip(previous.segments().rev()), true))
                .max(start)
        } else {
            len
        };
        if text.is_ascii() && old.is_ascii() {
            return (start as u16, start..end);
        }

        // Cut around ASCII cells only, so a partial write cannot split a
        // Unicode glyph, combining sequence or styled grapheme.
        while start > 0 && !ascii_cut(text, old, start) {
            start -= 1;
        }
        while end < len && !ascii_cut(text, old, end) {
            end += 1;
        }
        let column = stable_columns(&text[..start]).unwrap_or_else(|| {
            start = 0;
            0
        });
        if end < len
            && !stable_columns(&text[..end])
                .is_some_and(|width| stable_columns(&old[..end]) == Some(width))
        {
            end = len;
        }
        (column as u16, start..end)
    }

    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        self.contents().to_owned()
    }

    #[cfg(test)]
    pub(super) fn has(&self, tone: Tone) -> bool {
        self.segments().any(|(actual, _)| actual == tone)
    }

    #[cfg(test)]
    pub(super) fn shares_storage(&self, other: &Self) -> bool {
        matches!((&self.0, &other.0), (Some(a), Some(b)) if Rc::ptr_eq(a, b))
    }
}

fn matching_edge<'a>(
    runs: impl Iterator<Item = ((Tone, &'a str), (Tone, &'a str))>,
    reverse: bool,
) -> usize {
    let mut count = 0;
    for ((tone, new), (old_tone, old)) in runs {
        if tone != old_tone {
            break;
        }
        if new == old {
            count += new.len();
            continue;
        }
        count += if reverse {
            new.bytes()
                .rev()
                .zip(old.bytes().rev())
                .take_while(|(a, b)| a == b)
                .count()
        } else {
            new.bytes()
                .zip(old.bytes())
                .take_while(|(a, b)| a == b)
                .count()
        };
        break;
    }
    count
}

fn ascii_cut(new: &str, old: &str, byte: usize) -> bool {
    let printable = |byte: &u8| byte.is_ascii_graphic() || *byte == b' ';
    [new, old].into_iter().all(|text| {
        let bytes = text.as_bytes();
        bytes.get(byte - 1).is_some_and(printable) && bytes.get(byte).is_none_or(printable)
    })
}

fn stable_columns(text: &str) -> Option<usize> {
    // Use a full-row write when joined glyphs have contextual width.
    let width = text.width();
    let cells = text
        .chars()
        .filter_map(UnicodeWidthChar::width)
        .sum::<usize>();
    (width == cells).then_some(width)
}

pub(super) struct MenuRows {
    pub(super) columns: Range<u16>,
    pub(super) rows: Range<u16>,
    pub(super) first: usize,
}

pub(super) struct Frame {
    pub(super) size: (u16, u16),
    pub(super) rows: Vec<Line>,
    pub(super) cursor: Option<(u16, u16)>,
    pub(super) menu: Option<MenuRows>,
    pub(super) suggestions: Option<MenuRows>,
    pub(super) input: Option<InputHit>,
    pub(super) input_border: Option<InputBorder>,
    pub(super) readable: Option<Readable>,
    pub(super) feedback_row: Option<u16>,
}

pub(super) struct Readable {
    pub(super) first: u16,
    pub(super) rows: Vec<SourceRow>,
    pub(super) inset: usize,
}

#[derive(Clone)]
pub(super) struct SourceRow {
    pub(super) bytes: Range<usize>,
    pub(super) substituted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InputBorder {
    pub(super) left: u16,
    pub(super) top: u16,
    pub(super) width: u16,
}

impl InputBorder {
    fn length(self) -> usize {
        usize::from(self.width) * 2 + 2
    }

    fn sweep(self, phase: Duration) -> Sweep {
        Sweep::new(phase, self.length())
    }

    fn cell(self, index: usize) -> (u16, u16, &'static str, PalettePart) {
        use PalettePart::{InputEdge, InputRim};
        let width = usize::from(self.width);
        let right = self.left + self.width - 1;
        match index {
            0 => (self.left, self.top, "╭", InputRim),
            n if n < width - 1 => (self.left + n as u16, self.top, "─", InputEdge),
            n if n == width - 1 => (right, self.top, "╮", InputRim),
            n if n == width => (right, self.top + 1, "│", InputEdge),
            n if n == width + 1 => (right, self.top + 2, "╯", InputRim),
            n if n < width * 2 => (right - (n - width - 1) as u16, self.top + 2, "─", InputEdge),
            n if n == width * 2 => (self.left, self.top + 2, "╰", InputRim),
            _ => (self.left, self.top + 1, "│", InputEdge),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct InputHit {
    pub(super) row: u16,
    pub(super) columns: Range<u16>,
    pub(super) offset: usize,
}

impl Frame {
    pub(super) fn new(size: (u16, u16)) -> Self {
        Self {
            size,
            cursor: None,
            rows: vec![Line::default(); usize::from(size.1)],
            menu: None,
            suggestions: None,
            input: None,
            input_border: None,
            readable: None,
            feedback_row: None,
        }
    }

    pub(super) fn menu_hit(&self, x: u16, y: u16) -> Option<usize> {
        let menu = self.menu.as_ref()?;
        (menu.columns.contains(&x) && menu.rows.contains(&y))
            .then(|| menu.first + usize::from(y - menu.rows.start))
    }

    pub(super) fn suggestion_hit(&self, x: u16, y: u16) -> Option<usize> {
        let choices = self.suggestions.as_ref()?;
        (choices.columns.contains(&x) && choices.rows.contains(&y))
            .then(|| choices.first + usize::from(y - choices.rows.start))
    }

    pub(super) fn changed_rows<'a>(
        &'a self,
        previous: Option<&'a Self>,
    ) -> impl Iterator<Item = (u16, &'a Line)> {
        let resized = previous.is_some_and(|old| old.size != self.size);
        let previous = previous.filter(|old| old.size == self.size);
        self.rows
            .iter()
            .enumerate()
            .filter(move |(y, line)| {
                resized
                    || match previous {
                        Some(old) => &old.rows[*y] != *line,
                        None => line.0.is_some(),
                    }
            })
            .map(|(y, line)| (y as u16, line))
    }
}

pub(super) struct Painter {
    previous: Option<Frame>,
    depth: SemanticDepth,
    input_phase: Option<Duration>,
}

impl Default for Painter {
    fn default() -> Self {
        Self {
            previous: None,
            depth: SemanticDepth::detect(),
            input_phase: None,
        }
    }
}

impl Painter {
    pub(super) fn last_frame(&self, size: (u16, u16)) -> Option<&Frame> {
        self.previous.as_ref().filter(|frame| frame.size == size)
    }

    pub(super) fn supports_input_animation(&self, color: bool, interactive: bool) -> bool {
        self.depth.allows_input_animation(
            color && color_enabled(),
            interactive,
            env::var_os("CNL_REDUCED_MOTION").is_some(),
        )
    }

    pub(super) fn has_animated_input(&self, size: (u16, u16)) -> bool {
        self.last_frame(size)
            .is_some_and(|frame| frame.input_border.is_some())
    }

    pub(super) fn animate_input(
        &mut self,
        out: &mut impl Write,
        phase: Option<Duration>,
    ) -> io::Result<()> {
        let Some(frame) = &self.previous else {
            return Ok(());
        };
        let Some(border) = frame
            .input_border
            .filter(|_| matches!(self.depth, SemanticDepth::Rgb))
        else {
            return Ok(());
        };
        if self.input_phase == phase {
            return Ok(());
        }
        let mut active = Style::default();
        let previous = self.input_phase.map(|p| border.sweep(p));
        let next = phase.map(|p| border.sweep(p));
        let mut written = false;
        for (cell, _) in previous.into_iter().flat_map(Sweep::cells) {
            if next.is_none_or(|sweep| sweep.strength(cell) == 0) {
                self.paint_input_cell(out, border, cell, None, &mut active)?;
                written = true;
            }
        }
        for (cell, strength) in next.into_iter().flat_map(Sweep::cells) {
            if previous.is_none_or(|sweep| sweep.strength(cell) != strength) {
                self.paint_input_cell(out, border, cell, Some(strength), &mut active)?;
                written = true;
            }
        }
        if written {
            Style::default().write(out, active)?;
            if let Some((x, y)) = frame.cursor {
                queue!(out, MoveTo(x, y))?;
            }
            out.flush()?;
        }
        self.input_phase = phase;
        Ok(())
    }

    fn paint_input_cell(
        &self,
        out: &mut impl Write,
        border: InputBorder,
        index: usize,
        strength: Option<u8>,
        active: &mut Style,
    ) -> io::Result<()> {
        let (x, y, glyph, part) = border.cell(index);
        let tone = Tone::Palette(strength.map_or(part, PalettePart::InputGlow));
        let next = Style::for_tone(tone, self.depth);
        queue!(out, MoveTo(x, y))?;
        next.write(out, *active)?;
        queue!(out, Print(glyph))?;
        *active = next;
        Ok(())
    }

    pub(super) fn draw(
        &mut self,
        out: &mut impl Write,
        frame: Frame,
        color: bool,
    ) -> io::Result<()> {
        let previous = self.previous.as_ref();
        let initial = previous.is_none();
        let resized = previous.is_some_and(|old| old.size != frame.size);
        if initial {
            queue!(out, MoveTo(0, 0), Clear(ClearType::All))?;
        }
        let mut changed = initial || resized;
        let mut active = Style::default();
        let old_border = previous.and_then(|old| old.input_border);
        if (resized || old_border != frame.input_border)
            && let Some(phase) = self.input_phase.take()
            && let Some(border) = old_border.filter(|_| !resized)
        {
            for (cell, _) in border.sweep(phase).cells() {
                self.paint_input_cell(out, border, cell, None, &mut active)?;
            }
            changed = true;
        }
        let glowing = frame.input_border.zip(self.input_phase);
        for (y, line) in frame.changed_rows(previous) {
            let old = previous
                .filter(|_| !resized)
                .map(|old| &old.rows[usize::from(y)]);
            let (column, dirty) = line.damage(old);
            let covered = glowing
                .filter(|(border, _)| (border.top..=border.top + 2).contains(&y))
                .map(|(border, phase)| {
                    let end = column + line.contents()[dirty.clone()].width() as u16;
                    (border, phase, column..end)
                });
            queue!(out, MoveTo(column, y))?;
            if color || line.segments().any(|(tone, _)| tone == Tone::Selection) {
                let mut offset = 0;
                for (tone, text) in line.segments() {
                    let end = offset + text.len();
                    if offset >= dirty.end {
                        break;
                    }
                    if end > dirty.start {
                        let next = if color || tone == Tone::Selection {
                            Style::for_tone(tone, self.depth)
                        } else {
                            Style::default()
                        };
                        next.write(out, active)?;
                        let start = dirty.start.saturating_sub(offset);
                        let stop = dirty.end.min(end) - offset;
                        queue!(out, Print(&text[start..stop]))?;
                        active = next;
                    }
                    offset = end;
                }
            } else {
                queue!(out, Print(&line.contents()[dirty]))?;
            }
            if resized {
                let plain = Style::default();
                plain.write(out, active)?;
                active = plain;
                queue!(out, Clear(ClearType::UntilNewLine))?;
            } else if let Some(old) = old {
                let mut stale = old
                    .contents()
                    .width()
                    .saturating_sub(line.contents().width());
                if stale > 0 {
                    let plain = Style::default();
                    plain.write(out, active)?;
                    active = plain;
                    const BLANKS: &str =
                        "                                                                ";
                    while stale > 0 {
                        let count = stale.min(BLANKS.len());
                        queue!(out, Print(&BLANKS[..count]))?;
                        stale -= count;
                    }
                }
            }
            if let Some((border, phase, columns)) = covered {
                for (cell, strength) in border.sweep(phase).cells() {
                    let (x, row, _, _) = border.cell(cell);
                    if row == y && columns.contains(&x) {
                        self.paint_input_cell(out, border, cell, Some(strength), &mut active)?;
                    }
                }
            }
            changed = true;
        }
        Style::default().write(out, active)?;
        let old_cursor = previous.and_then(|old| old.cursor);
        if let Some((x, y)) = frame.cursor {
            if changed || frame.cursor != old_cursor {
                queue!(out, MoveTo(x, y))?;
            }
            if old_cursor.is_none() {
                queue!(out, Show)?;
            }
        } else if old_cursor.is_some() {
            queue!(out, Hide)?;
        }
        if changed || frame.cursor != old_cursor {
            out.flush()?;
        }
        // A frame can change its hit regions without changing any pixels.
        self.previous = Some(frame);
        Ok(())
    }
}

pub(super) struct Screen<W: Write> {
    pub(super) out: W,
    raw: bool,
    mouse: bool,
    #[cfg(not(windows))]
    paste: bool,
}

impl<W: Write> Screen<W> {
    pub(super) fn enter(out: W) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Self::activate(out, true)
    }

    fn activate(out: W, raw: bool) -> io::Result<Self> {
        let mut screen = Self {
            out,
            raw,
            mouse: false,
            #[cfg(not(windows))]
            paste: false,
        };
        // A flush can fail after a command has already reached the terminal.
        execute!(screen.out, EnterAlternateScreen, Hide)?;
        screen.mouse = true;
        execute!(screen.out, EnableMouseCapture)?;
        #[cfg(not(windows))]
        {
            screen.paste = true;
            execute!(screen.out, EnableBracketedPaste)?;
        }
        Ok(screen)
    }
}

impl<W: Write> Drop for Screen<W> {
    fn drop(&mut self) {
        let _ = execute!(self.out, SetAttribute(Attribute::Reset));
        #[cfg(not(windows))]
        if self.paste {
            let _ = execute!(self.out, DisableBracketedPaste);
        }
        if self.mouse {
            let _ = execute!(self.out, DisableMouseCapture);
        }
        let _ = execute!(self.out, Show);
        let _ = execute!(self.out, LeaveAlternateScreen);
        if self.raw {
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn ran_in_ansi_fixture(test: &str) -> bool {
        const FIXTURE: &str = "CNL_TERMINAL_TEST_ANSI";
        if env::var_os(FIXTURE).is_some() || (!cfg!(windows) && env::var_os("NO_COLOR").is_none()) {
            return false;
        }
        // Crossterm selects ANSI or Win32 once per process.
        let (_, module) = module_path!().split_once("::").unwrap();
        let output = std::process::Command::new(env::current_exe().unwrap())
            .args(["--exact", &format!("{module}::{test}"), "--nocapture"])
            .env("TERM", "xterm-256color")
            .env_remove("NO_COLOR")
            .env(FIXTURE, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        true
    }

    #[test]
    fn copper_glow_moves_with_only_border_cell_damage() {
        if ran_in_ansi_fixture("copper_glow_moves_with_only_border_cell_damage") {
            return;
        }
        use crate::tui::{Input, Menu, view};

        let mut menu = Menu::new(true);
        menu.open_prompt(Input::at_end("run --desk".into()));
        let size = (180, 30);
        let mut painter = Painter {
            previous: None,
            depth: SemanticDepth::Rgb,
            input_phase: None,
        };
        let mut bytes = Vec::new();
        painter
            .draw(&mut bytes, view::frame(&mut menu, size, true, true), true)
            .unwrap();
        let initial_bytes = bytes.len();
        assert!(painter.has_animated_input(size));
        let frame = painter.last_frame(size).unwrap();
        let border = frame.input_border.unwrap();
        let (x, y) = frame.cursor.unwrap();
        let cursor_restore = format!("\x1b[{};{}H", y + 1, x + 1);
        assert!(border.width > 100);
        let changed_cells = |bytes: &[u8]| {
            let trace = std::str::from_utf8(bytes).unwrap();
            assert!(
                !trace.contains("--desk")
                    && !trace.contains("\x1b[2J")
                    && !trace.contains("\x1b[K"),
                "{trace:?}"
            );
            assert!(
                !trace.contains("\x1b[?25h") && !trace.contains("\x1b[?25l"),
                "{trace:?}"
            );
            assert!(trace.ends_with(&cursor_restore), "{trace:?}");
            trace.chars().filter(|ch| "╭─╮│╰╯".contains(*ch)).count()
        };
        let speed = crate::tui::animation::SWEEP_SPEED;
        let phase = Duration::from_secs_f64(f64::from(border.width - 1) / speed);
        let next_phase = phase + Duration::from_millis(42);
        bytes.clear();
        painter.animate_input(&mut bytes, Some(phase)).unwrap();
        assert!((12..=22).contains(&changed_cells(&bytes)));
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .contains("\x1b[38;2;196;145;117m╮")
        );
        bytes.clear();
        painter.animate_input(&mut bytes, Some(next_phase)).unwrap();
        let changed = changed_cells(&bytes);
        assert!((1..=24).contains(&changed));
        eprintln!(
            "180×30 first frame {initial_bytes} bytes; steady sweep tick {} bytes, {changed} cells",
            bytes.len()
        );
        bytes.clear();
        painter.animate_input(&mut bytes, Some(next_phase)).unwrap();
        assert!(bytes.is_empty());
        painter.animate_input(&mut bytes, None).unwrap();
        assert!((12..=22).contains(&changed_cells(&bytes)));
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("196;145;117"));
        bytes.clear();
        painter
            .draw(
                &mut bytes,
                view::frame(&mut menu, (17, 3), true, true),
                true,
            )
            .unwrap();
        assert!(!painter.has_animated_input((17, 3)));
    }

    #[test]
    fn copper_sweep_has_no_hard_change_between_adjacent_cells() {
        for width in [9, 18, 40, 84, 178] {
            let border = InputBorder {
                left: 2,
                top: 1,
                width,
            };
            let mut colors = vec![INPUT_RIM_RGB.0; border.length()];
            for (index, strength) in border.sweep(Duration::from_millis(133)).cells() {
                let tone = Style::palette(PalettePart::InputGlow(strength), SemanticDepth::Rgb);
                if let Some(Color::Rgb { r, .. }) = tone.color {
                    colors[index] = r;
                }
            }
            let steepest = colors
                .iter()
                .copied()
                .zip(colors.iter().copied().cycle().skip(1))
                .map(|(left, right)| left.abs_diff(right))
                .max()
                .unwrap();
            assert!(
                steepest <= 26,
                "{width} columns: neighboring cells change by {steepest}"
            );
        }
    }

    #[test]
    fn glow_requires_rgb_interactive_color_and_motion() {
        assert!(SemanticDepth::Rgb.allows_input_animation(true, true, false));
        assert!(!SemanticDepth::Rgb.allows_input_animation(false, true, false));
        assert!(!SemanticDepth::Rgb.allows_input_animation(true, false, false));
        assert!(!SemanticDepth::Rgb.allows_input_animation(true, true, true));
        assert!(!SemanticDepth::Indexed.allows_input_animation(true, true, false));
        assert!(!SemanticDepth::Basic.allows_input_animation(true, true, false));
    }

    #[test]
    fn resizing_repaints_without_clearing_the_screen_and_erases_stale_rows() {
        if ran_in_ansi_fixture(
            "resizing_repaints_without_clearing_the_screen_and_erases_stale_rows",
        ) {
            return;
        }
        let mut painter = Painter::default();
        let mut bytes = Vec::new();
        let mut original = Frame::new((24, 4));
        original.rows[0].push(Tone::Plain, "Việt 東京");
        original.rows[2].push(Tone::Plain, "stale content");
        painter.draw(&mut bytes, original, true).unwrap();

        for size in [(12, 4), (30, 6)] {
            let mut next = Frame::new(size);
            next.rows[0].push(Tone::Plain, "Việt 東京");
            bytes.clear();
            painter.draw(&mut bytes, next, true).unwrap();
            let trace = String::from_utf8_lossy(&bytes);
            assert!(!trace.contains("\x1b[2J"), "{trace:?}");
            assert!(trace.contains("Việt 東京"), "{trace:?}");
            assert_eq!(
                trace.matches("\x1b[K").count(),
                usize::from(size.1),
                "{trace:?}"
            );
        }
    }

    #[test]
    fn painter_updates_a_changed_row_without_toggling_the_cursor() {
        if ran_in_ansi_fixture("painter_updates_a_changed_row_without_toggling_the_cursor") {
            return;
        }

        let frame = |text| {
            let mut frame = Frame::new((50, 8));
            frame.rows[0].push(Tone::Banner(0), "CIRNETLAB");
            frame.rows[1].push(Tone::Plain, text);
            frame.cursor = Some((2, 5));
            frame
        };
        let mut bytes = Vec::new();
        let mut painter = Painter::default();
        painter.draw(&mut bytes, frame("first"), true).unwrap();
        bytes.clear();
        painter.draw(&mut bytes, frame("newer"), true).unwrap();
        let delta = String::from_utf8_lossy(&bytes);
        assert!(delta.contains("newer"));
        assert!(!delta.contains("CIRNETLAB"));
        assert!(!delta.contains("\x1b[2J"));
        assert!(
            !delta.contains("\x1b[?25h") && !delta.contains("\x1b[?25l"),
            "{delta:?}"
        );
        assert!(!delta.contains("\x1b[K"), "{delta:?}");
        let length = bytes.len();
        painter.draw(&mut bytes, frame("newer"), true).unwrap();
        assert_eq!(bytes.len(), length);
        bytes.clear();
        painter.draw(&mut bytes, frame("new"), true).unwrap();
        let delta = String::from_utf8_lossy(&bytes);
        assert!(delta.contains("\x1b[2;4H  "), "{delta:?}");
        assert!(!delta.contains("\x1b[K"), "{delta:?}");
    }

    #[test]
    fn styled_ascii_damage_rewrites_only_the_changed_run() {
        let line = |number| {
            let mut row = Line::default();
            row.push(Tone::Plain, "  ");
            row.push(Tone::Punctuation, "[");
            row.push(Tone::Number, number);
            row.push(Tone::Punctuation, "]");
            row
        };
        let old = line("99");
        assert_eq!(line("88").damage(Some(&old)), (3, 3..5));
        assert_eq!(line("1").damage(Some(&old)), (3, 3..5));
        assert_eq!(line("東京").damage(Some(&old)), (2, 2..10));
        let old_copy = old.clone();
        assert!(old.shares_storage(&old_copy));
    }

    #[test]
    fn palette_edit_preserves_static_unicode_cells_and_writes_inside_a_run() {
        if ran_in_ansi_fixture(
            "palette_edit_preserves_static_unicode_cells_and_writes_inside_a_run",
        ) {
            return;
        }

        let frame = |input| {
            let mut frame = Frame::new((80, 20));
            let row = &mut frame.rows[4];
            row.push(Tone::Plain, "  Danh mục 東京");
            row.push(Tone::Muted, "  │  › ");
            row.push(Tone::Plain, input);
            row.push(Tone::Muted, "  │");
            frame.cursor = Some((25, 4));
            frame
        };
        let mut bytes = Vec::new();
        let mut painter = Painter::default();
        painter
            .draw(&mut bytes, frame("env lock       "), true)
            .unwrap();
        bytes.clear();
        painter
            .draw(&mut bytes, frame("env plan       "), true)
            .unwrap();
        let trace = String::from_utf8_lossy(&bytes);
        assert!(trace.starts_with("\x1b[5;27Hplan"), "{trace:?}");
        assert!(!trace.contains("Danh mục"), "{trace:?}");
        assert!(!trace.contains("       "), "{trace:?}");
        assert!(!trace.contains("\x1b[2J"), "{trace:?}");
        bytes.clear();
        painter
            .draw(&mut bytes, frame("env plan       "), true)
            .unwrap();
        assert!(bytes.is_empty());

        painter.draw(&mut bytes, frame("env"), true).unwrap();
        let trace = String::from_utf8_lossy(&bytes);
        assert!(!trace.contains("Danh mục"), "{trace:?}");
        assert!(trace.contains("            "), "{trace:?}");
        assert!(!trace.contains("\x1b[K"), "{trace:?}");
    }

    #[test]
    fn damage_does_not_start_inside_a_combining_character() {
        let line = |input| {
            let mut row = Line::default();
            row.push(Tone::Muted, " │  ");
            row.push(Tone::Plain, input);
            row
        };
        let old = line("e\u{301}dit");
        let new = line("e\u{300}dit");
        let (column, dirty) = new.damage(Some(&old));
        assert_eq!(column, 4);
        assert!(new.contents()[dirty].starts_with("e\u{300}"));
    }

    #[test]
    fn selection_from_an_older_language_cannot_split_a_utf8_character() {
        let mut line = Line::default();
        line.push(Tone::Plain, "Đã chạy");
        line.select(1..3);
        assert_eq!(line.contents(), "Đã chạy");
        assert!(!line.has(Tone::Selection));
    }

    #[test]
    fn same_color_is_emitted_once_across_updated_rows() {
        if ran_in_ansi_fixture("same_color_is_emitted_once_across_updated_rows") {
            return;
        }

        let frame = |number| {
            let mut frame = Frame::new((20, 6));
            for line in &mut frame.rows[..2] {
                line.push(Tone::Punctuation, "[");
                line.push(Tone::Number, number);
                line.push(Tone::Punctuation, "]");
            }
            frame
        };
        let mut painter = Painter {
            previous: None,
            depth: SemanticDepth::Rgb,
            input_phase: None,
        };
        let mut bytes = Vec::new();
        painter.draw(&mut bytes, frame("1"), true).unwrap();
        bytes.clear();
        painter.draw(&mut bytes, frame("2"), true).unwrap();
        let delta = String::from_utf8_lossy(&bytes);
        assert_eq!(
            delta.matches("\x1b[38;2;238;170;117m").count(),
            1,
            "{delta:?}"
        );
        assert!(delta.ends_with("\x1b[39m"));
    }

    #[test]
    fn palette_uses_neutral_contrast_for_selection_and_descriptions() {
        let color = |part| {
            let style = Style::palette(part, SemanticDepth::Rgb);
            match style.color.unwrap() {
                Color::Rgb { r, g, b } => (r, g, b, style.background),
                _ => panic!("expected RGB palette color"),
            }
        };
        let (dr, dg, db, detail_bg) = color(PalettePart::Detail);
        let (nr, ng, nb, normal_bg) = color(PalettePart::Suggestion);
        let (sr, sg, sb, selected_bg) = color(PalettePart::Selected);
        for (r, g, b) in [(dr, dg, db), (nr, ng, nb), (sr, sg, sb)] {
            assert!(r.abs_diff(g) <= 8 && g.abs_diff(b) <= 8);
        }
        assert!(dr < nr && nr < sr);
        assert_eq!(detail_bg, normal_bg);
        assert_eq!(normal_bg, selected_bg);
    }

    #[test]
    fn input_and_border_share_the_terminal_surface_without_halos() {
        let input = Style::palette(PalettePart::Input, SemanticDepth::Rgb);
        let edge = Style::palette(PalettePart::InputEdge, SemanticDepth::Rgb);
        let corner = Style::palette(PalettePart::InputRim, SemanticDepth::Rgb);
        assert_eq!(edge.background, input.background);
        assert!(edge.background.is_none());
        assert!(corner.background.is_none());
    }

    #[test]
    fn suggestion_half_cells_use_the_surface_color_without_an_outer_background() {
        for depth in [
            SemanticDepth::Rgb,
            SemanticDepth::Indexed,
            SemanticDepth::Basic,
        ] {
            let surface = Style::palette(PalettePart::Detail, depth);
            let edge = Style::palette(PalettePart::SuggestionEdge, depth);
            assert_eq!(edge.color, surface.background);
            assert!(edge.background.is_none());
            assert!(edge.attribute.is_none());
        }
    }

    #[test]
    fn shrinking_a_row_clears_only_the_stale_cells() {
        if ran_in_ansi_fixture("shrinking_a_row_clears_only_the_stale_cells") {
            return;
        }
        let mut painter = Painter::default();
        let mut out = Vec::new();
        let mut previous = Frame::new((40, 4));
        previous.rows[1].push(Tone::Palette(PalettePart::Detail), "  suggestion");
        painter.draw(&mut out, previous, true).unwrap();
        out.clear();
        let mut next = Frame::new((40, 4));
        next.rows[1].push(Tone::Plain, "  done");
        painter.draw(&mut out, next, true).unwrap();
        let trace = String::from_utf8_lossy(&out);
        assert!(!trace.contains("\x1b[K"), "{trace:?}");
        assert!(trace.contains("done      "), "{trace:?}");
    }

    #[test]
    fn extrusion_styles_keep_the_face_color_and_distinct_depth_layers() {
        let near = Style::for_tone(Tone::BannerOnShadow(1, true), SemanticDepth::Rgb);
        let far = Style::for_tone(Tone::BannerOnShadow(1, false), SemanticDepth::Rgb);
        assert_eq!(near.color, Some(BANNER_COLORS[1]));
        assert_eq!(far.color, near.color);
        assert_eq!(near.background, Some(SHADOW));
        assert_eq!(far.background, Some(SHADOW_SOFT));
        assert_ne!(near.background, far.background);
        let transition = Style::for_tone(Tone::ShadowOverSoft, SemanticDepth::Rgb);
        assert_eq!(transition.color, Some(SHADOW));
        assert_eq!(transition.background, Some(SHADOW_SOFT));
    }

    #[test]
    fn palette_background_does_not_leak_to_output_or_cleared_cells() {
        if ran_in_ansi_fixture("palette_background_does_not_leak_to_output_or_cleared_cells") {
            return;
        }
        let mut frame = Frame::new((40, 6));
        frame.rows[1].push(Tone::Palette(PalettePart::Detail), "› env       ");
        frame.rows[2].push(Tone::Plain, "output");
        let mut painter = Painter {
            previous: None,
            depth: SemanticDepth::Rgb,
            input_phase: None,
        };
        let mut bytes = Vec::new();
        painter.draw(&mut bytes, frame, true).unwrap();
        let initial = String::from_utf8_lossy(&bytes);
        assert!(initial.contains("\x1b[48;2;31;33;37m› env"), "{initial:?}");
        assert!(initial.contains("\x1b[49moutput"), "{initial:?}");

        let mut closed = Frame::new((40, 6));
        closed.rows[1].push(Tone::Plain, "env");
        closed.rows[2].push(Tone::Plain, "output");
        bytes.clear();
        painter.draw(&mut bytes, closed, true).unwrap();
        let delta = String::from_utf8_lossy(&bytes);
        assert!(delta.contains("env         "), "{delta:?}");
        assert!(!delta.contains("\x1b[K"), "{delta:?}");
        assert!(!delta.contains("\x1b[48;"), "{delta:?}");
        assert!(!delta.contains("\x1b[2J"), "{delta:?}");
    }

    #[test]
    fn input_border_meets_its_surface_and_suggestions_have_no_border() {
        use crate::tui::{Menu, view};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut menu = Menu::new(false);
        for ch in "env s".chars() {
            menu.handle(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let frame = view::frame(&mut menu, (120, 35), true, true);
        let input = usize::from(frame.cursor.unwrap().1);
        let input_surface = Style::palette(PalettePart::Input, SemanticDepth::Rgb).background;
        let suggestions = Style::palette(PalettePart::Detail, SemanticDepth::Rgb).background;
        for (index, line) in frame.rows[input - 1..=input + 3].iter().enumerate() {
            let spans: Vec<_> = line
                .segments()
                .filter(|(tone, _)| matches!(tone, Tone::Palette(_)))
                .collect();
            let (first, _) = spans[0];
            let (last, _) = *spans.last().unwrap();
            let style = |tone| Style::for_tone(tone, SemanticDepth::Rgb);
            if matches!(index, 0 | 2) {
                assert!(style(first).background.is_none(), "row {index}, corner");
                assert!(style(last).background.is_none(), "row {index}, corner");
                assert_eq!(style(spans[1].0).background, input_surface);
            } else if index == 1 {
                assert_eq!(style(first).background, input_surface);
                assert_eq!(style(last).background, input_surface);
            } else {
                assert_eq!(first, Tone::Palette(PalettePart::SuggestionEdge));
                assert_eq!(last, Tone::Palette(PalettePart::SuggestionEdge));
                assert_eq!(style(first).color, suggestions);
                assert!(style(first).background.is_none());
                assert!(style(last).background.is_none());
                assert!(
                    spans[1..spans.len() - 1]
                        .iter()
                        .all(|(tone, _)| style(*tone).background == suggestions),
                    "row {index}, suggestion surface"
                );
            }
        }
    }

    struct FailOnce {
        bytes: Rc<RefCell<Vec<u8>>>,
        failed: bool,
    }

    impl Write for FailOnce {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if std::mem::replace(&mut self.failed, true) {
                Ok(())
            } else {
                Err(io::Error::other(
                    "first flush fails after entering alternate screen",
                ))
            }
        }
    }

    #[test]
    fn entering_alternate_screen_restores_cursor_and_screen_after_flush_error() {
        if ran_in_ansi_fixture(
            "entering_alternate_screen_restores_cursor_and_screen_after_flush_error",
        ) {
            return;
        }

        let bytes = Rc::new(RefCell::new(Vec::new()));
        let writer = FailOnce {
            bytes: Rc::clone(&bytes),
            failed: false,
        };
        assert!(Screen::activate(writer, false).is_err());
        let bytes = bytes.borrow();
        let trace = String::from_utf8_lossy(&bytes);
        assert!(trace.contains("\x1b[?1049h"));
        assert!(trace.contains("\x1b[?25h"));
        assert!(trace.contains("\x1b[?1049l"));
    }
}
