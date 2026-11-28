from pathlib import Path

report = Path(__file__).with_name("B12-research-willow.md")
text = report.read_text()
required_urls = [
    "https://willowprotocol.org/specs/data-model/",
    "https://willowprotocol.org/specs/willow25/index.html",
    "https://willowprotocol.org/specs/confidential-sync/",
]
required_terms = ["join semantics", "paused", "authorization", "Options", "Conclusion"]
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), (
    "incomplete scoped research decision"
)
print("research artifact complete:", report.name)
