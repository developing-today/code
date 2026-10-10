Screen := {
    width : U64,
    height : U64,
    cells : List({ ch : Str, fg : U8, bg : U8, bold : Bool }),
}.{
    ## A blank screen of `width` by `height` cells, every cell a space in the default style.
    new : U64, U64 -> Screen
    new = |width, height| {
        width: width,
        height: height,
        cells: List.repeat({ ch: " ", fg: 0, bg: 0, bold: Bool.False }, width * height),
    }

    range : U64, U64 -> List(U64)
    range = |start, end| List.map_with_index(List.repeat(0, end - start), |_, i| start + i)

    ## Split text into one string per Unicode scalar value.
    chars : Str -> List(Str)
    chars = |text| {
        split = List.fold(Str.to_utf8(text), { done: [], current: [] }, |acc, byte| {
            if byte >= 128 and byte < 192 {
                { done: acc.done, current: List.append(acc.current, byte) }
            } else if List.is_empty(acc.current) {
                { done: acc.done, current: [byte] }
            } else {
                { done: List.append(acc.done, acc.current), current: [byte] }
            }
        })
        all = if List.is_empty(split.current) { split.done } else { List.append(split.done, split.current) }
        List.map(all, bytes_to_str)
    }

    bytes_to_str : List(U8) -> Str
    bytes_to_str = |bytes| match Str.from_utf8(bytes) {
        Ok(text) => text
        Err(_) => "?"
    }

    ## Write `text` starting at column `x` of row `y`, clipped to the screen.
    put_text : Screen, U64, U64, Str, { fg : U8, bg : U8, bold : Bool } -> Screen
    put_text = |screen, x, y, text, style| {
        List.fold(chars(text), { screen: screen, column: x }, |acc, ch| {
            column = acc.column
            next = if column < acc.screen.width and y < acc.screen.height {
                index = y * acc.screen.width + column
                cell = { ch: ch, fg: style.fg, bg: style.bg, bold: style.bold }
                match List.set(acc.screen.cells, index, cell) {
                    Ok(cells) => { width: acc.screen.width, height: acc.screen.height, cells: cells }
                    Err(_) => acc.screen
                }
            } else {
                acc.screen
            }
            { screen: next, column: column + 1 }
        }).screen
    }

    ## Draw an ASCII border around the rectangle at (`x`, `y`) of size `w` by `h`,
    ## with `title` written into the top edge.
    boxed : Screen, U64, U64, U64, U64, Str -> Screen
    boxed = |screen, x, y, w, h, title| {
        plain = { fg: 0, bg: 0, bold: Bool.False }
        if w < 2 or h < 2 {
            screen
        } else {
            inner = Str.join_with(List.repeat("-", w - 2), "")
            top = put_text(screen, x, y, Str.concat("+", Str.concat(inner, "+")), plain)
            titled = put_text(top, x + 2, y, title, plain)
            sides = List.fold(range(y + 1, y + h - 1), titled, |acc, row| {
                with_left = put_text(acc, x, row, "|", plain)
                put_text(with_left, x + w - 1, row, "|", plain)
            })
            put_text(sides, x, y + h - 1, Str.concat("+", Str.concat(inner, "+")), plain)
        }
    }

    ## The screen as plain text: rows joined by newlines, styles dropped.
    render_plain : Screen -> Str
    render_plain = |screen| {
        lines = List.map(range(0, screen.height), |row| row_text(screen, row))
        Str.join_with(lines, "\n")
    }

    row_text : Screen, U64 -> Str
    row_text = |screen, row| {
        start = row * screen.width
        cells = List.sublist(screen.cells, { start: start, len: screen.width })
        Str.join_with(List.map(cells, |cell| cell.ch), "")
    }

    ## The screen as ANSI terminal output: home the cursor, then each row with
    ## SGR codes wherever the style changes. Rows end with CRLF for raw terminals.
    render_ansi : Screen -> Str
    render_ansi = |screen| {
        lines = List.map(range(0, screen.height), |row| ansi_row(screen, row))
        Str.concat("\u(1b)[H", Str.join_with(lines, "\r\n"))
    }

    ansi_row : Screen, U64 -> Str
    ansi_row = |screen, row| {
        start = row * screen.width
        cells = List.sublist(screen.cells, { start: start, len: screen.width })
        pieces = List.fold(cells, { out: "", style: "" }, |acc, cell| {
            style = sgr(cell.fg, cell.bg, cell.bold)
            if style == acc.style {
                { out: Str.concat(acc.out, cell.ch), style: acc.style }
            } else {
                { out: Str.concat(acc.out, Str.concat("\u(1b)[0m", Str.concat(style, cell.ch))), style: style }
            }
        })
        Str.concat(pieces.out, "\u(1b)[0m")
    }

    sgr : U8, U8, Bool -> Str
    sgr = |fg, bg, bold| {
        weight = if bold { "\u(1b)[1m" } else { "" }
        foreground = if fg == 0 { "" } else { "\u(1b)[38;5;${U8.to_str(fg)}m" }
        background = if bg == 0 { "" } else { "\u(1b)[48;5;${U8.to_str(bg)}m" }
        Str.concat(weight, Str.concat(foreground, background))
    }
}

expect Screen.chars("ab") == ["a", "b"]
expect Screen.chars("h\u(e9)") == ["h", "\u(e9)"]
expect Screen.range(2, 5) == [2, 3, 4]
expect Screen.render_plain(Screen.put_text(Screen.new(4, 2), 1, 0, "hi", { fg: 0, bg: 0, bold: Bool.False })) == " hi \n    "
expect Screen.render_plain(Screen.put_text(Screen.new(3, 1), 2, 0, "abc", { fg: 0, bg: 0, bold: Bool.False })) == "  a"
expect Screen.render_plain(Screen.boxed(Screen.new(6, 3), 0, 0, 6, 3, "t")) == "+-t--+\n|    |\n+----+"
