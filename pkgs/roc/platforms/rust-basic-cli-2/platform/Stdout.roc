module [
    IOErr,
    line!,
    write!,
    write_bytes!,
    hello!,
]
import Host
import InternalIOErr
IOErr : [
    NotFound,
    PermissionDenied,
    BrokenPipe,
    AlreadyExists,
    Interrupted,
    Unsupported,
    OutOfMemory,
    Other Str,
]
handle_err : InternalIOErr.IOErrFromHost -> [StdoutErr IOErr]
handle_err = |{ tag, msg }|
    when tag is
        NotFound -> StdoutErr(NotFound)
        PermissionDenied -> StdoutErr(PermissionDenied)
        BrokenPipe -> StdoutErr(BrokenPipe)
        AlreadyExists -> StdoutErr(AlreadyExists)
        Interrupted -> StdoutErr(Interrupted)
        Unsupported -> StdoutErr(Unsupported)
        OutOfMemory -> StdoutErr(OutOfMemory)
        Other | EndOfFile -> StdoutErr(Other(msg))
line! : Str => Result {} [StdoutErr IOErr]
line! = |str|
    Host.stdout_line!(str)
    |> Result.map_err(handle_err)
write! : Str => Result {} [StdoutErr IOErr]
write! = |str|
    Host.stdout_write!(str)
    |> Result.map_err(handle_err)
write_bytes! : List U8 => Result {} [StdoutErr IOErr]
write_bytes! = |bytes|
    Host.stdout_write_bytes!(bytes)
    |> Result.map_err(handle_err)
hello! : Str => Result Str [StdoutErr IOErr]
hello! = |str|
    Host.hello!(str)
    |> Result.map_err(handle_err)
