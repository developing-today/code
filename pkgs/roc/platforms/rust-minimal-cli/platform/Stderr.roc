module [
    IOErr,
    line!,
    write!,
    write_bytes!,
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
handle_err : InternalIOErr.IOErrFromHost -> [StderrErr IOErr]
handle_err = |{ tag, msg }|
    when tag is
        NotFound -> StderrErr(NotFound)
        PermissionDenied -> StderrErr(PermissionDenied)
        BrokenPipe -> StderrErr(BrokenPipe)
        AlreadyExists -> StderrErr(AlreadyExists)
        Interrupted -> StderrErr(Interrupted)
        Unsupported -> StderrErr(Unsupported)
        OutOfMemory -> StderrErr(OutOfMemory)
        Other | EndOfFile -> StderrErr(Other(msg))
line! : Str => Result {} [StderrErr IOErr]
line! = |str|
    Host.stderr_line!(str)
    |> Result.map_err(handle_err)
write! : Str => Result {} [StderrErr IOErr]
write! = |str|
    Host.stderr_write!(str)
    |> Result.map_err(handle_err)
write_bytes! : List U8 => Result {} [StderrErr IOErr]
write_bytes! = |bytes|
    Host.stderr_write_bytes!(bytes)
    |> Result.map_err(handle_err)
