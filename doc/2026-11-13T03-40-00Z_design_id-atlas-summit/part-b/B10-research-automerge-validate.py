from pathlib import Path

report = Path(__file__).with_name('B10-research-automerge.md')
text = report.read_text()
required_urls = ['https://automerge.org/docs/reference/concepts/', 'https://automerge.org/docs/reference/under-the-hood/merge-rules/', 'https://automerge.org/automerge/automerge/sync/index.html']
required_terms = ['history', 'convergence', 'signature', 'interop', 'Options', 'Conclusion']
assert all(url in text for url in required_urls), "missing cited source"
assert all(term.lower() in text.lower() for term in required_terms), "incomplete scoped research decision"
print("research artifact complete:", report.name)
