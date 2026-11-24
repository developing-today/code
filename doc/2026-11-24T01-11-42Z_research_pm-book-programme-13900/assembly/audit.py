import pathlib, re, sys

D = pathlib.Path("pm-stack/book")


def h1s(path):
    out = []
    infence = False
    for line in path.read_text().splitlines():
        if line.strip().startswith("```"):
            infence = not infence
        elif not infence and line.startswith("# "):
            out.append(line[2:].strip())
    return out


files = sorted(D.rglob("*.md"))
h1map = {}
problems = []
for f in files:
    hs = h1s(f)
    if f.name == "SUMMARY.md":
        continue
    if len(hs) != 1:
        problems.append(f"H1 count {len(hs)} in {f.relative_to(D)}")
    else:
        h1map[str(f.relative_to(D))] = hs[0]
link_re = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
missing = []
bad = []
for f in files:
    rel = str(f.relative_to(D))
    text = re.sub(r"```.*?```", "", f.read_text(), flags=re.S)
    for label, target in link_re.findall(text):
        if target.startswith(("http", "#", "mailto:")):
            continue
        t = target.split("#")[0]
        if not t:
            continue
        dest = f.parent / t
        try:
            destrel = str(dest.resolve().relative_to(D.resolve()))
        except ValueError:
            problems.append(f"escapes: {rel} -> {target}")
            continue
        if not (D / destrel).exists():
            missing.append(f"{rel} -> {target}")
        elif destrel in h1map and label != h1map[destrel]:
            bad.append(f"{rel}: {label!r} != {h1map[destrel]!r}")
# SUMMARY exactness
summ = (D / "SUMMARY.md").read_text()
entries = re.findall(r"^- \[(.+)\]\((.+)\)", summ, re.M)
listed = {t for _, t in entries}
ondisk = {str(f.relative_to(D)) for f in files if f.name != "SUMMARY.md"}
print("chapters:", len(ondisk), "summary entries:", len(entries))
print("summary<->files delta:", sorted(listed ^ ondisk))
print("summary text mismatches:", [(tx, t) for tx, t in entries if h1map.get(t) != tx])
print("H1 problems:", problems)
print("missing targets:", missing)
print("link-text mismatches:", bad)
sys.exit(1 if (problems or missing or bad or listed != ondisk) else 0)
