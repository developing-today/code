import pathlib

base = pathlib.Path.home() / "wt-mcpx-worker-book/pkgs/mcpx"
s = base / "docs/SUMMARY.md"
t = s.read_text()
old = "- [Reaching a daemon on another machine](remote.md)\n"
assert t.count(old) == 1, "summary anchor"
if "worker.md" not in t:
    s.write_text(
        t.replace(old, old + "- [The door Worker at mcpx.security.cab](worker.md)\n")
    )

m = base / "docs/compendium.manifest.json"
j = m.read_text()
anchor = '      "note": "MCPX issue map for program #13264"\n    }\n  ]\n}'
assert j.count(anchor) == 1, "manifest anchor"
new = """      "note": "MCPX issue map for program #13264"
    },
    {
      "path": "docs/worker.md",
      "chapter": "designs",
      "role": "chapter-source",
      "lines": 350,
      "note": "the door Worker at mcpx.security.cab (#13657): the OAuth/bearer gate, the ns selector enforced at the JSON-RPC layer, the Access and tunnel chain to the laptop bridge, and the door's related work in order"
    },
    {
      "path": "worker/README.md",
      "chapter": null,
      "role": "not-compendium",
      "lines": 171,
      "note": "operational readme for the Worker package in pkgs/mcpx/worker; its book chapter is docs/worker.md, which carries the same surface in book form"
    }
  ]
}"""
if "docs/worker.md" not in j:
    m.write_text(j.replace(anchor, new))
print("EDITED_OK")
