hosted [
    stdout_line!,
    hello!,
]
import InternalIOErr
stdout_line! : Str => Result {} InternalIOErr.IOErrFromHost
hello! : Str => Result Str InternalIOErr.IOErrFromHost
