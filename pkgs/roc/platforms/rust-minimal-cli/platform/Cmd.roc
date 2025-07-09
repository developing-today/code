module [
    exec!, # build.roc
]

import InternalCmd
import InternalIOErr
import Host
Cmd := InternalCmd.Command
new : Str -> Cmd
new = |program|
    @Cmd(
        {
            program,
            args: [],
            envs: [],
            clear_envs: Bool.false,
        },
    )
args : Cmd, List Str -> Cmd
args = |@Cmd(cmd), values|
    @Cmd({ cmd & args: List.concat(cmd.args, values) })
status! : Cmd => Result I32 [CmdStatusErr InternalIOErr.IOErr]
status! = |@Cmd(cmd)|
    Host.command_status!(cmd)
    |> Result.map_err(InternalIOErr.handle_err)
    |> Result.map_err(CmdStatusErr)
exec! : Str, List Str => Result {} [CmdStatusErr InternalIOErr.IOErr]
exec! = |program, arguments|
    exit_code =
        new(program)
        |> args(arguments)
        |> status!?
    if exit_code == 0i32 then
        Ok({})
    else
        Err(CmdStatusErr(Other("Non-zero exit code ${Num.to_str(exit_code)}")))
