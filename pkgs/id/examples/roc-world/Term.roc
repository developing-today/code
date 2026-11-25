import Screen

## The three-mode terminal harness for Roc world programs (#14861).
##
## A world program's `view` produces a `Screen` frame; how that frame
## reaches the user's terminal is a presentation choice, negotiated per
## session. This module is the contract the three presenters share, so a
## game never changes when the mode does:
##
## - `Alternate` — the presenter owns an alternate screen buffer and
##   repaints it (the world SSH shell's raw mode today). Frames are the
##   `Screen.render_json` document; the presenter diffs locally.
## - `Primary` — no alternate buffer: the presenter writes each frame
##   transition into the primary screen with cursor addressing and
##   erase/rewrite, via `paint_primary`. Scrollback above the frame is
##   left alone.
## - `Text` — the low-write mode: no escape sequences at all. A frame
##   is plain text lines (`text_lines`) and a transition emits only the
##   lines that changed (`diff_lines`), so a quiet frame costs zero
##   bytes and a busy one costs a few lines. Board games (checkers and
##   chess, #14862) require this mode.
##
## Everything here is a pure function over `Screen` frames. The diff
## currency is `Screen.diff_cells`: the minimal set of cells a
## transition must write.
Mode := [Alternate, Primary, Text].{
    is_eq : Mode, Mode -> Bool
    is_eq = |a, b| Str.inspect(a) == Str.inspect(b)

    ## The session-negotiation name of a mode.
    name : Mode -> Str
    name = |mode| match mode {
        Alternate => "alternate"
        Primary => "primary"
        Text => "text"
    }

    ## Parse a negotiation name. Unknown names fall back to `Alternate`,
    ## the fleet's long-standing default, so a peer that asks for a mode
    ## this build does not know still gets a working session; a caller
    ## that cares can compare `name(from_name(text))` with its request.
    from_name : Str -> Mode
    from_name = |text| match text {
        "primary" => Primary
        "text" => Text
        _ => Alternate
    }
}

Term := {}.{
    ## One frame transition in primary mode, as a single ANSI byte
    ## stream a host can write verbatim: for each maximal run of changed
    ## cells in a row that share a style, a cursor address
    ## (`ESC[row;colH`, 1-based), a style reset plus the run's SGR, and
    ## the run's text. A run of default-styled spaces that reaches the
    ## row's last column is an erase-in-line (`ESC[K`) instead of
    ## literal spaces. Identical frames paint as the empty string, and
    ## no alternate-screen sequence is ever emitted. A size change
    ## between the frames diffs blank-padded (see `Screen.diff_cells`);
    ## hosts should answer a size change with `paint_primary_full`.
    paint_primary : Screen, Screen -> Str
    paint_primary = |prev, next| {
        width = Screen.dimensions(next).width
        start = { out: "", open: Bool.False, x: 0, y: 0, len: 0, text: "", spaces: Bool.True, fg: 0, bg: 0, bold: Bool.False }
        folded = List.fold(Screen.diff_cells(prev, next), start, |acc, cell| {
            extends = acc.open and cell.y == acc.y and cell.x == acc.x + acc.len and cell.fg == acc.fg and cell.bg == acc.bg and cell.bold == acc.bold
            if extends {
                { out: acc.out, open: Bool.True, x: acc.x, y: acc.y, len: acc.len + 1, text: Str.concat(acc.text, cell.ch), spaces: acc.spaces and cell.ch == " ", fg: acc.fg, bg: acc.bg, bold: acc.bold }
            } else {
                { out: Str.concat(acc.out, emit_run(acc, width)), open: Bool.True, x: cell.x, y: cell.y, len: 1, text: cell.ch, spaces: cell.ch == " ", fg: cell.fg, bg: cell.bg, bold: cell.bold }
            }
        })
        out = Str.concat(folded.out, emit_run(folded, width))
        if out == "" {
            ""
        } else {
            Str.concat(out, "\u(1b)[0m")
        }
    }

    ## A full primary-mode repaint: clear the screen, then paint the
    ## frame over a blank one. Hosts use this on join and on resize,
    ## and `paint_primary` for every frame after.
    paint_primary_full : Screen -> Str
    paint_primary_full = |next| {
        dims = Screen.dimensions(next)
        Str.concat("\u(1b)[2J", paint_primary(Screen.new(dims.width, dims.height), next))
    }

    emit_run : { open : Bool, x : U64, y : U64, len : U64, text : Str, spaces : Bool, fg : U8, bg : U8, bold : Bool }, U64 -> Str
    emit_run = |run, width| {
        if !run.open {
            ""
        } else {
            cursor = "\u(1b)[${U64.to_str(run.y + 1)};${U64.to_str(run.x + 1)}H"
            if run.spaces and run.fg == 0 and run.bg == 0 and run.bold == Bool.False and run.x + run.len == width {
                Str.concat(cursor, "\u(1b)[K")
            } else {
                Str.concat(cursor, Str.concat("\u(1b)[0m", Str.concat(sgr(run.fg, run.bg, run.bold), run.text)))
            }
        }
    }

    sgr : U8, U8, Bool -> Str
    sgr = |fg, bg, bold| {
        weight = if bold { "\u(1b)[1m" } else { "" }
        foreground = if fg == 0 { "" } else { "\u(1b)[38;5;${U8.to_str(fg)}m" }
        background = if bg == 0 { "" } else { "\u(1b)[48;5;${U8.to_str(bg)}m" }
        Str.concat(weight, Str.concat(foreground, background))
    }

    ## A frame as text-mode lines: the plain rendering with trailing
    ## spaces removed from every row (trailing blanks carry no meaning
    ## in text mode, and dropping them is most of the write saving).
    text_lines : Screen -> List(Str)
    text_lines = |screen| List.map(Str.split_on(Screen.render_plain(screen), "\n"), trim_end)

    ## The lines a text-mode presenter emits for one transition: for
    ## every row whose text changed, its row index and new text (an
    ## empty text clears the row). `prev_lines` is the previous frame's
    ## `text_lines`; rows past either frame's height count as empty, so
    ## a shrinking frame still clears what it leaves behind.
    diff_lines : List(Str), Screen -> List({ row : U64, text : Str })
    diff_lines = |prev_lines, next| diff_walk(prev_lines, text_lines(next), 0, [])

    ## Lockstep walk of the two line lists. Positional `List.get` on a
    ## `Str.split_on` product is avoided on purpose: the pinned nightly
    ## mis-reads elements past index 1 of such lists in some module
    ## compositions (see this package's README caveat).
    diff_walk : List(Str), List(Str), U64, List({ row : U64, text : Str }) -> List({ row : U64, text : Str })
    diff_walk = |prev_lines, next_lines, row, acc| {
        if List.is_empty(prev_lines) and List.is_empty(next_lines) {
            acc
        } else {
            before = List.first(prev_lines) ?? ""
            after = List.first(next_lines) ?? ""
            next_acc = if before == after { acc } else { List.append(acc, { row: row, text: after }) }
            diff_walk(tail(prev_lines), tail(next_lines), row + 1, next_acc)
        }
    }

    tail : List(Str) -> List(Str)
    tail = |lines| if List.is_empty(lines) { [] } else { List.sublist(lines, { start: 1, len: List.len(lines) - 1 }) }

    trim_end : Str -> Str
    trim_end = |text| {
        bytes = Str.to_utf8(text)
        end = trim_end_len(bytes, List.len(bytes))
        match Str.from_utf8(List.sublist(bytes, { start: 0, len: end })) {
            Ok(trimmed) => trimmed
            Err(_) => text
        }
    }

    trim_end_len : List(U8), U64 -> U64
    trim_end_len = |bytes, end| {
        if end == 0 {
            0
        } else {
            match List.get(bytes, end - 1) {
                Ok(32) => trim_end_len(bytes, end - 1)
                _ => end
            }
        }
    }
}

expect Mode.name(Alternate) == "alternate"
expect Mode.name(Primary) == "primary"
expect Mode.name(Text) == "text"
expect Mode.from_name("primary") == Primary
expect Mode.from_name("text") == Text
expect Mode.from_name("alternate") == Alternate
expect Mode.from_name("carrier-pigeon") == Alternate
expect Term.text_lines(Screen.put_text(Screen.new(4, 2), 1, 0, "hi", { fg: 0, bg: 0, bold: Bool.False })) == [" hi", ""]
expect Term.diff_lines([" hi", ""], Screen.put_text(Screen.new(4, 2), 1, 0, "hi", { fg: 0, bg: 0, bold: Bool.False })) == []
expect Term.diff_lines([" hi", ""], Screen.put_text(Screen.put_text(Screen.new(4, 2), 1, 0, "hi", { fg: 0, bg: 0, bold: Bool.False }), 1, 1, "yo", { fg: 0, bg: 0, bold: Bool.False })) == [{ row: 1, text: " yo" }]
expect Term.diff_lines(["a", "b"], Screen.new(1, 1)) == [{ row: 0, text: "" }, { row: 1, text: "" }]
expect Term.paint_primary(Screen.new(3, 1), Screen.new(3, 1)) == ""
expect Term.paint_primary(Screen.new(3, 1), Screen.put_text(Screen.new(3, 1), 1, 0, "x", { fg: 0, bg: 0, bold: Bool.False })) == "\u(1b)[1;2H\u(1b)[0mx\u(1b)[0m"
expect Term.paint_primary(Screen.put_text(Screen.new(3, 1), 0, 0, "abc", { fg: 0, bg: 0, bold: Bool.False }), Screen.new(3, 1)) == "\u(1b)[1;1H\u(1b)[K\u(1b)[0m"
expect Term.paint_primary(Screen.new(2, 1), Screen.put_text(Screen.new(2, 1), 0, 0, "a", { fg: 196, bg: 0, bold: Bool.True })) == "\u(1b)[1;1H\u(1b)[0m\u(1b)[1m\u(1b)[38;5;196ma\u(1b)[0m"
expect Term.paint_primary_full(Screen.put_text(Screen.new(2, 1), 0, 0, "a", { fg: 0, bg: 0, bold: Bool.False })) == "\u(1b)[2J\u(1b)[1;1H\u(1b)[0ma\u(1b)[0m"
