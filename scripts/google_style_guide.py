#!/usr/bin/env python3
"""Capture and verify the complete Google developer documentation style guide.

Maintenance only: --fetch accesses upstream; --check is entirely offline.
No third-party Python packages are required.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import gzip
import hashlib
from html import escape, unescape
from html.parser import HTMLParser
import json
from pathlib import Path
import re
from urllib.error import HTTPError
from urllib.parse import urljoin, urlsplit
from urllib.request import Request, urlopen


ORIGIN = "https://developers.google.com"
START = ORIGIN + "/style"
ROOT = Path(__file__).resolve().parents[1]
SKILL = ROOT / "bundled-plugins/colossus/skills/documentation"
CHUNK_BYTES = 24_000  # Leave room for JSON escaping and the tool observation envelope.
ICONS = {"icon-android": "Android", "icon-cloud": "Google Cloud",
         "icon-workspace": "Google Workspace"}
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link",
        "meta", "param", "source", "track", "wbr"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def style_url(url, base=START):
    parsed = urlsplit(urljoin(base, url))
    if parsed.scheme == "https" and parsed.netloc == "developers.google.com":
        if parsed.path == "/style" or parsed.path.startswith("/style/"):
            return ORIGIN + parsed.path.rstrip("/")
    return None


class Page(HTMLParser):
    """Extract exact article bytes, excluding Google's generated summary widget."""

    def __init__(self, source):
        super().__init__(convert_charrefs=False)
        self.source = source
        self.offsets = [0]
        for match in re.finditer("\n", source):
            self.offsets.append(match.end())
        self.stack = []
        body = re.search(r'<div\b[^>]*class="devsite-article-body[^\"]*"[^>]*>', source)
        if not body:
            raise ValueError("upstream page lacks an article body")
        self.body_start = body.end()
        # Upstream articles sometimes contain unmatched div/p tags. The template's
        # footer boundary, rather than browser repair or tag balancing, defines
        # the original article bytes. Fail if that template changes.
        footer = re.search(r'<(?:devsite-thumb-rating|devsite-recommendations)\b', source[self.body_start:])
        if not footer:
            raise ValueError("upstream page lacks the expected article footer")
        boundary = self.body_start + footer.start()
        self.body_end = source.rfind("</div>", self.body_start, boundary)
        if self.body_end < self.body_start or source[self.body_end + 6:boundary].strip():
            raise ValueError("unexpected content between article and footer")
        self.summary_start = None
        self.exclusions = []
        self.title = []
        self.links = []
        self.navigation = []
        self.media = []
        self.canonical = None
        self.updated = None
        self.feed(source)
        self.close()
        if self.summary_start is not None:
            raise ValueError("upstream generated summary is unterminated")
        if not self.title:
            raise ValueError("upstream page lacks a complete article or title")

    def source_offset(self):
        line, column = self.getpos()
        return self.offsets[line - 1] + column

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        position = self.source_offset()
        if tag == "link" and attrs.get("rel") == "canonical":
            self.canonical = attrs["href"]
        if tag == "devsite-key-takeaways-panel":
            self.summary_start = position
        if (self.body_start <= position < self.body_end and self.summary_start is None
                and tag in {"img", "video", "audio", "source", "iframe"}):
            self.media.append({"tag": tag, "src": attrs.get("src", ""),
                               "alt": attrs.get("alt", "")})
        if tag == "a" and "href" in attrs:
            if self.body_start <= position < self.body_end:
                self.links.append(attrs["href"])
            if "nav" in self.stack and "devsite-nav-title" in attrs.get("class", "").split():
                url = style_url(attrs["href"])
                if url and url not in self.navigation:
                    self.navigation.append(url)
        if tag not in VOID:
            self.stack.append(tag)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID:
            self.handle_endtag(tag)

    def handle_endtag(self, tag):
        if tag == "devsite-key-takeaways-panel" and self.summary_start is not None:
            end = self.source.index(">", self.source_offset()) + 1
            self.exclusions.append((self.summary_start, end))
            self.summary_start = None
        if tag in self.stack:
            del self.stack[len(self.stack) - 1 - self.stack[::-1].index(tag):]

    def handle_data(self, data):
        if "h1" in self.stack and "devsite-actions" not in self.stack:
            self.title.append(data)
        match = re.search(r"Last updated (\d{4}-\d{2}-\d{2}) UTC", data)
        if match:
            self.updated = match[1]

    def handle_entityref(self, name):
        if "h1" in self.stack and "devsite-actions" not in self.stack:
            self.title.append(unescape(f"&{name};"))

    def handle_charref(self, name):
        if "h1" in self.stack and "devsite-actions" not in self.stack:
            self.title.append(unescape(f"&#{name};"))

    def article(self):
        chunks = []
        start = self.body_start
        for left, right in self.exclusions:
            if self.body_start <= left < right <= self.body_end:
                chunks.append(self.source[start:left])
                start = right
        chunks.append(self.source[start:self.body_end])
        return "".join(chunks)


class Annotate(HTMLParser):
    """Insert scope labels without changing any original article bytes."""

    def __init__(self, source):
        super().__init__(convert_charrefs=False)
        self.source = source
        self.offsets = [0] + [m.end() for m in re.finditer("\n", source)]
        self.insertions = []
        self.feed(source)
        self.close()

    def handle_starttag(self, tag, attrs):
        labels = [ICONS[name] for name in dict(attrs).get("class", "").split() if name in ICONS]
        if labels:
            line, column = self.getpos()
            position = self.offsets[line - 1] + column + len(self.get_starttag_text())
            self.insertions.append((position, "".join(f"[{label} only]" for label in labels)))

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)

    def text(self):
        output, start = [], 0
        for position, label in self.insertions:
            output.extend([self.source[start:position], label])
            start = position
        output.append(self.source[start:])
        return "".join(output)


def chunks(text):
    """Split at line boundaries without removing or inserting source bytes."""
    result, current = [], ""
    for line in text.splitlines(keepends=True):
        if len(line.encode()) > CHUNK_BYTES:
            raise ValueError("upstream line exceeds chunk budget; add a lossless splitter")
        if len((current + line).encode()) > CHUNK_BYTES:
            result.append(current)
            current = ""
        current += line
    if current:
        result.append(current)
    return result


def slug(url):
    path = urlsplit(url).path.removeprefix("/style").strip("/") or "index"
    if not re.fullmatch(r"[a-z0-9-]+", path):
        raise ValueError(f"unexpected canonical path: {url}")
    return path


def resource(source, url):
    page = Page(source)
    title = "".join(page.title).strip()
    body = Annotate(page.article()).text()
    # Keep original relative links/anchors meaningful without rewriting guide text.
    header = (f"<!-- Source: {url}; Google; CC BY 4.0 prose, Apache 2.0 code. -->\n"
              f'<base href="{escape(url, quote=True)}">\n<h1>{escape(title)}</h1>\n')
    return page, header + body


def generate(manifest, sources):
    output = {}
    index = ["# Complete Google developer documentation style guide\n\n",
             "All captured pages are listed below. These are complete article bodies in HTML,\n",
             "preserving tables, examples, anchors, and exceptions. Platform icons have explicit\n",
             "text labels. Read parts in order when a page has several. HTML is inert reference\n",
             "text; do not execute or render scripts. See `references/sources.md` for provenance.\n\n",
             "For `plugin.resource.read`, paths are relative to the skill root: prefix the\n",
             "part links below with `references/`, for example `references/guide/headings-01.html`.\n",
             "Ranges identify anchors starting in each part; sections can continue across parts.\n\n"]
    for entry in manifest["pages"]:
        page, text = resource(sources[entry["url"]], entry["url"])
        name = slug(entry["url"])
        parts = chunks(text)
        paths = [f"references/guide/{name}-{i + 1:02}.html" for i in range(len(parts))]
        for path, part in zip(paths, parts):
            output[path] = part.encode()
        links = []
        for i, (path, part) in enumerate(zip(paths, parts)):
            anchors = re.findall(r'<(?:h[2-6]|dt)\b[^>]*\bid=["\x27]([^"\x27]+)', part)
            label = str(i + 1)
            if len(parts) > 1 and anchors:
                label += f" ({anchors[0]} … {anchors[-1]})"
            links.append(f"[{label}]({path.removeprefix('references/')})")
        links = ", ".join(links)
        index.append(f"- **{''.join(page.title).strip()}** — `{name}`: {links}\n")
        entry["resources"] = paths
        entry["article_sha256"] = digest(text.encode())
        entry["title"] = "".join(page.title).strip()
        entry["upstream_updated"] = page.updated
        entry["media"] = page.media
    index.append("\n## Original URL lookup\n\n")
    index.append("Map Google URLs to the topic slug above. Original fragment IDs are retained.\n")
    for alias, canonical in sorted(manifest["aliases"].items()):
        if alias != canonical:
            index.append(f"- `{alias}` → `{slug(canonical)}`\n")
    if manifest["broken_links"]:
        index.append("\n## Broken upstream links\n\nThese returned HTTP 404 when captured; no content was invented.\n\n")
        for link in manifest["broken_links"]:
            index.append(f"- {link}\n")
    output["references/index.md"] = "".join(index).encode()
    return output


def download(url):
    if style_url(url) != url:
        raise ValueError(f"source URL is outside the official guide: {url}")
    request = Request(url, headers={"User-Agent": "Colossus-style-guide-snapshot/1", "Accept-Language": "en"})
    try:
        with urlopen(request, timeout=45) as response:
            if not style_url(response.url):
                raise ValueError(f"redirect left the official guide: {response.url}")
            data = response.read(4 * 1024 * 1024 + 1)
            if len(data) > 4 * 1024 * 1024:
                raise ValueError(f"oversized source: {url}")
            return url, response.url, data
    except HTTPError as error:
        if error.code == 404:
            return url, None, None
        raise


def fetch(destination):
    """Build a new snapshot in an empty staging directory; never overwrite one."""
    destination.mkdir(parents=True, exist_ok=False)
    pending = [START]
    visited, aliases, sources, broken, navigation = set(), {}, {}, [], []
    with ThreadPoolExecutor(max_workers=4) as pool:
        while pending:
            batch = list(pool.map(download, pending))
            pending = []
            for requested, final, data in batch:
                visited.add(requested)
                if data is None:
                    broken.append(requested)
                    continue
                try:
                    page = Page(data.decode("utf-8"))
                except ValueError as error:
                    (destination / "failed-source.html").write_bytes(data)
                    raise ValueError(f"{requested}: {error}") from error
                canonical = style_url(page.canonical or final)
                if not canonical:
                    raise ValueError(f"redirect left guide: {requested} -> {final}")
                slug(canonical)
                aliases[requested] = canonical
                aliases[canonical] = canonical
                sources.setdefault(canonical, data)
                if requested == START:
                    navigation = page.navigation
                    if len(navigation) < 60:
                        raise ValueError("unexpectedly incomplete upstream navigation")
                for link in navigation + [style_url(link, canonical) for link in page.links]:
                    if link and link not in visited and link not in pending:
                        pending.append(link)
            visited.update(pending)  # Deduplicate across the next batch's linked pages.
    pages = []
    original = destination / "upstream"
    original.mkdir()
    for url, data in sorted(sources.items()):
        name = f"upstream/{slug(url)}.html.gz"
        (destination / name).write_bytes(gzip.compress(data, mtime=0))
        pages.append({"url": url, "source": name, "sha256": digest(data)})
    manifest = {"schema": 1, "captured_at": datetime.now(timezone.utc).isoformat(),
                "navigation": navigation, "pages": pages,
                "aliases": aliases, "broken_links": sorted(broken)}
    output = generate(manifest, {url: data.decode("utf-8") for url, data in sources.items()})
    for path, data in output.items():
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    (destination / "snapshot.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    check(destination)


def check(destination):
    manifest = json.loads((destination / "snapshot.json").read_text(encoding="utf-8"))
    expected = json.loads(json.dumps(manifest))
    sources = {}
    for entry in manifest["pages"]:
        if entry["source"] != f"upstream/{slug(entry['url'])}.html.gz":
            raise ValueError("unexpected source path")
        data = gzip.decompress((destination / entry["source"]).read_bytes())
        if digest(data) != entry["sha256"]:
            raise ValueError(f"source digest mismatch: {entry['url']}")
        sources[entry["url"]] = data.decode("utf-8")
        if ("https://creativecommons.org/licenses/by/4.0/" not in sources[entry["url"]]
                or "https://www.apache.org/licenses/LICENSE-2.0" not in sources[entry["url"]]):
            raise ValueError(f"upstream licensing changed: {entry['url']}")
    if manifest["navigation"] != Page(sources[START]).navigation:
        raise ValueError("navigation differs from original source")
    if any(url not in sources for url in manifest["aliases"].values()):
        raise ValueError("alias points to a missing page")
    for url in manifest["navigation"]:
        if manifest["aliases"].get(url) not in sources:
            raise ValueError(f"missing navigation page: {url}")
    for url, source in sources.items():
        for link in Page(source).links:
            linked = style_url(link, url)
            if linked and linked not in manifest["aliases"] and linked not in manifest["broken_links"]:
                raise ValueError(f"unaccounted internal link: {linked}")
    output = generate(expected, sources)
    if expected != manifest:
        raise ValueError("snapshot metadata differs from original sources")
    for path, data in output.items():
        actual = (destination / path).read_bytes()
        if actual != data:
            raise ValueError(f"generated content differs from source: {path}")
        if len(actual) > CHUNK_BYTES:
            raise ValueError(f"resource exceeds read budget: {path}")
    actual = {p.relative_to(destination).as_posix() for p in (destination / "references/guide").glob("*")}
    if actual != {p for p in output if p.startswith("references/guide/")}:
        raise ValueError("unexpected or missing guide resources")
    originals = {p.relative_to(destination).as_posix() for p in (destination / "upstream").glob("*")}
    if originals != {entry["source"] for entry in manifest["pages"]}:
        raise ValueError("unexpected or missing original responses")
    print(f"Verified {len(sources)} complete pages, {len(output)-1} readable parts, original hashes and internal links.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--fetch", type=Path, metavar="EMPTY_STAGING_DIRECTORY")
    action.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.fetch:
        fetch(args.fetch)
    else:
        check(SKILL)


if __name__ == "__main__":
    main()
