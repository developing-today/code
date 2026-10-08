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
