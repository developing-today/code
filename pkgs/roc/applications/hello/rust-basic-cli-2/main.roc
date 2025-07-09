app [main!] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import pf.Stdout exposing [line!, hello!]
import lib.Hello exposing [str]
hello = str
language = "Rust"
user = "User"
main! = |_|
    line!(hello language)?
    line!("Roc ❤️ ${language}")?
    line!(hello!(user)?)?
    Ok({})
