// Explicitly published browser assets; never serve arbitrary files by name.
export const assets = Object.freeze({
 '/': 'index.html', '/access-ui.js': 'access-ui.js', '/app.js': 'app.js', '/catalog.js': 'catalog.js',
 '/evidence.js': 'evidence.js', '/summary.js': 'summary.js',
 '/decision.js': 'decision.js', '/steering.js': 'steering.js',
 '/variants.js': 'variants.js', '/reporting.js': 'reporting.js',
 '/reporting-ui.js': 'reporting-ui.js', '/seed.js': 'seed.js', '/style.css': 'style.css',
});
export const assetType = file => file.endsWith('.html') ? 'text/html; charset=utf-8' :
 file.endsWith('.css') ? 'text/css; charset=utf-8' : 'application/javascript; charset=utf-8';
