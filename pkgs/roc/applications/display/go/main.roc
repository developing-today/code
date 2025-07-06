app [main] {
  pf: platform "../../../platforms/go/main.roc"
}
import Lib.Display as Display
main : Str
main = Display.str [
  ["Roc", "<3", "Go", "!", "!", "!"],
  ["This", "is", "a", "Roc", "application", "running", "on", "Go."],
  ["It", "imports", "a", "Roc", "module", "and", "calls", "a", "function", "from", "it."],
]
