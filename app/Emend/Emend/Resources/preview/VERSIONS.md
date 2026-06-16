# Vendored preview assets

These assets are bundled so the WKWebView preview renders **fully offline** (no
CDN, no remote loads — Constitution II / SC-008). Versions are pinned and
verified against the npm registry (Constitution VII — never bump from memory).

| Asset | Version | Source |
|-------|---------|--------|
| Mermaid (`mermaid.min.js`) | 11.15.0 | `npm pack mermaid@11.15.0` → `dist/mermaid.min.js` |
| KaTeX (`katex/`) | 0.17.0 | `npm pack katex@0.17.0` → `dist/{katex.min.js,katex.min.css,fonts/*}` |

KaTeX's `katex.min.css` references its fonts with relative `fonts/…` URLs, so the
CSS and `fonts/` directory are kept co-located under `katex/`. All three font
formats (woff2/woff/ttf) are shipped unmodified so the stylesheet resolves with
zero 404s.

## Regenerating

```bash
TMP=$(mktemp -d) && cd "$TMP"
npm pack mermaid@11.15.0 katex@0.17.0
mkdir m k && tar xzf mermaid-*.tgz -C m && tar xzf katex-*.tgz -C k
DEST=app/Emend/Emend/Resources/preview
cp m/package/dist/mermaid.min.js  "$DEST/mermaid.min.js"
cp k/package/dist/katex.min.js    "$DEST/katex/katex.min.js"
cp k/package/dist/katex.min.css   "$DEST/katex/katex.min.css"
cp k/package/dist/fonts/*         "$DEST/katex/fonts/"
```

To bump a version, change it here and in the command above, re-run, and confirm
the offline preview still renders (Phase 6 test T083: zero network access).
