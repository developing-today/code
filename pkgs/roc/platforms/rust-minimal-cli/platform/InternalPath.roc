module [
    UnwrappedPath,
    InternalPath,
    InternalPathType,
    wrap,
    unwrap,
    to_bytes,
    from_arbitrary_bytes,
    from_os_bytes,
]
InternalPath := UnwrappedPath implements [Inspect]
UnwrappedPath : [
    FromOperatingSystem (List U8),
    ArbitraryBytes (List U8),
    FromStr Str,
]
InternalPathType : {
    is_file : Bool,
    is_sym_link : Bool,
    is_dir : Bool,
}
wrap : UnwrappedPath -> InternalPath
wrap = @InternalPath
unwrap : InternalPath -> UnwrappedPath
unwrap = |@InternalPath(raw)| raw
to_bytes : InternalPath -> List U8
to_bytes = |@InternalPath(path)|
    when path is
        FromOperatingSystem(bytes) -> bytes
        ArbitraryBytes(bytes) -> bytes
        FromStr(str) -> Str.to_utf8(str)
from_arbitrary_bytes : List U8 -> InternalPath
from_arbitrary_bytes = |bytes|
    @InternalPath(ArbitraryBytes(bytes))
from_os_bytes : List U8 -> InternalPath
from_os_bytes = |bytes|
    @InternalPath(FromOperatingSystem(bytes))
