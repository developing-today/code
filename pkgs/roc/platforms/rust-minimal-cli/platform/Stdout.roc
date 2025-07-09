module [
    line!,
    hello!,
]
import Host
import InternalIOErr
handle_err : InternalIOErr.IOErrFromHost -> [StdoutErr InternalIOErr.IOErr]
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
line! : Str => Result {} [StdoutErr InternalIOErr.IOErr]
line! = |str|
    Host.stdout_line!(str)
    |> Result.map_err(handle_err)
hello! : Str => Result Str [StdoutErr InternalIOErr.IOErr]
hello! = |str|
    Host.hello!(str)
    |> Result.map_err(handle_err)
