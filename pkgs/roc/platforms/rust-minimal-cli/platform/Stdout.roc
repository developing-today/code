module [
    line!,
    hello!,
]
import Host
import InternalIOErr
handle_err = |io_err_from_host| StdoutErr InternalIOErr.handle_err(io_err_from_host)
line! = |str|
    Host.stdout_line!(str)
    |> Result.map_err(handle_err)
hello! = |str|
    Host.hello!(str)
    |> Result.map_err(handle_err)
