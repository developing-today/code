app [main!] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello
import pf.Stdout
import pf.Arg
language = "Rust"
user = "User"
main! : List Arg.Arg => Result {} [Exit I32 Str]_
main! = |_|
    Stdout.line!(Hello.str language)?
    Stdout.line!(Str.join_with(["Roc ❤️", language], "  "))?
    Stdout.line!(Stdout.hello!(user)?)?
    # Stdout.write!("Hi")?
    Ok({})
