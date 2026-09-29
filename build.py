#!/usr/bin/env python3
"""Builds the notes site into _site/.

    python3 build.py                 build everything (release)
    python3 build.py --debug         faster, unoptimised wasm
    python3 build.py --skip-wasm     pages, data, scripts and search only
    python3 build.py serve           serve _site at http://localhost:8000/notes/
    python3 build.py new "A title"   create a new note from a template

Notes are Markdown files under content/. Each one becomes a static HTML page,
and the build also writes the data the list view and the 3D scene load.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import gzip
import hashlib
import html
import json
import math
import random
import re
import shutil
import struct
import subprocess
import sys
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CACHE_VERSION = "1"
WASM_NAME = "pooya_notes"
# The wasm-bindgen CLI has to match the wasm-bindgen crate in Cargo.lock exactly.
WASM_BUDGET_BYTES = 6 * 1024 * 1024

MONTHS = ["January", "February", "March", "April", "May", "June", "July",
          "August", "September", "October", "November", "December"]
RESERVED_SLUGS = {"assets", "data", "pkg", "pagefind", "tags", "feed.xml",
                  "sitemap.xml", "404.html", "index.html"}
TAG_ALIASES = {"c++": "cpp", "c#": "csharp", "f#": "fsharp"}
RTL_LANGUAGES = {"ar", "fa", "he", "ur"}
NO_TAG = 0xFFFF
MAX_TAGS_PER_NOTE = 5
MAX_BRIDGES = 200
# Runs before the page is drawn. With WebGPU the list is hidden from the first
# frame, so the scene is the first thing a visitor sees. Without it, or when
# the address asks for the list, the list shows as usual.
WAIT_FOR_SCENE = """<script>
    (function () {
      var asked = new URLSearchParams(location.search);
      if (navigator.gpu && (asked.get("view") !== "list" || asked.has("note"))) {
        document.documentElement.classList.add("waiting-for-scene");
      }
    })();
  </script>"""


class BuildError(Exception):
    pass


@dataclasses.dataclass
class Note:
    source: str  # path relative to the repository root
    slug: str
    title: str
    date: dt.date
    tags: list[str]
    summary: str
    lang: str
    body_html: str = ""
    links: list[str] = dataclasses.field(default_factory=list)
    words: int = 0
    updated: dt.date | None = None
    asset_dir: Path | None = None
    id: int = -1


# ---------------------------------------------------------------- utilities

def log(message: str) -> None:
    print(message, flush=True)


def esc(value: object) -> str:
    return html.escape(str(value), quote=True)


def slugify(value: str) -> str:
    value = value.strip().lower()
    value = re.sub(r"[^a-z0-9]+", "-", value)
    return value.strip("-")


def normalise_tag(value: str) -> str:
    value = value.strip().lower().lstrip("#")
    value = TAG_ALIASES.get(value, value)
    return slugify(value)


def long_date(value: dt.date) -> str:
    return f"{MONTHS[value.month - 1]} {value.day}, {value.year}"


def short_date(value: dt.date) -> str:
    return f"{MONTHS[value.month - 1][:3]} {value.day}"


def render_template(name: str, context: dict[str, object]) -> str:
    text = (ROOT / "templates" / name).read_text(encoding="utf-8")

    def replace(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in context:
            raise BuildError(f"templates/{name} uses '{key}', which the build does not provide")
        return str(context[key])

    return re.sub(r"\{\{\s*(\w+)\s*\}\}", replace, text)


def write(path: Path, data: str | bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(data, str):
        path.write_text(data, encoding="utf-8")
    else:
        path.write_bytes(data)


def json_for_script(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2).replace("</", "<\\/")


# -------------------------------------------------------------- front matter

def parse_scalar(raw: str) -> object:
    raw = raw.strip()
    if len(raw) >= 2 and raw[0] == raw[-1] and raw[0] in "\"'":
        return raw[1:-1]
    if raw.startswith("[") and raw.endswith("]"):
        inner = raw[1:-1].strip()
        return [str(parse_scalar(part)) for part in inner.split(",") if part.strip()] if inner else []
    if raw.lower() in ("true", "false"):
        return raw.lower() == "true"
    return raw


def parse_front_matter(text: str, source: str) -> tuple[dict[str, object], str]:
    """Reads the block between the two '---' lines at the top of a note.

    Supported: 'key: value', 'key: [a, b]', and a 'key:' line followed by
    '- item' lines.
    """
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        raise BuildError(f"{source}: the note has to start with a '---' front matter block")
    try:
        end = next(i for i in range(1, len(lines)) if lines[i].strip() == "---")
    except StopIteration:
        raise BuildError(f"{source}: the front matter block is not closed with '---'") from None

    meta: dict[str, object] = {}
    current_list: list[str] | None = None
    for number, line in enumerate(lines[1:end], start=2):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        item = re.match(r"^\s+-\s+(.*)$", line)
        if item and current_list is not None:
            current_list.append(str(parse_scalar(item.group(1))))
            continue
        pair = re.match(r"^([A-Za-z_][\w-]*):\s*(.*)$", line)
        if not pair:
            raise BuildError(f"{source}:{number}: cannot read this front matter line: {line!r}")
        key, raw = pair.group(1).lower(), pair.group(2)
        if raw.strip() == "":
            current_list = []
            meta[key] = current_list
        else:
            current_list = None
            meta[key] = parse_scalar(raw)
    return meta, "\n".join(lines[end + 1:])


def parse_date(value: object, source: str, field: str) -> dt.date:
    try:
        return dt.date.fromisoformat(str(value).strip())
    except ValueError:
        raise BuildError(f"{source}: '{field}' has to look like 2026-09-29, got {value!r}") from None


# ------------------------------------------------------------------ markdown

def make_markdown():
    from markdown_it import MarkdownIt
    from markdown_it.renderer import RendererHTML
    from mdit_py_plugins.anchors import anchors_plugin
    from mdit_py_plugins.footnote import footnote_plugin
    from mdit_py_plugins.tasklists import tasklists_plugin
    from pygments import highlight as pygments_highlight
    from pygments.formatters import HtmlFormatter
    from pygments.lexers import get_lexer_by_name
    from pygments.util import ClassNotFound

    formatter = HtmlFormatter(nowrap=True)

    def highlight(code: str, language: str, _attrs: str) -> str:
        if not language:
            return ""
        try:
            lexer = get_lexer_by_name(language)
        except ClassNotFound:
            return ""
        return pygments_highlight(code, lexer, formatter)

    md = MarkdownIt("commonmark", {"html": True, "typographer": True, "highlight": highlight})
    md.enable(["table", "strikethrough", "replacements", "smartquotes"])
    md.use(footnote_plugin).use(tasklists_plugin)
    md.use(anchors_plugin, min_level=2, max_level=4)

    def link_open(self, tokens, idx, options, env):
        token = tokens[idx]
        href = token.attrGet("href") or ""
        if re.match(r"^[a-z][a-z0-9+.-]*://", href, re.I):
            token.attrSet("target", "_blank")
            token.attrSet("rel", "noopener noreferrer")
        elif re.search(r"\.md(#.*)?$", href, re.I) and not href.startswith("/"):
            # A link to another note's Markdown file. It is resolved to that
            # note's address once every note is known.
            target, _, fragment = href.partition("#")
            resolved = normalise_path((Path(env["source"]).parent / target).as_posix())
            token.attrSet("href", f"note://{note_key(resolved)}" + (f"#{fragment}" if fragment else ""))
        return RendererHTML.renderToken(self, tokens, idx, options, env)

    def image(self, tokens, idx, options, env):
        tokens[idx].attrSet("loading", "lazy")
        tokens[idx].attrSet("decoding", "async")
        return RendererHTML.image(self, tokens, idx, options, env)

    md.add_render_rule("link_open", link_open)
    md.add_render_rule("image", image)
    return md


def normalise_path(path: str) -> str:
    parts: list[str] = []
    for part in path.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if parts:
                parts.pop()
            continue
        parts.append(part)
    return "/".join(parts)


def note_key(source: str) -> str:
    """content/2026/hello.md and content/2026/hello/index.md share one key."""
    return re.sub(r"/index\.md$", ".md", source)


def plain_text(fragment: str) -> str:
    text = re.sub(r"<[^>]+>", "", fragment)
    return re.sub(r"\s+", " ", html.unescape(text)).strip()


def render_markdown(md, body: str, source: str, cache_dir: Path) -> dict[str, object]:
    digest = hashlib.sha256(f"{CACHE_VERSION}\0{source}\0{body}".encode()).hexdigest()
    cached = cache_dir / f"{digest}.json"
    if cached.exists():
        return json.loads(cached.read_text(encoding="utf-8"))

    rendered = md.render(body, {"source": source})
    rendered = rendered.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
    first = re.search(r"<p>(.*?)</p>", rendered, re.S)
    result = {
        "html": rendered,
        "first_paragraph": plain_text(first.group(1)) if first else "",
        "words": len(plain_text(rendered).split()),
    }
    write(cached, json.dumps(result, ensure_ascii=False))
    return result


# --------------------------------------------------------------------- notes

def git_last_changed(content_dir: Path) -> dict[str, dt.date]:
    """The date of the last commit that touched each file, from one git call."""
    try:
        output = subprocess.run(
            ["git", "log", "--format=%x00%cs", "--name-only", "--", str(content_dir)],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return {}
    changed: dict[str, dt.date] = {}
    current: dt.date | None = None
    for line in output.splitlines():
        if line.startswith("\0"):
            current = dt.date.fromisoformat(line[1:].strip())
        elif line.strip() and current is not None:
            changed.setdefault(line.strip(), current)
    return changed


def load_notes(content_dir: Path, cache_dir: Path, include_drafts: bool) -> list[Note]:
    md = make_markdown()
    changed = git_last_changed(content_dir)
    notes: list[Note] = []
    slugs: dict[str, str] = {}

    for path in sorted(content_dir.rglob("*.md")):
        try:
            source = path.relative_to(ROOT).as_posix()
        except ValueError:
            source = path.as_posix()
        meta, body = parse_front_matter(path.read_text(encoding="utf-8"), source)
        if meta.get("draft") is True and not include_drafts:
            continue

        title = str(meta.get("title", "")).strip()
        if not title:
            raise BuildError(f"{source}: 'title' is missing")
        if "date" not in meta:
            raise BuildError(f"{source}: 'date' is missing")
        date = parse_date(meta["date"], source, "date")

        is_bundle = path.name == "index.md"
        stem = path.parent.name if is_bundle else path.stem
        stem = re.sub(r"^\d{4}-\d{2}-\d{2}-", "", stem)
        slug = slugify(str(meta.get("slug", stem)))
        if not slug or slug in RESERVED_SLUGS:
            raise BuildError(f"{source}: '{slug}' cannot be used as the address of a note")
        if slug in slugs:
            raise BuildError(f"{source}: the address '{slug}' is already used by {slugs[slug]}")
        slugs[slug] = source

        raw_tags = meta.get("tags", [])
        if isinstance(raw_tags, str):
            raw_tags = [raw_tags]
        tags: list[str] = []
        for raw in raw_tags:  # type: ignore[union-attr]
            tag = normalise_tag(str(raw))
            if tag and tag not in tags:
                tags.append(tag)
        if len(tags) > MAX_TAGS_PER_NOTE:
            raise BuildError(f"{source}: a note can have at most {MAX_TAGS_PER_NOTE} tags, this one has {len(tags)}")

        rendered = render_markdown(md, body, source, cache_dir)
        summary = str(meta.get("summary", "")).strip() or str(rendered["first_paragraph"])
        if len(summary) > 220:
            summary = summary[:217].rsplit(" ", 1)[0] + "…"

        note = Note(
            source=source, slug=slug, title=title, date=date, tags=tags, summary=summary,
            lang=str(meta.get("lang", "en")).strip().lower() or "en",
            body_html=str(rendered["html"]), words=int(rendered["words"]),  # type: ignore[arg-type]
            asset_dir=path.parent if is_bundle else None,
        )
        if "updated" in meta:
            note.updated = parse_date(meta["updated"], source, "updated")
        elif source in changed and changed[source] > date:
            note.updated = changed[source]
        notes.append(note)

    # Oldest first, so a note keeps its number when newer notes are added.
    notes.sort(key=lambda n: (n.date, n.slug))
    for index, note in enumerate(notes):
        note.id = index
    return notes


def resolve_note_links(notes: list[Note], base: str) -> None:
    by_key = {note_key(note.source): note for note in notes}

    for note in notes:
        def replace(match: re.Match[str]) -> str:
            key, fragment = match.group(1), match.group(2) or ""
            target = by_key.get(key)
            if target is None:
                raise BuildError(f"{note.source}: links to '{key}', which is not a note")
            if target.slug != note.slug and target.slug not in note.links:
                note.links.append(target.slug)
            return f'href="{base}{target.slug}/{fragment}"'

        note.body_html = re.sub(r'href="note://([^"#]+)(#[^"]*)?"', replace, note.body_html)


# ------------------------------------------------------------------- palette

def hex_to_rgb(value: str) -> tuple[float, float, float]:
    value = value.strip().lstrip("#")
    if not re.fullmatch(r"[0-9a-fA-F]{6}", value):
        raise BuildError(f"palette.toml: '{value}' is not a colour like #56e8ff")
    return tuple(int(value[i:i + 2], 16) / 255 for i in (0, 2, 4))  # type: ignore[return-value]


def lift(colour: str, min_lightness: float) -> str:
    """Raises the lightness of a colour that is too dark for a black page."""
    import colorsys

    r, g, b = hex_to_rgb(colour)
    hue, lightness, saturation = colorsys.rgb_to_hls(r, g, b)
    if lightness < min_lightness:
        r, g, b = colorsys.hls_to_rgb(hue, min_lightness, saturation)
    return "#{:02x}{:02x}{:02x}".format(*(round(c * 255) for c in (r, g, b)))


def load_palette() -> tuple[dict[str, str], str]:
    raw = tomllib.loads((ROOT / "palette.toml").read_text(encoding="utf-8"))
    min_lightness = float(raw.get("min_lightness", 0.6))
    default = lift(str(raw.get("default", "#56e8ff")), min_lightness)
    colours = {normalise_tag(name): lift(str(value), min_lightness)
               for name, value in raw.get("tags", {}).items()}
    return colours, default


# --------------------------------------------------------------------- scene

class NeighbourFinder:
    """Finds the nearest notes of a group, using a grid so it stays fast."""

    def __init__(self, members: list[int], positions: list[tuple[float, float, float]]):
        self.positions = positions
        self.count = len(members)
        self.members = set(members)
        points = [positions[member] for member in members]
        spans = [max(p[axis] for p in points) - min(p[axis] for p in points) for axis in range(3)]
        # About two notes to a cell.
        volume = max(spans[0], 1.0) * max(spans[1], 1.0) * max(spans[2], 1.0)
        self.cell = max((volume / max(self.count / 2, 1)) ** (1 / 3), 0.5)
        self.cells: dict[tuple[int, int, int], list[int]] = {}
        for member in members:
            self.cells.setdefault(self.key(positions[member]), []).append(member)

    def key(self, point: tuple[float, float, float]) -> tuple[int, int, int]:
        return tuple(math.floor(c / self.cell) for c in point)  # type: ignore[return-value]

    def nearest(self, note: int, wanted: int) -> list[int]:
        origin = self.positions[note]
        others = self.count - (1 if note in self.members else 0)
        wanted = min(wanted, others)
        if wanted <= 0:
            return []
        home = self.key(origin)
        reach = 1
        while True:
            found: list[tuple[float, int]] = []
            for x in range(home[0] - reach, home[0] + reach + 1):
                for y in range(home[1] - reach, home[1] + reach + 1):
                    for z in range(home[2] - reach, home[2] + reach + 1):
                        for other in self.cells.get((x, y, z), ()):
                            if other != note:
                                found.append((math.dist(origin, self.positions[other]), other))
            found.sort()
            # Anything closer than the searched shell cannot be beaten by a
            # note outside it.
            sure = [other for distance, other in found if distance <= reach * self.cell]
            if len(sure) >= wanted or len(found) >= others:
                return [other for _, other in found[:wanted]]
            reach += 1


def layout_scene(notes: list[Note], tags: list[dict[str, object]]) -> dict[str, object]:
    """Places every note in space. Notes gather around their first tag."""
    count = len(notes)
    tag_index = {tag["name"]: i for i, tag in enumerate(tags)}
    ring = 10.0 * max(1.0, (count / 24) ** (1 / 3))
    golden = math.pi * (3 - math.sqrt(5))

    centres: list[tuple[float, float, float]] = []
    for i in range(len(tags)):
        if len(tags) == 1:
            centres.append((0.0, 0.0, 0.0))
            continue
        y = 1 - 2 * (i + 0.5) / len(tags)
        radius = math.sqrt(max(0.0, 1 - y * y))
        angle = i * golden
        centres.append((ring * radius * math.cos(angle), ring * 0.45 * y, ring * radius * math.sin(angle)))

    first_tag_count: dict[int, int] = {}
    for note in notes:
        if note.tags:
            index = tag_index[note.tags[0]]
            first_tag_count[index] = first_tag_count.get(index, 0) + 1

    positions: list[tuple[float, float, float]] = []
    for note in notes:
        rng = random.Random(int.from_bytes(hashlib.sha256(note.slug.encode()).digest()[:8], "big"))
        while True:
            jitter = (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))
            if sum(c * c for c in jitter) <= 1:
                break
        if note.tags:
            first = tag_index[note.tags[0]]
            centre = centres[first]
            if len(note.tags) > 1:
                other = centres[tag_index[note.tags[1]]]
                centre = tuple(a + (b - a) * 0.25 for a, b in zip(centre, other))  # type: ignore[assignment]
            spread = 4.0 * max(1.0, (first_tag_count[first] / 6) ** (1 / 3))
        else:
            centre, spread = (0.0, 0.0, 0.0), ring * 0.6
        positions.append((centre[0] + jitter[0] * spread,
                          centre[1] + jitter[1] * spread * 0.75,
                          centre[2] + jitter[2] * spread))

    # Links: each note joins its two nearest neighbours with the same first
    # tag. Joining neighbours keeps the links short. A note with a second tag
    # also reaches across to the nearest note of that tag; those links are long,
    # so only MAX_BRIDGES of them are kept, spread evenly over the notes. Notes
    # that link to each other in their text are always joined.
    links: dict[tuple[int, int], int] = {}
    groups: dict[str, list[int]] = {}
    for note in notes:
        if note.tags:
            groups.setdefault(note.tags[0], []).append(note.id)
    finders = {tag: NeighbourFinder(members, positions) for tag, members in groups.items()}
    two_tags = [note.id for note in notes if len(note.tags) > 1]
    step = max(1, math.ceil(len(two_tags) / MAX_BRIDGES))
    bridging = set(two_tags[::step])
    for note in notes:
        if not note.tags:
            continue
        for other in finders[note.tags[0]].nearest(note.id, 2):
            links.setdefault(tuple(sorted((note.id, other))), 0)  # type: ignore[arg-type]
        if note.id in bridging and note.tags[1] in finders:
            for other in finders[note.tags[1]].nearest(note.id, 1):
                links.setdefault(tuple(sorted((note.id, other))), 0)  # type: ignore[arg-type]
    by_slug = {note.slug: note.id for note in notes}
    for note in notes:
        for slug in note.links:
            pair = tuple(sorted((note.id, by_slug[slug])))
            links[pair] = 1  # type: ignore[index]

    if positions:
        centroid = tuple(sum(p[axis] for p in positions) / count for axis in range(3))
        radius = max(math.dist(p, centroid) for p in positions)
        floor = min(p[1] for p in positions) - 3.0
    else:
        centroid, radius, floor = (0.0, 0.0, 0.0), 0.0, -3.0
    return {"positions": positions, "links": links, "centroid": centroid, "radius": radius, "floor": floor}


def scene_bytes(notes: list[Note], tags: list[dict[str, object]], default_colour: str) -> bytes:
    scene = layout_scene(notes, tags)
    tag_index = {tag["name"]: i for i, tag in enumerate(tags)}
    links: dict[tuple[int, int], int] = scene["links"]  # type: ignore[assignment]

    out = bytearray()
    out += b"PNSC"
    out += struct.pack("<IIII", 1, len(notes), len(links), len(tags))
    out += struct.pack("<3f", *scene["centroid"])  # type: ignore[misc]
    out += struct.pack("<2f", scene["radius"], scene["floor"])
    out += struct.pack("<3B", *(round(c * 255) for c in hex_to_rgb(default_colour))) + b"\xff"
    for tag in tags:
        out += struct.pack("<3B", *(round(c * 255) for c in hex_to_rgb(str(tag["color"])))) + b"\xff"
    for position in scene["positions"]:  # type: ignore[union-attr]
        out += struct.pack("<3f", *position)
    for note in notes:
        out += struct.pack("<I", tag_index[note.tags[0]] if note.tags else NO_TAG)
    for (a, b), kind in links.items():
        out += struct.pack("<III", a, b, kind)
    return bytes(out)


def index_bytes(notes: list[Note], tags: list[dict[str, object]]) -> bytes:
    """Fourteen bytes per note: the date, then up to five tag numbers."""
    tag_index = {tag["name"]: i for i, tag in enumerate(tags)}
    out = bytearray(b"PNIX")
    out += struct.pack("<II", 1, len(notes))
    for note in notes:
        numbers = [tag_index[tag] for tag in note.tags] + [NO_TAG] * MAX_TAGS_PER_NOTE
        out += struct.pack(f"<I{MAX_TAGS_PER_NOTE}H", note.date.year * 10000 + note.date.month * 100 + note.date.day,
                           *numbers[:MAX_TAGS_PER_NOTE])
    return bytes(out)


# --------------------------------------------------------------------- pages

class Site:
    def __init__(self, config: dict[str, object], out: Path, build_id: str):
        self.config = config
        self.out = out
        self.build_id = build_id
        self.origin = str(config["origin"]).rstrip("/")
        self.base = "/" + str(config["base_path"]).strip("/") + "/"
        if self.base == "//":
            self.base = "/"
        self.colours, self.default_colour = load_palette()

    def url(self, path: str = "") -> str:
        return f"{self.origin}{self.base}{path}"

    def colour(self, tag: str) -> str:
        return self.colours.get(tag, self.default_colour)

    def tag_link(self, tag: str, extra: str = "") -> str:
        return (f'<a class="tag" href="{self.base}tags/{esc(tag)}/" style="--tag:{self.colour(tag)}"'
                f'{extra}>#{esc(tag)}</a>')

    def nav(self) -> str:
        items = []
        for entry in self.config.get("nav", []):  # type: ignore[union-attr]
            href = str(entry["href"])
            external = re.match(r"^https?://", href) is not None
            current = ' aria-current="page"' if href.rstrip("/") == self.base.rstrip("/") else ""
            target = ' target="_blank" rel="noopener noreferrer"' if external else ""
            items.append(f'      <a href="{esc(href)}"{current}{target}>{esc(entry["label"])}</a>')
        return "\n".join(items)

    def page(self, *, path: str, title: str, description: str, body: str, body_class: str,
             og_type: str, json_ld: object, head_extra: str = "", scripts: str = "",
             lang: str = "en", robots: str = "index, follow") -> None:
        canonical = self.url(path)
        context = {
            "lang": esc(lang),
            "title": esc(title),
            "description": esc(description),
            "author": esc(self.config["author"]),
            "robots": robots,
            "og_type": og_type,
            "site_title": esc(self.config["title"]),
            "canonical": esc(canonical),
            "social_image": esc(self.config["social_image"]),
            "base": self.base,
            "build": self.build_id,
            "json_ld": json_for_script(json_ld),
            "head_extra": head_extra,
            "body_class": body_class,
            "wordmark": esc(self.config["wordmark"]),
            "nav": self.nav(),
            "body": body,
            "year": dt.date.today().year,
            "origin": esc(self.origin),
            "repository": esc(self.config["repository"]),
            "branch": esc(self.config["branch"]),
            "scripts": scripts,
        }
        target = self.out / path / "index.html" if not path.endswith(".html") else self.out / path
        write(target, render_template("base.html", context))

    def person(self) -> dict[str, object]:
        return {"@type": "Person", "name": self.config["author"], "url": self.origin + "/",
                "sameAs": self.config.get("same_as", [])}

    def note_item(self, note: Note, heading: str = "h3") -> str:
        tags = "".join(f"<li>{self.tag_link(tag)}</li>" for tag in note.tags)
        tag_list = f'\n      <ul class="note-tags" aria-label="Tags">{tags}</ul>' if tags else ""
        return f"""  <article class="note-item" data-id="{note.id}">
    <time datetime="{note.date.isoformat()}">{short_date(note.date)}</time>
    <div>
      <{heading}><a href="{self.base}{note.slug}/">{esc(note.title)}</a></{heading}>
      <p>{esc(note.summary)}</p>{tag_list}
      <div class="note-actions">
        <a class="btn" href="{self.base}{note.slug}/" aria-label="Read more: {esc(note.title)}">Read more</a>
        <button class="btn" type="button" data-locate="{note.id}" hidden>Locate in 3D</button>
      </div>
    </div>
  </article>"""

    def note_list(self, notes: list[Note], heading: str = "h3") -> str:
        """Newest first, with a divider each time the year changes."""
        parts: list[str] = []
        year: int | None = None
        for note in notes:
            if note.date.year != year:
                year = note.date.year
                parts.append(f'  <div class="year-divider" role="heading" aria-level="2">{year}</div>')
            parts.append(self.note_item(note, heading))
        return "\n".join(parts)


def build_index(site: Site, notes: list[Note], tags: list[dict[str, object]]) -> None:
    config = site.config
    newest = list(reversed(notes))[: int(config["chunk_size"])]  # type: ignore[call-overload]
    bar = "\n".join(
        "      " + site.tag_link(str(tag["name"]), f' data-tag="{i}" role="button" aria-pressed="false"')
        for i, tag in enumerate(tags[: int(config["tag_bar_size"])])  # type: ignore[call-overload]
    )
    body = render_template("index.html", {
        "base": site.base,
        "title": esc(config["title"]),
        "count": len(notes),
        "count_label": f"{len(notes)} note" + ("" if len(notes) == 1 else "s") + ", newest first",
        "tag_bar": bar,
        "items": site.note_list(newest),
    })
    json_ld = {
        "@context": "https://schema.org",
        "@type": "Blog",
        "name": config["title"],
        "description": config["description"],
        "url": site.url(),
        "inLanguage": "en",
        "author": site.person(),
        "blogPost": [{"@type": "BlogPosting", "headline": n.title, "url": site.url(f"{n.slug}/"),
                      "datePublished": n.date.isoformat()} for n in newest[:20]],
    }
    scripts = f'<script type="module" src="{site.base}assets/js/app.js?v={site.build_id}"></script>'
    site.page(path="", title=str(config["title"]), description=str(config["description"]), body=body,
              body_class="page-index view-list", og_type="website", json_ld=json_ld, scripts=scripts,
              head_extra=WAIT_FOR_SCENE)


def build_notes(site: Site, notes: list[Note]) -> None:
    config = site.config
    for position, note in enumerate(notes):
        older = notes[position - 1] if position > 0 else None
        newer = notes[position + 1] if position + 1 < len(notes) else None
        neighbours = []
        if older:
            neighbours.append(f'<a href="{site.base}{older.slug}/" rel="prev">&larr; {esc(older.title)}</a>')
        if newer:
            neighbours.append(f'<a href="{site.base}{newer.slug}/" rel="next">{esc(newer.title)} &rarr;</a>')

        meta = []
        for tag in note.tags:
            search_filter = f' data-pagefind-filter="tag[data-tag]" data-tag="{esc(tag)}"'
            meta.append(f"<li>{site.tag_link(tag, search_filter)}</li>")
        minutes = max(1, round(note.words / 200))
        meta.append(f"<li>{minutes} min read</li>")
        if note.updated and note.updated != note.date:
            meta.append(f'<li>Revised <time datetime="{note.updated.isoformat()}">{long_date(note.updated)}</time></li>')

        history = f'{config["repository"]}/commits/{config["branch"]}/{note.source}'
        direction = ' dir="rtl"' if note.lang in RTL_LANGUAGES else ""
        body = render_template("note.html", {
            "base": site.base,
            "id": note.id,
            "lang": esc(note.lang),
            "direction": direction,
            "iso_date": note.date.isoformat(),
            "long_date": long_date(note.date),
            "title": esc(note.title),
            "meta": "\n        ".join(meta),
            "content": note.body_html,
            "history": esc(history),
            "footer_tags": "".join(site.tag_link(tag) for tag in note.tags),
            "author": esc(config["author"]),
            "origin": esc(site.origin),
            "canonical": esc(site.url(f"{note.slug}/")),
            "year": note.date.year,
            "neighbours": ('        <nav class="note-neighbours" aria-label="Older and newer notes">\n          '
                           + "\n          ".join(neighbours) + "\n        </nav>\n") if neighbours else "",
        })
        json_ld = {
            "@context": "https://schema.org",
            "@type": "BlogPosting",
            "headline": note.title,
            "description": note.summary,
            "url": site.url(f"{note.slug}/"),
            "mainEntityOfPage": site.url(f"{note.slug}/"),
            "datePublished": note.date.isoformat(),
            "dateModified": (note.updated or note.date).isoformat(),
            "inLanguage": note.lang,
            "keywords": note.tags,
            "wordCount": note.words,
            "image": config["social_image"],
            "copyrightYear": note.date.year,
            "copyrightHolder": site.person(),
            "author": site.person(),
            "publisher": site.person(),
            "isPartOf": {"@type": "Blog", "name": config["title"], "url": site.url()},
        }
        head = [f'<meta property="article:published_time" content="{note.date.isoformat()}">',
                f'<meta property="article:modified_time" content="{(note.updated or note.date).isoformat()}">',
                f'<meta property="article:author" content="{esc(config["author"])}">']
        head += [f'<meta property="article:tag" content="{esc(tag)}">' for tag in note.tags]
        site.page(path=f"{note.slug}/", title=f"{note.title} | {config['title']}", description=note.summary,
                  body=body, body_class="page-note", og_type="article", json_ld=json_ld,
                  head_extra="\n  ".join(head), lang=note.lang,
                  scripts=f'<script type="module" src="{site.base}assets/js/note.js?v={site.build_id}"></script>')

        if note.asset_dir is not None:
            for asset in note.asset_dir.rglob("*"):
                if asset.is_file() and asset.suffix.lower() != ".md":
                    target = site.out / note.slug / asset.relative_to(note.asset_dir)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(asset, target)


def build_tag_pages(site: Site, notes: list[Note], tags: list[dict[str, object]]) -> None:
    config = site.config
    limit = 200
    for tag in tags:
        name = str(tag["name"])
        tagged = [note for note in reversed(notes) if name in note.tags]
        more = ""
        if len(tagged) > limit:
            more = (f'<p class="list-more">Showing the newest {limit} of {len(tagged)} notes. '
                    f'<a href="{site.base}?tag={esc(name)}&amp;view=list">Browse all of them</a>.</p>')
        label = f"{len(tagged)} note" + ("" if len(tagged) == 1 else "s")
        body = render_template("tag.html", {
            "base": site.base,
            "tag": esc(name),
            "colour": site.colour(name),
            "count_label": label,
            "items": site.note_list(tagged[:limit], heading="h2"),
            "more": more,
        })
        description = f"{label} tagged #{name} in {config['title']}."
        json_ld = {
            "@context": "https://schema.org",
            "@type": "CollectionPage",
            "name": f"#{name}",
            "description": description,
            "url": site.url(f"tags/{name}/"),
            "isPartOf": {"@type": "Blog", "name": config["title"], "url": site.url()},
        }
        site.page(path=f"tags/{name}/", title=f"#{name} | {config['title']}", description=description,
                  body=body, body_class="page-tag", og_type="website", json_ld=json_ld)


def build_not_found(site: Site) -> None:
    body = render_template("404.html", {"base": site.base})
    site.page(path="404.html", title=f"Not found | {site.config['title']}",
              description="This page does not exist.", body=body, body_class="page-note",
              og_type="website", json_ld={"@context": "https://schema.org", "@type": "WebPage", "name": "Not found"},
              robots="noindex")


def build_data(site: Site, notes: list[Note], tags: list[dict[str, object]]) -> None:
    chunk = int(site.config["chunk_size"])  # type: ignore[call-overload]
    tag_index = {tag["name"]: i for i, tag in enumerate(tags)}
    data = site.out / "data"
    for start in range(0, len(notes), chunk):
        rows = [[n.slug, n.title, n.date.isoformat(), [tag_index[t] for t in n.tags], n.summary]
                for n in notes[start:start + chunk]]
        write(data / "notes" / f"{start // chunk}.json", json.dumps(rows, ensure_ascii=False, separators=(",", ":")))
    write(data / "index.bin", index_bytes(notes, tags))
    write(data / "scene.bin", scene_bytes(notes, tags, site.default_colour))
    write(data / "manifest.json", json.dumps({
        "build": site.build_id,
        "count": len(notes),
        "chunk": chunk,
        "defaultColor": site.default_colour,
        "tags": tags,
    }, ensure_ascii=False, separators=(",", ":")))


def build_feed(site: Site, notes: list[Note]) -> None:
    config = site.config
    newest = list(reversed(notes))[: int(config["feed_size"])]  # type: ignore[call-overload]
    updated = max((n.updated or n.date for n in notes), default=dt.date.today())
    entries = []
    for note in newest:
        categories = "".join(f'\n    <category term="{esc(tag)}"/>' for tag in note.tags)
        entries.append(f"""  <entry>
    <title>{esc(note.title)}</title>
    <link href="{esc(site.url(f'{note.slug}/'))}"/>
    <id>{esc(site.url(f'{note.slug}/'))}</id>
    <published>{note.date.isoformat()}T00:00:00Z</published>
    <updated>{(note.updated or note.date).isoformat()}T00:00:00Z</updated>
    <summary>{esc(note.summary)}</summary>{categories}
    <content type="html">{esc(note.body_html)}</content>
  </entry>""")
    feed = f"""<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>{esc(config['title'])}</title>
  <subtitle>{esc(config['description'])}</subtitle>
  <link href="{esc(site.url())}"/>
  <link rel="self" href="{esc(site.url('feed.xml'))}"/>
  <id>{esc(site.url())}</id>
  <updated>{updated.isoformat()}T00:00:00Z</updated>
  <rights>© {dt.date.today().year} {esc(config['author'])}. All rights reserved.</rights>
  <author>
    <name>{esc(config['author'])}</name>
    <uri>{esc(site.origin)}/</uri>
  </author>
{chr(10).join(entries)}
</feed>
"""
    write(site.out / "feed.xml", feed)


def build_sitemap(site: Site, notes: list[Note], tags: list[dict[str, object]]) -> None:
    newest = max((n.updated or n.date for n in notes), default=dt.date.today())
    rows = [(site.url(), newest)]
    rows += [(site.url(f"{n.slug}/"), n.updated or n.date) for n in reversed(notes)]
    for tag in tags:
        dates = [n.updated or n.date for n in notes if tag["name"] in n.tags]
        rows.append((site.url(f"tags/{tag['name']}/"), max(dates)))
    body = "\n".join(f"  <url>\n    <loc>{esc(loc)}</loc>\n    <lastmod>{date.isoformat()}</lastmod>\n  </url>"
                     for loc, date in rows)
    write(site.out / "sitemap.xml",
          f'<?xml version="1.0" encoding="UTF-8"?>\n'
          f'<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n{body}\n</urlset>\n')


# ------------------------------------------------------------ external tools

def run(command: list[str], cwd: Path = ROOT) -> None:
    log("  $ " + " ".join(command))
    result = subprocess.run(command, cwd=cwd)
    if result.returncode != 0:
        raise BuildError(f"'{command[0]}' failed with exit code {result.returncode}")


def build_scripts(site: Site) -> None:
    if not (ROOT / "node_modules" / ".bin" / "tsc").exists():
        run(["npm", "ci" if (ROOT / "package-lock.json").exists() else "install"])
    run([str(ROOT / "node_modules" / ".bin" / "tsc"), "--project", "web/tsconfig.json",
         "--outDir", str(site.out / "assets" / "js")])


def build_wasm(site: Site, release: bool) -> None:
    if shutil.which("wasm-bindgen") is None:
        raise BuildError("wasm-bindgen is not installed. Install the version recorded in Cargo.lock with "
                         "'cargo install wasm-bindgen-cli --version <version> --locked'.")
    profile = "release" if release else "debug"
    command = ["cargo", "build", "--locked", "--target", "wasm32-unknown-unknown", "--lib"]
    if release:
        command.append("--release")
    run(command)
    package = site.out / "pkg"
    run(["wasm-bindgen", "--target", "web", "--out-dir", str(package), "--out-name", WASM_NAME,
         "--no-typescript", str(ROOT / "target" / "wasm32-unknown-unknown" / profile / f"{WASM_NAME}.wasm")])
    binary = package / f"{WASM_NAME}_bg.wasm"
    if release and shutil.which("wasm-opt") is not None:
        run(["wasm-opt", "-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int",
             "-o", str(binary), str(binary)])
    size = binary.stat().st_size
    compressed = len(gzip.compress(binary.read_bytes(), compresslevel=6))
    log(f"  wasm: {size / 1024:.0f} KiB, {compressed / 1024:.0f} KiB gzipped")
    if release and size > WASM_BUDGET_BYTES:
        log(f"  warning: the wasm is over the {WASM_BUDGET_BYTES / 1024 / 1024:.0f} MiB budget")


def build_search(site: Site, notes: list[Note]) -> None:
    run([sys.executable, "-m", "pagefind", "--site", str(site.out), "--output-subdir", "pagefind"])
    # Pagefind names every page by a hash. This table turns those names into
    # note numbers, so a search can light up notes in the scene without
    # downloading each result.
    for unused in (site.out / "pagefind").glob("pagefind-*ui*"):
        unused.unlink()
    by_url = {f"/{note.slug}/": note.id for note in notes}
    table: dict[str, int] = {}
    for fragment in (site.out / "pagefind" / "fragment").glob("*.pf_fragment"):
        raw = gzip.decompress(fragment.read_bytes())
        start = raw.find(b"{")
        if start < 0:
            raise BuildError(f"{fragment.name}: Pagefind's fragment format has changed")
        url = json.loads(raw[start:])["url"]
        if url not in by_url:
            raise BuildError(f"{fragment.name}: Pagefind indexed {url}, which is not a note")
        table[fragment.name.split(".")[0]] = by_url[url]
    if len(table) != len(notes):
        raise BuildError(f"Pagefind indexed {len(table)} pages, but there are {len(notes)} notes")
    write(site.out / "data" / "search-map.json", json.dumps(table, separators=(",", ":")))


# ------------------------------------------------------------------ commands

def command_build(args: argparse.Namespace) -> None:
    started = time.monotonic()
    config = tomllib.loads((ROOT / "site.toml").read_text(encoding="utf-8"))
    out = Path(args.out).resolve()
    content = Path(args.content).resolve()
    build_id = hashlib.sha256(str(time.time_ns()).encode()).hexdigest()[:10]

    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    site = Site(config, out, build_id)

    log(f"Reading notes from {content}")
    notes = load_notes(content, ROOT / ".cache" / "markdown", args.drafts)
    if not notes:
        raise BuildError("there are no notes to publish")
    resolve_note_links(notes, site.base)

    counts: dict[str, int] = {}
    for note in notes:
        for tag in note.tags:
            counts[tag] = counts.get(tag, 0) + 1
    tags = [{"name": name, "color": site.colour(name), "count": number}
            for name, number in sorted(counts.items(), key=lambda item: (-item[1], item[0]))]
    if len(tags) >= NO_TAG:
        raise BuildError("there are too many different tags")

    log(f"Writing {len(notes)} notes and {len(tags)} tags")
    build_index(site, notes, tags)
    build_notes(site, notes)
    build_tag_pages(site, notes, tags)
    build_not_found(site)
    build_data(site, notes, tags)
    build_feed(site, notes)
    build_sitemap(site, notes, tags)
    shutil.copytree(ROOT / "assets", out / "assets", dirs_exist_ok=True)
    write(out / ".nojekyll", "")

    log("Compiling the TypeScript")
    build_scripts(site)
    if args.skip_wasm:
        log("Skipping the WebGPU scene")
    else:
        log("Building the WebGPU scene")
        build_wasm(site, not args.debug)
    if args.skip_search:
        log("Skipping the search index")
    else:
        log("Indexing for search")
        build_search(site, notes)

    log(f"Built {out} in {time.monotonic() - started:.1f}s")


def command_serve(args: argparse.Namespace) -> None:
    import http.server

    config = tomllib.loads((ROOT / "site.toml").read_text(encoding="utf-8"))
    base = "/" + str(config["base_path"]).strip("/") + "/"
    out = Path(args.out).resolve()

    class Handler(http.server.SimpleHTTPRequestHandler):
        extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map,
                          ".wasm": "application/wasm", ".js": "text/javascript"}

        def __init__(self, *handler_args, **handler_kwargs):
            super().__init__(*handler_args, directory=str(out), **handler_kwargs)

        def do_GET(self) -> None:
            if not self.path.startswith(base):
                self.send_response(302)
                self.send_header("Location", base)
                self.end_headers()
                return
            self.path = "/" + self.path[len(base):]
            super().do_GET()

        def end_headers(self) -> None:
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

        def log_message(self, *_args) -> None:
            pass

    with http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler) as server:
        log(f"Serving {out} at http://localhost:{args.port}{base}")
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


def command_new(args: argparse.Namespace) -> None:
    today = dt.date.today()
    slug = slugify(args.title)
    if not slug:
        raise BuildError("the title needs at least one letter or digit")
    path = ROOT / "content" / str(today.year) / f"{slug}.md"
    if path.exists():
        raise BuildError(f"{path.relative_to(ROOT)} already exists")
    write(path, f"---\ntitle: {args.title}\ndate: {today.isoformat()}\ntags: []\nsummary:\n---\n\n")
    log(f"Created {path.relative_to(ROOT)}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Builds the notes site.")
    parser.add_argument("--out", default=str(ROOT / "_site"), help="output directory")
    sub = parser.add_subparsers(dest="command")

    def add_build_options(target: argparse.ArgumentParser) -> None:
        target.add_argument("--content", default=str(ROOT / "content"), help="directory of Markdown notes")
        target.add_argument("--debug", action="store_true", help="build the wasm without optimisation")
        target.add_argument("--skip-wasm", action="store_true", help="do not build the WebGPU scene")
        target.add_argument("--skip-search", action="store_true", help="do not build the search index")
        target.add_argument("--drafts", action="store_true", help="include notes marked 'draft: true'")

    add_build_options(parser)
    add_build_options(sub.add_parser("build", help="build the site (the default)"))
    serve = sub.add_parser("serve", help="serve the built site locally")
    serve.add_argument("--port", type=int, default=8000)
    new = sub.add_parser("new", help="create a new note")
    new.add_argument("title")

    args = parser.parse_args()
    try:
        {"serve": command_serve, "new": command_new}.get(args.command, command_build)(args)
    except BuildError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
