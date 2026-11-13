from pathlib import Path

report = Path(__file__).with_name('B11-research-diamond.md')
text = report.read_text()
required_urls = ['https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html', 'https://github.com/josephg/egwalker-paper', 'https://docs.rs/diamond-types/latest/diamond_types/']
required_terms = ['plausible referent', 'text workload', 'authority', 'Options', 'Conclusion']
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), "incomplete scoped research decision"
print("research artifact complete:", report.name)
