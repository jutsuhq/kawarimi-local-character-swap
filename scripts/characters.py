"""Character test images for the cutout, from Wikimedia Commons, only public-domain or CC0 files, into
.cache/characters/ (1024 px wide thumbnails), with sources.json naming each file's page and license.

    python -I scripts/characters.py
"""
import json
import os
import urllib.parse
import urllib.request

API = "https://commons.wikimedia.org/w/api.php"
UA = {"User-Agent": "kawarimi test images (https://github.com/jutsuhq/kawarimi-local-character-swap)"}
OUT = os.path.join(os.path.dirname(__file__), "..", ".cache", "characters")
FREE = ("public domain", "pd", "cc0", "cc-zero")

TITLES = [
    "File:Mona Lisa, by Leonardo da Vinci, from C2RMF retouched.jpg",
    "File:1665 Girl with a Pearl Earring.jpg",
    "File:John Singer Sargent - Madame X (Madame Pierre Gautreau) - Google Art Project.jpg",
    "File:Vincent van Gogh - Self-Portrait - Google Art Project (454045).jpg",
]
SEARCHES = ["character sheet drawing", "anime girl illustration", "cartoon character full body", "knight armour full body"]


def get(params):
    url = API + "?" + urllib.parse.urlencode({**params, "format": "json"})
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA)))


def info(titles):
    pages = get({"action": "query", "titles": "|".join(titles), "prop": "imageinfo",
                 "iiprop": "url|extmetadata|mime", "iiurlwidth": 1024})["query"]["pages"]
    for p in pages.values():
        if "imageinfo" not in p:
            continue
        ii = p["imageinfo"][0]
        lic = ii["extmetadata"].get("LicenseShortName", {}).get("value", "")
        if ii.get("mime") in ("image/jpeg", "image/png") and lic.lower().startswith(FREE):
            yield p["title"], ii["thumburl"], ii["descriptionurl"], lic


def main():
    os.makedirs(OUT, exist_ok=True)
    chosen = list(info(TITLES))
    for q in SEARCHES:
        hits = get({"action": "query", "list": "search", "srsearch": f"{q} filetype:bitmap", "srnamespace": 6, "srlimit": 20})
        titles = [h["title"] for h in hits["query"]["search"]]
        found = [c for c in info(titles) if c[0] not in [x[0] for x in chosen]]
        chosen.extend(found[:2])
    sources = []
    for i, (title, url, page, lic) in enumerate(chosen[:12]):
        name = f"{i:02d}" + os.path.splitext(urllib.parse.urlparse(url).path)[1].lower()
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(urllib.request.urlopen(urllib.request.Request(url, headers=UA)).read())
        sources.append({"file": name, "title": title, "page": page, "license": lic})
        print(name, lic, title[:70])
    json.dump(sources, open(os.path.join(OUT, "sources.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
