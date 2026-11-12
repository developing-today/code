## Package root for the roc-cap-demo demo library.
##
## The library (Greeter.roc) depends on the world platform: it uses
## the platform-provided RocCap/RocJson modules, so this package can
## only be used with a platform that provides them. ../main.roc is
## the demo app that imports it; ../lib-no-caps is the same library
## bound to a reduced platform, for the negative tests.
package [Greeter] { pf: platform "../../roc-world/platform.roc" }
