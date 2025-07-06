app [main] {
  pf: platform "../../../platforms/go/main.roc"
}
import Lib.Display exposing [str]
main : Str
main = str [
  ["Roc", "<3", "Go", "!", "!", "!"],
  ["This", "is", "a", "Roc", "application", "running", "on", "Go."],
  ["It", "imports", "a", "Roc", "module", "and", "calls", "a", "function", "from", "it."],
]
#matrix 2 3
#[
#  ["Roc", "<3", "Go", "!", "!", "!"],
#  ["This", "is", "a", "Roc", "application", "running", "on", "Go."],
#  ["It", "imports", "a", "Roc", "module", "and", "calls", "a", "function", "from", "it."],
#]
