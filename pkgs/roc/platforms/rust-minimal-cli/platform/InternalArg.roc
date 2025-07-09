module [to_os_raw]
ArgToAndFromHost := {
    type : [Unix, Windows],
    unix : List U8,
    windows : List U16,
}
to_os_raw : ArgToAndFromHost -> [Unix (List U8), Windows (List U16)]
to_os_raw = |@ArgToAndFromHost(inner)|
    when inner.type is
        Unix -> Unix(inner.unix)
        Windows -> Windows(inner.windows)
