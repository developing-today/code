from pathlib import Path

report = Path(__file__).with_name("B26-research-reputation-abuse.md")
text = report.read_text()
required_urls = [
    "https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.1.md",
    "https://www.rfc-editor.org/rfc/rfc9162.html",
    "https://www.rfc-editor.org/rfc/rfc9942.html",
]
required_terms = [
    "Self-promotion",
    "Retaliation",
    "Whitewashing",
    "Collusion",
    "no public allegation alone",
]
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), (
    "incomplete scoped research decision"
)
print("research artifact complete:", report.name)
