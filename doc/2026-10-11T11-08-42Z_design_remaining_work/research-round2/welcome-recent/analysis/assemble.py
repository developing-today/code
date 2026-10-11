# Builds RESULTS.md from RESULTS-body.md, splicing in the generated tables so no number is retyped.
import subprocess

def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout

tables = run(["python3", "-I", "analysis/tables.py"])
part_a, part_b = tables.split("### Table B")
part_a = part_a.split("\n", 1)[1].strip()
part_b = part_b.split("\n", 1)[1].strip()
extrap = run(["python3", "-I", "analysis/extrap_table.py"]).strip()

body = open("RESULTS-body.md").read()
for placeholder in ["{{TABLE_A}}", "{{TABLE_B}}", "{{EXTRAP}}"]:
    assert body.count(placeholder) == 1, placeholder
body = body.replace("{{TABLE_A}}", part_a).replace("{{TABLE_B}}", part_b).replace("{{EXTRAP}}", extrap)
open("RESULTS.md", "w").write(body)
print("RESULTS.md written:", body.count("\n") + 1, "lines")
