#!/usr/bin/env python3
"""把静态网站与仓库 pics 素材组装到 target/website，供预览和 GitHub Pages 使用。"""

from html.parser import HTMLParser
from pathlib import Path
import shutil
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target" / "website"


class PageAssets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.paths = set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        for key in ("src", "href", "data-image"):
            value = attrs.get(key, "")
            url = urlsplit(value)
            if value and not url.scheme and not url.netloc and url.path:
                self.paths.add(unquote(url.path))


def main():
    if OUTPUT.exists():
        shutil.rmtree(OUTPUT)
    OUTPUT.mkdir(parents=True)
    for name in ("index.html", "styles.css", "main.js"):
        shutil.copy2(ROOT / "website" / name, OUTPUT / name)
    shutil.copytree(ROOT / "website" / "assets", OUTPUT / "assets", ignore=shutil.ignore_patterns(".DS_Store"))
    shutil.copytree(ROOT / "pics", OUTPUT / "pics", ignore=shutil.ignore_patterns(".DS_Store"))
    (OUTPUT / ".nojekyll").touch()

    page = PageAssets()
    page.feed((OUTPUT / "index.html").read_text())
    missing = sorted(path for path in page.paths if not (OUTPUT / path).is_file())
    if missing:
        raise SystemExit("网站素材不存在：\n" + "\n".join(missing))
    print(f"网站已构建到 {OUTPUT}（已检查 {len(page.paths)} 个本地资源路径）")


if __name__ == "__main__":
    main()
