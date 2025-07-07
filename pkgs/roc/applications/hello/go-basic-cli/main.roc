app [main] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello
main : Str
main = Hello.str "Go"
