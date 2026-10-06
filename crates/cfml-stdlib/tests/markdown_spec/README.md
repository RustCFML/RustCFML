# Markdown spec corpora (test data only)

Used by `crates/cfml-stdlib/tests/markdown_spec.rs`. Not compiled into any binary.

| File | Source | Licence |
|---|---|---|
| `commonmark-0.31.2.json` | https://spec.commonmark.org/0.31.2/spec.json (652 examples) | CC-BY-SA 4.0, John MacFarlane |
| `gfm-extensions.txt` | https://github.com/github/cmark-gfm/blob/master/test/extensions.txt (tables, strikethrough, autolinks, tag filter, footnotes, task lists) | CC-BY-SA 4.0 |

The tests use only each example's markdown input. Expected output comes from comrak
itself, so these files check that our tree loses nothing, not that comrak follows
the spec (comrak's own test suite does that).
