## The roc-cap-demo library bound to the reduced platform
## (roc-world/platform-no-caps.roc), which does NOT provide the
## RocCap capability module. Negative test A (#5957): Greeter.roc
## imports pf.RocCap, so anything using this package must fail to
## compile — the capability is not on the platform.
package [Greeter] { pf: platform "../../roc-world/platform-no-caps.roc" }
