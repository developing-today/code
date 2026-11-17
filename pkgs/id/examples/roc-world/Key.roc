Modifier := [Shift, Alt, Control, Super, Hyper, Meta, CapsLock, NumLock].{
    is_eq : Modifier, Modifier -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)
}

NamedKey := [
    NamedEnter,
    NamedTab,
    NamedBackspace,
    NamedEscape,
    NamedInsert,
    NamedDelete,
    NamedUp,
    NamedDown,
    NamedLeft,
    NamedRight,
    NamedHome,
    NamedEnd,
    NamedPageUp,
    NamedPageDown,
    NamedF1,
    NamedF2,
    NamedF3,
    NamedF4,
    NamedF5,
    NamedF6,
    NamedF7,
    NamedF8,
    NamedF9,
    NamedF10,
    NamedF11,
    NamedF12,
].{
    is_eq : NamedKey, NamedKey -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)
}

EventKey := [Named(NamedKey), UnicodeScalar(Str)].{
    is_eq : EventKey, EventKey -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)
}

EventKind := [Press].{
    is_eq : EventKind, EventKind -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)
}

KeyEvent := [KeyEvent({ key : EventKey, modifiers : List(Modifier), kind : EventKind, source : Str }), EventUnknown(Str)].{
    is_eq : KeyEvent, KeyEvent -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)
}

DecodeResult(a) := [Ok(a), Err({})]

Key := [
    Char(Str),
    Ctrl(Str),
    Enter,
    Tab,
    Backspace,
    Escape,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Unknown(Str),
].{
    is_eq : Key, Key -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)

    ## Decode raw terminal input bytes (as text) into keys. Unrecognised escape
    ## sequences come back as `Unknown` with their bytes, so nothing is dropped.
    parse : Str -> List(Key)
    parse = |text| parse_loop(Str.to_utf8(text), [])

    ## Decode a kitty-disambiguate-v1 active-mode stream into rich key events.
    ## Additive: callers must only use it after selecting that profile (the
    ## disambiguation push is owned by slice E, still gated on F13.4 detection
    ## in #786). Unsupported control forms stay explicit `EventUnknown` values
    ## that retain their source text; a lone trailing ESC is unknown, never
    ## guessed, because active decoding uses no Escape timeout.
    parse_events : Str -> List(KeyEvent)
    parse_events = |text| events_loop(Str.to_utf8(text), [])

    events_loop : List(U8), List(KeyEvent) -> List(KeyEvent)
    events_loop = |bytes, acc| if List.is_empty(bytes) {
        acc
    } else {
        step = next_event(bytes)
        events_loop(step.rest, List.append(acc, step.event))
    }

    next_event : List(U8) -> { event : KeyEvent, rest : List(U8) }
    next_event = |bytes| {
        lead = List.first(bytes) ?? 0
        if lead == 27 {
            escape_event(bytes)
        } else if lead == 13 {
            { event: event_named(NamedEnter, [], "\r"), rest: drop(bytes, 1) }
        } else if lead == 9 {
            { event: event_named(NamedTab, [], "\t"), rest: drop(bytes, 1) }
        } else if lead == 8 or lead == 127 {
            { event: event_named(NamedBackspace, [], utf8_text([lead])), rest: drop(bytes, 1) }
        } else if lead < 32 {
            { event: EventUnknown(utf8_text([lead])), rest: drop(bytes, 1) }
        } else {
            scalar_event(bytes)
        }
    }

    scalar_event : List(U8) -> { event : KeyEvent, rest : List(U8) }
    scalar_event = |bytes| {
        lead = List.first(bytes) ?? 0
        len = utf8_len(lead)
        if List.len(bytes) < len {
            { event: EventUnknown(utf8_text(bytes)), rest: [] }
        } else {
            chunk = take(bytes, len)
            match Str.from_utf8(chunk) {
                Ok(text) => { event: event_scalar(text, [], text), rest: drop(bytes, len) }
                Err(_) => { event: EventUnknown(utf8_text(chunk)), rest: drop(bytes, len) }
            }
        }
    }

    escape_event : List(U8) -> { event : KeyEvent, rest : List(U8) }
    escape_event = |bytes| {
        second = List.get(bytes, 1) ?? 0
        if second == 91 {
            len = csi_len(bytes)
            token = take(bytes, len)
            { event: parse_csi(token), rest: drop(bytes, len) }
        } else if List.len(bytes) == 1 {
            { event: EventUnknown(utf8_text(bytes)), rest: [] }
        } else if second == 79 {
            n = if List.len(bytes) < 3 { List.len(bytes) } else { 3 }
            { event: EventUnknown(utf8_text(take(bytes, n))), rest: drop(bytes, n) }
        } else {
            want = 1 + utf8_len(second)
            n = if List.len(bytes) < want { List.len(bytes) } else { want }
            { event: EventUnknown(utf8_text(take(bytes, n))), rest: drop(bytes, n) }
        }
    }

    event_named : NamedKey, List(Modifier), Str -> KeyEvent
    event_named = |key, modifiers, source| KeyEvent({ key: Named(key), modifiers: modifiers, kind: Press, source: source })

    event_scalar : Str, List(Modifier), Str -> KeyEvent
    event_scalar = |scalar, modifiers, source| KeyEvent({ key: UnicodeScalar(scalar), modifiers: modifiers, kind: Press, source: source })

    parse_csi : List(U8) -> KeyEvent
    parse_csi = |token| {
        length = List.len(token)
        if length < 3 {
            EventUnknown(utf8_text(token))
        } else {
            final = List.get(token, length - 1) ?? 0
            body = List.sublist(token, { start: 2, len: length - 3 })
            if final == 117 {
                parse_csi_u(body, utf8_text(token))
            } else if final == 126 {
                parse_csi_tilde(body, utf8_text(token))
            } else if final == 90 {
                if List.is_empty(body) {
                    event_named(NamedTab, [Shift], utf8_text(token))
                } else {
                    EventUnknown(utf8_text(token))
                }
            } else {
                parse_csi_letter(body, final, utf8_text(token))
            }
        }
    }

    parse_csi_u : List(U8), Str -> KeyEvent
    parse_csi_u = |body, source| {
        fields = split_byte(body, 59)
        if List.len(fields) == 1 or List.len(fields) == 2 {
            match parse_decimal(List.first(fields) ?? []) {
                Err(_) => EventUnknown(source)
                Ok(code) => {
                    mods_field = if List.len(fields) == 1 { Ok(1) } else { parse_decimal(List.get(fields, 1) ?? []) }
                    match mods_field {
                        Err(_) => EventUnknown(source)
                        Ok(modcode) => match modifiers_from_code(modcode) {
                            Err(_) => EventUnknown(source)
                            Ok(modifiers) => match key_from_code(code) {
                                Err(_) => EventUnknown(source)
                                Ok(Named(key)) => event_named(key, modifiers, source)
                                Ok(UnicodeScalar(scalar)) => {
                                    normalized = normalize_shift(scalar, modifiers)
                                    normalized_modifiers = if normalized != scalar { remove_shift(modifiers) } else { modifiers }
                                    event_scalar(normalized, normalized_modifiers, source)
                                }
                            }
                        }
                    }
                }
            }
        } else {
            EventUnknown(source)
        }
    }

    parse_csi_tilde : List(U8), Str -> KeyEvent
    parse_csi_tilde = |body, source| {
        fields = split_byte(body, 59)
        if List.len(fields) == 1 or List.len(fields) == 2 {
            match parse_decimal(List.first(fields) ?? []) {
                Err(_) => EventUnknown(source)
                Ok(number) => {
                    mods_field = if List.len(fields) == 1 { Ok(1) } else { parse_decimal(List.get(fields, 1) ?? []) }
                    match mods_field {
                        Err(_) => EventUnknown(source)
                        Ok(modcode) => match key_from_tilde(number) {
                            Err(_) => EventUnknown(source)
                            Ok(key) => match modifiers_from_code(modcode) {
                                Err(_) => EventUnknown(source)
                                Ok(modifiers) => event_named(key, modifiers, source)
                            }
                        }
                    }
                }
            }
        } else {
            EventUnknown(source)
        }
    }

    parse_csi_letter : List(U8), U8, Str -> KeyEvent
    parse_csi_letter = |body, final, source| {
        key_field = if final == 65 { Ok(NamedUp) } else if final == 66 { Ok(NamedDown) } else if final == 67 { Ok(NamedRight) } else if final == 68 { Ok(NamedLeft) } else if final == 72 { Ok(NamedHome) } else if final == 70 { Ok(NamedEnd) } else if final == 80 { Ok(NamedF1) } else if final == 81 { Ok(NamedF2) } else if final == 83 { Ok(NamedF4) } else { Err({}) }
        match key_field {
            Err(_) => EventUnknown(source)
            Ok(key) => {
                fields = split_byte(body, 59)
                if List.is_empty(body) {
                    event_named(key, [], source)
                } else if List.len(fields) == 1 and fields == [[49]] {
                    event_named(key, [], source)
                } else if List.len(fields) == 2 and (List.first(fields) ?? []) == [49] {
                    match parse_decimal(List.get(fields, 1) ?? []) {
                        Err(_) => EventUnknown(source)
                        Ok(modcode) => match modifiers_from_code(modcode) {
                            Err(_) => EventUnknown(source)
                            Ok(modifiers) => event_named(key, modifiers, source)
                        }
                    }
                } else {
                    EventUnknown(source)
                }
            }
        }
    }

    key_from_code : U64 -> DecodeResult(EventKey)
    key_from_code = |code| if code == 9 {
        Ok(Named(NamedTab))
    } else if code == 13 {
        Ok(Named(NamedEnter))
    } else if code == 27 {
        Ok(Named(NamedEscape))
    } else if code == 127 {
        Ok(Named(NamedBackspace))
    } else if code >= 32 and code <= 1114111 and (code < 55296 or code > 57343) and (code < 57344 or code > 63743) {
        Ok(UnicodeScalar(utf8_text(u64_to_utf8(code))))
    } else {
        Err({})
    }

    key_from_tilde : U64 -> DecodeResult(NamedKey)
    key_from_tilde = |code| if code == 2 {
        Ok(NamedInsert)
    } else if code == 3 {
        Ok(NamedDelete)
    } else if code == 5 {
        Ok(NamedPageUp)
    } else if code == 6 {
        Ok(NamedPageDown)
    } else if code == 7 {
        Ok(NamedHome)
    } else if code == 8 {
        Ok(NamedEnd)
    } else if code == 11 {
        Ok(NamedF1)
    } else if code == 12 {
        Ok(NamedF2)
    } else if code == 13 {
        Ok(NamedF3)
    } else if code == 14 {
        Ok(NamedF4)
    } else if code == 15 {
        Ok(NamedF5)
    } else if code == 17 {
        Ok(NamedF6)
    } else if code == 18 {
        Ok(NamedF7)
    } else if code == 19 {
        Ok(NamedF8)
    } else if code == 20 {
        Ok(NamedF9)
    } else if code == 21 {
        Ok(NamedF10)
    } else if code == 23 {
        Ok(NamedF11)
    } else if code == 24 {
        Ok(NamedF12)
    } else {
        Err({})
    }

    modifiers_from_code : U64 -> DecodeResult(List(Modifier))
    modifiers_from_code = |code| if code < 1 or code > 256 {
        Err({})
    } else {
        bits = code - 1
        Ok(List.map(List.keep_if([{ mask: 1, modifier: Shift }, { mask: 2, modifier: Alt }, { mask: 4, modifier: Control }, { mask: 8, modifier: Super }, { mask: 16, modifier: Hyper }, { mask: 32, modifier: Meta }, { mask: 64, modifier: CapsLock }, { mask: 128, modifier: NumLock }], |entry| bits / entry.mask % 2 == 1), |entry| entry.modifier))
    }

    remove_shift : List(Modifier) -> List(Modifier)
    remove_shift = |modifiers| List.keep_if(modifiers, |modifier| modifier != Shift)

    normalize_shift : Str, List(Modifier) -> Str
    normalize_shift = |scalar, modifiers| {
        bytes = Str.to_utf8(scalar)
        if List.len(bytes) == 1 and List.any(modifiers, |modifier| modifier == Shift) {
            byte = List.first(bytes) ?? 0
            if byte >= 97 and byte <= 122 {
                utf8_text([byte - 32])
            } else if byte == 49 { "!" } else if byte == 50 { "@" } else if byte == 51 { "#" } else if byte == 52 { "$" } else if byte == 53 { "%" } else if byte == 54 { "^" } else if byte == 55 { "&" } else if byte == 56 { "*" } else if byte == 57 { "(" } else if byte == 48 { ")" } else if byte == 45 { "_" } else if byte == 61 { "+" } else if byte == 91 { "{" } else if byte == 93 { "}" } else if byte == 92 { "|" } else if byte == 59 { ":" } else if byte == 39 { "\"" } else if byte == 44 { "<" } else if byte == 46 { ">" } else if byte == 47 { "?" } else if byte == 96 { "~" } else { scalar }
        } else {
            scalar
        }
    }

    split_byte : List(U8), U8 -> List(List(U8))
    split_byte = |bytes, separator| split_byte_help(bytes, separator, [], [])

    split_byte_help : List(U8), U8, List(U8), List(List(U8)) -> List(List(U8))
    split_byte_help = |bytes, separator, current, acc| if List.is_empty(bytes) {
        List.append(acc, current)
    } else {
        byte = List.first(bytes) ?? 0
        if byte == separator {
            split_byte_help(drop(bytes, 1), separator, [], List.append(acc, current))
        } else {
            split_byte_help(drop(bytes, 1), separator, List.append(current, byte), acc)
        }
    }

    parse_decimal : List(U8) -> DecodeResult(U64)
    parse_decimal = |bytes| if List.is_empty(bytes) {
        Err({})
    } else {
        match parse_decimal_help(bytes, 0) {
            Err(_) => Err({})
            Ok(value) => if value > 1114111 { Err({}) } else { Ok(value) }
        }
    }

    parse_decimal_help : List(U8), U64 -> DecodeResult(U64)
    parse_decimal_help = |bytes, value| if List.is_empty(bytes) {
        Ok(value)
    } else {
        match digit_value(List.first(bytes) ?? 0) {
            Err(_) => Err({})
            Ok(digit) => if value > 111111 { Err({}) } else { parse_decimal_help(drop(bytes, 1), value * 10 + digit) }
        }
    }

    digit_value : U8 -> DecodeResult(U64)
    digit_value = |byte| if byte == 48 {
        Ok(0)
    } else if byte == 49 {
        Ok(1)
    } else if byte == 50 {
        Ok(2)
    } else if byte == 51 {
        Ok(3)
    } else if byte == 52 {
        Ok(4)
    } else if byte == 53 {
        Ok(5)
    } else if byte == 54 {
        Ok(6)
    } else if byte == 55 {
        Ok(7)
    } else if byte == 56 {
        Ok(8)
    } else if byte == 57 {
        Ok(9)
    } else {
        Err({})
    }

    u64_to_utf8 : U64 -> List(U8)
    u64_to_utf8 = |code| if code < 128 {
        [u64_to_u8(code)]
    } else if code < 2048 {
        [u64_to_u8(192 + code / 64), u64_to_u8(128 + code % 64)]
    } else if code < 65536 {
        [u64_to_u8(224 + code / 4096), u64_to_u8(128 + (code / 64) % 64), u64_to_u8(128 + code % 64)]
    } else {
        [u64_to_u8(240 + code / 262144), u64_to_u8(128 + (code / 4096) % 64), u64_to_u8(128 + (code / 64) % 64), u64_to_u8(128 + code % 64)]
    }

    u64_to_u8 : U64 -> U8
    u64_to_u8 = |number| u64_to_u8_help(number, 0)

    u64_to_u8_help : U64, U8 -> U8
    u64_to_u8_help = |remaining, value| if remaining == 0 { value } else { u64_to_u8_help(remaining - 1, value + 1) }

    ## Lossy projection for existing applications. Only semantics representable
    ## by the legacy Key union are projected; modifiers are never discarded to
    ## make an old application act.
    to_legacy : KeyEvent -> Key
    to_legacy = |event| match event {
        EventUnknown(source) => Unknown(source)
        KeyEvent(record) => if record.kind != Press {
            Unknown(record.source)
        } else {
            match record.key {
                Named(key) => if List.is_empty(record.modifiers) { named_to_legacy(key, record.source) } else { Unknown(record.source) }
                UnicodeScalar(scalar) => if List.is_empty(record.modifiers) {
                    Char(scalar)
                } else if record.modifiers == [Control] and is_plain_lower(scalar) {
                    Ctrl(scalar)
                } else {
                    Unknown(record.source)
                }
            }
        }
    }

    is_plain_lower : Str -> Bool
    is_plain_lower = |scalar| {
        bytes = Str.to_utf8(scalar)
        List.len(bytes) == 1 and (List.first(bytes) ?? 0) >= 97 and (List.first(bytes) ?? 0) <= 122
    }

    named_to_legacy : NamedKey, Str -> Key
    named_to_legacy = |key, source| if key == NamedEnter {
        Enter
    } else if key == NamedTab {
        Tab
    } else if key == NamedBackspace {
        Backspace
    } else if key == NamedEscape {
        Escape
    } else if key == NamedInsert {
        Unknown(source)
    } else if key == NamedDelete {
        Delete
    } else if key == NamedUp {
        Up
    } else if key == NamedDown {
        Down
    } else if key == NamedLeft {
        Left
    } else if key == NamedRight {
        Right
    } else if key == NamedHome {
        Home
    } else if key == NamedEnd {
        End
    } else if key == NamedPageUp {
        PageUp
    } else if key == NamedPageDown {
        PageDown
    } else {
        Unknown(source)
    }

    parse_loop : List(U8), List(Key) -> List(Key)
    parse_loop = |bytes, acc| if List.is_empty(bytes) {
        acc
    } else {
        step = next_key(bytes)
        parse_loop(step.rest, List.append(acc, step.key))
    }

    next_key : List(U8) -> { key : Key, rest : List(U8) }
    next_key = |bytes| {
        lead = List.first(bytes) ?? 0
        if lead == 27 {
            escape_key(bytes)
        } else if lead == 13 or lead == 10 {
            { key: Enter, rest: drop(bytes, 1) }
        } else if lead == 9 {
            { key: Tab, rest: drop(bytes, 1) }
        } else if lead == 8 or lead == 127 {
            { key: Backspace, rest: drop(bytes, 1) }
        } else if lead >= 1 and lead <= 26 {
            { key: Ctrl(utf8_text([lead + 96])), rest: drop(bytes, 1) }
        } else {
            len = utf8_len(lead)
            { key: Char(utf8_text(take(bytes, len))), rest: drop(bytes, len) }
        }
    }

    escape_key : List(U8) -> { key : Key, rest : List(U8) }
    escape_key = |bytes| {
        if starts_with(bytes, [27, 91, 65]) or starts_with(bytes, [27, 79, 65]) {
            { key: Up, rest: drop(bytes, 3) }
        } else if starts_with(bytes, [27, 91, 66]) or starts_with(bytes, [27, 79, 66]) {
            { key: Down, rest: drop(bytes, 3) }
        } else if starts_with(bytes, [27, 91, 67]) or starts_with(bytes, [27, 79, 67]) {
            { key: Right, rest: drop(bytes, 3) }
        } else if starts_with(bytes, [27, 91, 68]) or starts_with(bytes, [27, 79, 68]) {
            { key: Left, rest: drop(bytes, 3) }
        } else if starts_with(bytes, [27, 91, 72]) or starts_with(bytes, [27, 79, 72]) or starts_with(bytes, [27, 91, 49, 126]) {
            { key: Home, rest: drop(bytes, if starts_with(bytes, [27, 91, 49, 126]) { 4 } else { 3 }) }
        } else if starts_with(bytes, [27, 91, 70]) or starts_with(bytes, [27, 79, 70]) or starts_with(bytes, [27, 91, 52, 126]) {
            { key: End, rest: drop(bytes, if starts_with(bytes, [27, 91, 52, 126]) { 4 } else { 3 }) }
        } else if starts_with(bytes, [27, 91, 51, 126]) {
            { key: Delete, rest: drop(bytes, 4) }
        } else if starts_with(bytes, [27, 91, 53, 126]) {
            { key: PageUp, rest: drop(bytes, 4) }
        } else if starts_with(bytes, [27, 91, 54, 126]) {
            { key: PageDown, rest: drop(bytes, 4) }
        } else if List.len(bytes) == 1 {
            { key: Escape, rest: [] }
        } else if starts_with(bytes, [27, 91]) {
            len = csi_len(bytes)
            { key: Unknown(utf8_text(take(bytes, len))), rest: drop(bytes, len) }
        } else {
            { key: Escape, rest: drop(bytes, 1) }
        }
    }

    ## A control sequence runs to its final byte (0x40 to 0x7E), after `ESC [`.
    csi_len : List(U8) -> U64
    csi_len = |bytes| index_of_final(bytes, 2)

    index_of_final : List(U8), U64 -> U64
    index_of_final = |bytes, i| if i >= List.len(bytes) {
        List.len(bytes)
    } else {
        byte = List.get(bytes, i) ?? 0
        if byte >= 64 and byte <= 126 { i + 1 } else { index_of_final(bytes, i + 1) }
    }

    utf8_len : U8 -> U64
    utf8_len = |lead| if lead >= 240 {
        4
    } else if lead >= 224 {
        3
    } else if lead >= 192 {
        2
    } else {
        1
    }

    utf8_text : List(U8) -> Str
    utf8_text = |bytes| match Str.from_utf8(bytes) {
        Ok(text) => text
        Err(_) => "?"
    }

    starts_with : List(U8), List(U8) -> Bool
    starts_with = |bytes, prefix| List.len(bytes) >= List.len(prefix) and take(bytes, List.len(prefix)) == prefix

    take : List(U8), U64 -> List(U8)
    take = |bytes, n| List.sublist(bytes, { start: 0, len: n })

    drop : List(U8), U64 -> List(U8)
    drop = |bytes, n| List.sublist(bytes, { start: n, len: List.len(bytes) - n })
}

expect Key.parse("a") == [Char("a")]
expect Key.parse("\r") == [Enter]
expect Key.parse("\t") == [Tab]
expect Key.parse("\u(7f)") == [Backspace]
expect Key.parse("\u(03)") == [Ctrl("c")]
expect Key.parse("\u(1b)") == [Escape]
expect Key.parse("\u(1b)[A") == [Up]
expect Key.parse("\u(1b)[D") == [Left]
expect Key.parse("\u(1b)[3~") == [Delete]
expect Key.parse("\u(1b)OH") == [Home]
expect Key.parse("é") == [Char("é")]
expect Key.parse("hi\u(1b)[Bq") == [Char("h"), Char("i"), Down, Char("q")]
expect Key.parse("\u(1b)[1;5C") == [Unknown("\u(1b)[1;5C")]

expect Key.parse_events("\u(1b)[27u") == [Key.event_named(NamedEscape, [], "\u(1b)[27u")]
expect Key.parse_events("\u(1b)[120;3u") == [Key.event_scalar("x", [Alt], "\u(1b)[120;3u")]
expect Key.parse_events("\t") == [Key.event_named(NamedTab, [], "\t")]
expect Key.parse_events("\u(1b)[105;5u") == [Key.event_scalar("i", [Control], "\u(1b)[105;5u")]
expect Key.parse_events("\r") == [Key.event_named(NamedEnter, [], "\r")]
expect Key.parse_events("\u(1b)[109;5u") == [Key.event_scalar("m", [Control], "\u(1b)[109;5u")]
expect Key.parse_events("\u(1b)[1;5C") == [Key.event_named(NamedRight, [Control], "\u(1b)[1;5C")]
expect Key.parse_events("\u(1b)[A") == [Key.event_named(NamedUp, [], "\u(1b)[A")]
expect Key.parse_events("\u(1b)[1A") == [Key.event_named(NamedUp, [], "\u(1b)[1A")]
expect Key.parse_events("\u(1b)[9;2u") == [Key.event_named(NamedTab, [Shift], "\u(1b)[9;2u")]
expect Key.parse_events("\u(1b)[Z") == [Key.event_named(NamedTab, [Shift], "\u(1b)[Z")]
expect Key.parse_events("\u(1b)[27;3u") == [Key.event_named(NamedEscape, [Alt], "\u(1b)[27;3u")]
expect Key.parse_events("\u(1b)[3;7~") == [Key.event_named(NamedDelete, [Alt, Control], "\u(1b)[3;7~")]
expect Key.parse_events("\u(1b)[99;9u") == [Key.event_scalar("c", [Super], "\u(1b)[99;9u")]
expect Key.parse_events("hi") == [Key.event_scalar("h", [], "h"), Key.event_scalar("i", [], "i")]
expect Key.parse_events("é") == [Key.event_scalar("é", [], "é")]
expect Key.parse_events("😀") == [Key.event_scalar("😀", [], "😀")]
expect Key.parse_events("A") == [Key.event_scalar("A", [], "A")]
expect Key.parse_events("ą") == [Key.event_scalar("ą", [], "ą")]
expect Key.parse_events("\u(1b)[97;2u") == [Key.event_scalar("A", [], "\u(1b)[97;2u")]
expect Key.parse_events("\u(1b)[97;6u") == [Key.event_scalar("A", [Control], "\u(1b)[97;6u")]
expect Key.parse_events("\u(1b)[49;2u") == [Key.event_scalar("!", [], "\u(1b)[49;2u")]
expect Key.parse_events("\u(1b)[261;2u") == [Key.event_scalar("ą", [Shift], "\u(1b)[261;2u")]
expect Key.parse_events("\u(1b)[3u") == [EventUnknown("\u(1b)[3u")]
expect Key.parse_events("\u(1b)[15~") == [Key.event_named(NamedF5, [], "\u(1b)[15~")]
expect Key.parse_events("\u(1b)[15;2~") == [Key.event_named(NamedF5, [Shift], "\u(1b)[15;2~")]
expect Key.parse_events("\u(1b)[P") == [Key.event_named(NamedF1, [], "\u(1b)[P")]
expect Key.parse_events("\u(1b)[1;5P") == [Key.event_named(NamedF1, [Control], "\u(1b)[1;5P")]
expect Key.parse_events("\u(1b)[2~") == [Key.event_named(NamedInsert, [], "\u(1b)[2~")]
expect Key.parse_events("a\u(1b)[27u") == [Key.event_scalar("a", [], "a"), Key.event_named(NamedEscape, [], "\u(1b)[27u")]
expect Key.parse_events("\u(1b)R") == [EventUnknown("\u(1b)R")]
expect Key.parse_events("\u(1b)[57399u") == [EventUnknown("\u(1b)[57399u")]
expect Key.parse_events("\u(1b)[97;5:1u") == [EventUnknown("\u(1b)[97;5:1u")]
expect Key.parse_events("\u(1b)[97:65u") == [EventUnknown("\u(1b)[97:65u")]
expect Key.parse_events("\u(1b)") == [EventUnknown("\u(1b)")]
expect Key.parse_events("\u(1b)[1;") == [EventUnknown("\u(1b)[1;")]
expect Key.parse_events("\u(03)") == [EventUnknown("\u(03)")]
expect Key.parse_events("\n") == [EventUnknown("\n")]
expect Key.parse_events("\u(1b)[4~") == [EventUnknown("\u(1b)[4~")]
expect Key.parse_events("\u(1b)[29~") == [EventUnknown("\u(1b)[29~")]
expect Key.to_legacy(Key.event_named(NamedTab, [], "\t")) == Tab
expect Key.to_legacy(Key.event_named(NamedEscape, [], "\u(1b)[27u")) == Escape
expect Key.to_legacy(Key.event_scalar("i", [Control], "\u(1b)[105;5u")) == Ctrl("i")
expect Key.to_legacy(Key.event_scalar("i", [], "i")) == Char("i")
expect Key.to_legacy(Key.event_named(NamedTab, [Shift], "\u(1b)[Z")) == Unknown("\u(1b)[Z")
expect Key.to_legacy(Key.event_named(NamedF5, [], "\u(1b)[15~")) == Unknown("\u(1b)[15~")
