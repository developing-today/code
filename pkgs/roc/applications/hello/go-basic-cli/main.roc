app [main] {
    pf: platform "../../../platforms/go-basic-cli/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello
main : Str
main = Hello.str "Go"
