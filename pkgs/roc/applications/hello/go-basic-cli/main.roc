app [main] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello exposing [hello]
main = hello "Go"
