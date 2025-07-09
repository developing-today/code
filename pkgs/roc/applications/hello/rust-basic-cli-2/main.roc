app [main!] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello exposing [str]
hello : Str -> Str
hello = str
import pf.Stdout exposing [line!, hello!]
language = "Rust"
user = "User"
main! = |_|
    line!(hello language)?
    line!(Str.join_with(["Roc ❤️", language], "  "))?
    line!(hello!(user)?)?
    Ok({})
