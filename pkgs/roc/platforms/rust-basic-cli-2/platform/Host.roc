hosted [
    command_status!,
    current_arch_os!,
    env_var!,
    stderr_line!,
    stderr_write!,
    stderr_write_bytes!,
    stdout_line!,
    stdout_write!,
    stdout_write_bytes!,
    temp_dir!,
    hello!,
]
import InternalCmd
import InternalIOErr
command_status! : InternalCmd.Command => Result I32 InternalIOErr.IOErrFromHost
temp_dir! : {} => List U8
stdout_line! : Str => Result {} InternalIOErr.IOErrFromHost
stdout_write! : Str => Result {} InternalIOErr.IOErrFromHost
stdout_write_bytes! : List U8 => Result {} InternalIOErr.IOErrFromHost
stderr_line! : Str => Result {} InternalIOErr.IOErrFromHost
stderr_write! : Str => Result {} InternalIOErr.IOErrFromHost
stderr_write_bytes! : List U8 => Result {} InternalIOErr.IOErrFromHost
current_arch_os! : {} => { arch : Str, os : Str }
env_var! : Str => Result Str {}
hello! : Str => Result Str InternalIOErr.IOErrFromHost
