app [main] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Display exposing [to_str, matrix]
main = to_str(matrix [261, 23])
