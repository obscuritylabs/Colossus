"""Offline fidelity, completeness, and failure-path checks for the shipped guide."""

import contextlib
import gzip
from html.parser import HTMLParser
import io
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import google_style_guide as guide


def html(article, summary="generated summary"):
    return f'''<html><head><link rel="canonical" href="{guide.START}"></head>
<body><nav><a class="devsite-nav-title" href="/style">About</a></nav>
<h1>Original title<devsite-actions>Site chrome</devsite-actions></h1>
<div class="devsite-article-body clearfix">
<devsite-key-takeaways-panel>{summary}</devsite-key-takeaways-panel>{article}</div>
<devsite-thumb-rating></devsite-thumb-rating>
<a href="https://creativecommons.org/licenses/by/4.0/">CC BY 4.0</a>
<a href="https://www.apache.org/licenses/LICENSE-2.0">Apache 2.0</a>
<p>Last updated 2026-11-03 UTC.</p></body></html>'''


class Text(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.parts = []
        self.feed(source)

    def handle_data(self, data):
        self.parts.append(data)


class GuideTests(unittest.TestCase):
    def test_extraction_preserves_every_original_article_byte_even_malformed_html(self):
        article = '''<section id="task"><h2>Task &amp; concept</h2>
<p>Keep this text.</p><table><tr><td>Recommended</td><td>Not recommended</td></tr></table>
<pre>&lt;script&gt;example&lt;/script&gt;</pre><!-- original comment -->
<aside>Exception.</aside></p></div></section>'''
        page = guide.Page(html(article))
        self.assertEqual(page.article(), "\n" + article)
        self.assertEqual("".join(page.title), "Original title")
        self.assertEqual(page.updated, "2026-11-03")
        self.assertEqual(page.navigation, [guide.START])
        with self.assertRaisesRegex(ValueError, "footer"):
            guide.Page(html(article).replace("devsite-thumb-rating", "unknown-footer"))

    def test_scope_annotations_add_labels_without_rewriting_source(self):
        source = '<p>é <span class="icon-cloud"></span>A &amp; B.</p>\n' \
                 '<span class="icon-android"></span><span class="icon-workspace"></span>'
        expected = source.replace('class="icon-cloud">', 'class="icon-cloud">[Google Cloud only]') \
            .replace('class="icon-android">', 'class="icon-android">[Android only]') \
            .replace('class="icon-workspace">', 'class="icon-workspace">[Google Workspace only]')
        self.assertEqual(guide.Annotate(source).text(), expected)

    def test_parts_reassemble_losslessly_and_respect_utf8_byte_budget(self):
        text = ("é" * 3000 + "\n") * 20 + "Final line without newline"
        parts = guide.chunks(text)
        self.assertGreater(len(parts), 1)
        self.assertEqual("".join(parts), text)
        self.assertTrue(all(len(part.encode()) <= guide.CHUNK_BYTES for part in parts))
        with self.assertRaisesRegex(ValueError, "line exceeds"):
            guide.chunks("é" * guide.CHUNK_BYTES)

    def test_shipped_snapshot_is_complete_offline_and_fits_real_tool_envelopes(self):
        with patch.object(guide, "urlopen", side_effect=AssertionError("network forbidden")):
            with contextlib.redirect_stdout(io.StringIO()):
                guide.check(guide.SKILL, guide.SOURCE_ROOT)
        self.assertFalse((guide.SKILL / "upstream").exists())
        manifest = json.loads((guide.SKILL / "snapshot.json").read_text(encoding="utf-8"))
        self.assertGreaterEqual(len(manifest["pages"]), 70)
        for entry in manifest["pages"]:
            parts = [(guide.SKILL / path).read_text(encoding="utf-8") for path in entry["resources"]]
            source = gzip.decompress((guide.SOURCE_ROOT / entry["source"]).read_bytes()).decode("utf-8")
            original = guide.Page(source).article()
            # Remove only the mechanical scope labels and provenance/title wrapper.
            reconstructed = "".join(parts).split("</h1>\n", 1)[1]
            for label in guide.ICONS.values():
                reconstructed = reconstructed.replace(f"[{label} only]", "")
            self.assertEqual(reconstructed, original, entry["url"])
            for path, content in zip(entry["resources"], parts):
                envelope = json.dumps({"skill_id": "colossus/documentation", "path": path,
                                       "size": len(content.encode()), "content": content}).encode()
                self.assertLess(len(envelope), 64 * 1024, path)
        headings = next(e for e in manifest["pages"] if e["url"].endswith("/headings"))
        text = " ".join(Text("".join(
            (guide.SKILL / p).read_text(encoding="utf-8") for p in headings["resources"]
        )).parts)
        for guidance in ["bare infinitive", "noun phrase", "Billing", "Pricing", "Mixed heading styles"]:
            self.assertIn(guidance, text)

    def test_missing_modified_or_truncated_material_fails_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            skill = Path(temporary) / "skill"
            source_root = Path(temporary) / "sources"
            shutil.copytree(guide.SKILL, skill)
            shutil.copytree(guide.SOURCE_ROOT / "upstream", source_root / "upstream")
            manifest_path = skill / "snapshot.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            part = skill / manifest["pages"][0]["resources"][0]
            original = part.read_bytes()
            part.write_bytes(original[:-1])
            with self.assertRaisesRegex(ValueError, "differs from source"):
                guide.check(skill, source_root)
            part.write_bytes(original)
            removed = manifest["pages"].pop()
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "missing page|missing navigation"):
                guide.check(skill, source_root)
            manifest["pages"].append(removed)
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            source_path = source_root / removed["source"]
            data = gzip.decompress(source_path.read_bytes())
            source_path.write_bytes(gzip.compress(data.replace(b"<html", b"<HTML", 1), mtime=0))
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                guide.check(skill, source_root)


if __name__ == "__main__":
    unittest.main()
