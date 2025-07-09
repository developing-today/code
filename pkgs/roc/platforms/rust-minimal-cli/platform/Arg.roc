module [
    from_os_raw,
]
Arg := [Unix (List U8), Windows (List U16)]
from_os_raw : [Unix (List U8), Windows (List U16)] -> Arg
from_os_raw = @Arg
