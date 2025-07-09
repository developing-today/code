module [
    Arg,
    display,
    to_os_raw,
    from_os_raw,
]
Arg := [Unix (List U8), Windows (List U16)]
    implements [Eq, Inspect { to_inspector: arg_inspector }]
arg_inspector : Arg -> Inspector f where f implements InspectFormatter
arg_inspector = |arg| Inspect.str(display(arg))
test_hello : Arg
test_hello = Arg.from_os_raw(Unix([72, 101, 108, 108, 111]))
expect Arg.display(test_hello) == "Hello"
expect Inspect.to_str(test_hello) == "\"Hello\""
to_os_raw : Arg -> [Unix (List U8), Windows (List U16)]
to_os_raw = |@Arg(inner)| inner
from_os_raw : [Unix (List U8), Windows (List U16)] -> Arg
from_os_raw = @Arg
display : Arg -> Str
display = |@Arg(inner)|
    when inner is
        Unix(bytes) ->
            # TODO replace with Str.from_utf8_lossy : List U8 -> Str
            # see https://github.com/roc-lang/roc/issues/7390
            when Str.from_utf8(bytes) is
                Ok(str) -> str
                Err(_) -> crash("tried to display Arg containing invalid utf-8")

        Windows(_) ->
            # TODO replace with Str.from_utf16_lossy : List U16 -> Str
            # see https://github.com/roc-lang/roc/issues/7390
            crash("display for utf-16 Arg not yet supported")
