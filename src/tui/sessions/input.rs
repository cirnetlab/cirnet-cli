use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

pub(super) fn key_bytes(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    use KeyCode::*;
    let modifiers = key.modifiers;
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let control = modifiers.contains(KeyModifiers::CONTROL);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let csi = |suffix: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{suffix}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{suffix}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        }
    };
    let extended = |code| {
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let simple = |byte| {
        if alt { vec![0x1b, byte] } else { vec![byte] }
    };
    Some(match key.code {
        Enter => simple(b'\r'),
        Backspace => simple(if control { 0x08 } else { 0x7f }),
        Tab if shift => b"\x1b[Z".to_vec(),
        Tab => simple(b'\t'),
        BackTab => b"\x1b[Z".to_vec(),
        Esc => vec![0x1b],
        Up => csi('A'),
        Down => csi('B'),
        Right => csi('C'),
        Left => csi('D'),
        Home => csi('H'),
        End => csi('F'),
        Insert => extended(2),
        Delete => extended(3),
        PageUp => extended(5),
        PageDown => extended(6),
        F(number @ 1..=4) => {
            let suffix = char::from(b'P' + number - 1);
            if modifier > 1 {
                format!("\x1b[1;{modifier}{suffix}").into_bytes()
            } else {
                format!("\x1bO{suffix}").into_bytes()
            }
        }
        F(number @ 5..=12) => extended([15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)]),
        Char(character) if control && character.is_ascii() => {
            let ascii = character.to_ascii_uppercase() as u8;
            match ascii {
                b'@'..=b'_' => simple(ascii & 0x1f),
                b' ' => simple(0),
                b'?' => simple(0x7f),
                _ => return None,
            }
        }
        Char(character) if !character.is_control() => {
            let mut encoded = [0_u8; 4];
            let bytes = character.encode_utf8(&mut encoded).as_bytes();
            let mut packet = Vec::with_capacity(bytes.len() + 1);
            // AltGr can report both Alt and Ctrl for a printable character.
            if alt && !control {
                packet.push(0x1b);
            }
            packet.extend_from_slice(bytes);
            packet
        }
        _ => return None,
    })
}

pub(super) fn mouse_bytes(event: MouseEvent, screen: &vt100::Screen, top: u16) -> Option<Vec<u8>> {
    use MouseEventKind::*;
    use MouseProtocolMode::*;
    let mode = screen.mouse_protocol_mode();
    let x = event.column.checked_add(1)?;
    let y = event.row.checked_sub(top)?.checked_add(1)?;
    let (rows, columns) = screen.size();
    if matches!(mode, None) || x > columns.get() || y > rows.get() {
        return Option::None;
    }
    let button = |button| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let mut code = match event.kind {
        Down(which) | Up(which) if !matches!(event.kind, Up(_)) || mode != Press => button(which),
        Drag(which) if matches!(mode, ButtonMotion | AnyMotion) => 32 + button(which),
        Moved if mode == AnyMotion => 35,
        ScrollUp => 64,
        ScrollDown => 65,
        ScrollLeft => 66,
        ScrollRight => 67,
        _ => return Option::None,
    };
    let release = matches!(event.kind, Up(_));
    let encoding = screen.mouse_protocol_encoding();
    if release && encoding != MouseProtocolEncoding::Sgr {
        code = 3;
    }
    code += 4 * u16::from(event.modifiers.contains(KeyModifiers::SHIFT))
        + 8 * u16::from(event.modifiers.contains(KeyModifiers::ALT))
        + 16 * u16::from(event.modifiers.contains(KeyModifiers::CONTROL));
    if encoding == MouseProtocolEncoding::Sgr {
        let mark = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{mark}").into_bytes());
    }
    let mut bytes = b"\x1b[M".to_vec();
    for value in [code + 32, x + 32, y + 32] {
        if encoding == MouseProtocolEncoding::Utf8 {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(
                char::from_u32(u32::from(value))?
                    .encode_utf8(&mut encoded)
                    .as_bytes(),
            );
        } else {
            bytes.push(u8::try_from(value).ok()?);
        }
    }
    Some(bytes)
}
