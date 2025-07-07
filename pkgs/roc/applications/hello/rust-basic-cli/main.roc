app [main!] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello
import pf.Stdout
language = "Rust"
main! : {} => Result {} _
main! = |{}|
    Stdout.line!(Hello.str language)?
    Stdout.line!(Str.join_with(["Roc ❤️", language], "  "))?
    Ok({})
