from pathlib import Path

report = Path(__file__).with_name('B13-research-ocapn.md')
text = report.read_text()
required_urls = ['https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md', 'https://github.com/ocapn/ocapn/blob/main/implementation-guide/Implementation%20Guide.md', 'https://github.com/ocapn/ocapn/blob/main/draft-specifications/Netlayers.md']
required_terms = ['handoff', 'attenuation', 'Options', 'Conclusion']
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), "incomplete scoped research decision"
print("research artifact complete:", report.name)
