# Notes

[Pooya Eimandar's notes](https://pooya.ai/notes/).

## Writing a note

```bash
python3 build.py new "The title of the note"
```

This creates `content/<year>/<title>.md`:

```markdown
---
title: The title of the note
date: 2026-09-29
tags: [rust, graphics]
summary: One or two sentences shown in the list, in search results, and to search engines.
---

The note, in Markdown.
```

| Field     | Required | Meaning                                                              |
| --------- | -------- | -------------------------------------------------------------------- |
| `title`   | yes      | The heading of the note.                                             |
| `date`    | yes      | The day it was published, as `YYYY-MM-DD`.                           |
| `tags`    | no       | Up to five. The first one decides the colour of the note's orb.      |
| `summary` | no       | Defaults to the first paragraph.                                     |
| `updated` | no       | The day it was revised. Defaults to the date of its last git commit. |
| `slug`    | no       | The address of the note. Defaults to the file name.                  |
| `lang`    | no       | The language, such as `fa`. Worked out from the tags and the text. Defaults to `en`   |
| `draft`   | no       | `true` keeps the note out of the site.                               |

A note that needs images can be a folder: `content/2026/my-note/index.md`, with
the images next to it. To link to another note, link to its Markdown file, for
example `[hello](../2026/hello.md)`. The two notes are then joined in the scene.

## Colours

`palette.toml` holds one colour per tag. It starts from GitHub's language
colours. A tag that is not listed uses `default`.

## Building

Once:

```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
npm ci
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
```

Then:

```bash
.venv/bin/python build.py
.venv/bin/python build.py serve
```

The site is written to `_site/` and served at <http://localhost:8000/notes/>.

| Option          | Effect                                         |
| --------------- | ---------------------------------------------- |
| `--debug`       | Builds the scene faster, without optimisation. |
| `--skip-wasm`   | Leaves the scene out. The list still works.    |
| `--skip-search` | Leaves the search index out.                   |
| `--drafts`      | Includes notes marked `draft: true`.           |

If `wasm-opt` is installed, the build uses it to make the scene smaller.

## Publishing

Pushing to `main` builds and deploys the site with GitHub Actions. In the
repository settings, Pages has to use "GitHub Actions" as its source.

## License

| Material                                    | Terms                 |
| ------------------------------------------- | --------------------- |
| Source code, samples, and shaders           | [MIT](LICENSE-MIT)    |
| Articles and notes                          | [All rights reserved](content/LICENSE) |

