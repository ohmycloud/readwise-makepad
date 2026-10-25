"""Create an owned, deterministic EPUB for Web smoke tests (no user books)."""
import base64
import sys
import zipfile
from pathlib import Path


def create(destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as book:
        book.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        book.writestr("META-INF/container.xml", '''<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>''')
        book.writestr("OEBPS/book.opf", '''<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">readwise-web-test</dc:identifier><dc:title>林间低语 · Web 回归</dc:title><dc:creator>ReadWise QA</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="c1" href="one.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="two.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="image" href="pixel.png" media-type="image/png"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>''')
        book.writestr("OEBPS/nav.xhtml", '''<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="one.xhtml">The Forest</a></li><li><a href="two.xhtml#end">The Return</a></li></ol></nav></body></html>''')
        code = '<pre class="language-rust"><code>fn main() {\n    // 森林 🌿\n    let forest = &quot;&lt;script&gt;&amp;&quot;;\n    println!(&quot;{}&quot;, forest);\n}\n</code></pre>'
        for index, name in enumerate(["one", "two"]):
            paragraphs = "".join(f"<p>In the solitude of the forest, every murmur tells a story. A gentle breeze moved through the ancient trees. Paragraph {i}: 中文与 emoji 🌿 remain stable.</p>" for i in range(80))
            book.writestr(f"OEBPS/{name}.xhtml", f'''<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="end">{'The Forest' if index == 0 else 'The Return'}</h1>{paragraphs}{code}<p><img src="pixel.png" alt="Test pixel"/></p><p><a href="{'two' if index == 0 else 'one'}.xhtml">Other chapter</a></p></body></html>''')
        book.writestr("OEBPS/pixel.png", base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="))


if __name__ == "__main__":
    create(Path(sys.argv[1]))
