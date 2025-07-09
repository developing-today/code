module [
    Command,
    Output,
    OutputFromHost,
    from_host_output,
]
import InternalIOErr
Command : {
    program : Str,
    args : List Str,
    envs : List Str,
    clear_envs : Bool,
}
Output : {
    status : Result I32 InternalIOErr.IOErr,
    stdout : List U8,
    stderr : List U8,
}
from_host_output : OutputFromHost -> Output
from_host_output = |{ status, stdout, stderr }| {
    status: Result.map_err(status, InternalIOErr.handle_err),
    stdout,
    stderr,
}
OutputFromHost : {
    status : Result I32 InternalIOErr.IOErrFromHost,
    stdout : List U8,
    stderr : List U8,
}
