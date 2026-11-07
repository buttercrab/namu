"""The links on a page, to pages of the same site, as paths."""

import re
from dataclasses import dataclass
from urllib.parse import urljoin, urlparse

import namu

HREF = re.compile(r'href="([^"#]+)"')


@dataclass
class Page:
    url: str
    title: str
    html: str


@namu.task
def links(page: Page) -> list[str]:
    found = []
    for href in HREF.findall(page.html):
        url = urlparse(urljoin(page.url, href))
        if not url.netloc and url.path not in found:
            found.append(url.path)
    return found
