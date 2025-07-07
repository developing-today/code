app [main] {
    pf: platform "../../../platforms/go-basic-cli/main.roc",
}
import Lib.Display exposing [str, matrix]
main : Str
main = str(matrix [261, 23])
