from pathlib import Path

report = Path(__file__).with_name('B14-research-crdt-matrix.md')
text = report.read_text()
required_urls = ['https://automerge.org/docs/reference/concepts/', 'https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html', 'https://willowprotocol.org/specs/data-model/', 'https://willowprotocol.org/specs/confidential-sync/', 'https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md']
required_terms = ['Automerge', 'Diamond Types', 'Willow', 'OCapN', 'adopt none']
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), "incomplete scoped research decision"
print("research artifact complete:", report.name)
