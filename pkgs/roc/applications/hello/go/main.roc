app [main] { pf: platform "../../../platforms/go/main.roc" }

# Can segfault on some Ubuntu 20.04 CI machines, see #164.
main : Str
main = "Roc <3 Go!\n"
