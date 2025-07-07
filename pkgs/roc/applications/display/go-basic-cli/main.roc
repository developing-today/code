app [main] {
    pf: platform "../../../platforms/go-basic-cli/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Display exposing [str, matrix]
main : Str
main = str(matrix [261, 23])
