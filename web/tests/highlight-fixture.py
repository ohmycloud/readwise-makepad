"""Owned EPUB: real wrapping, inline code and paragraph/column boundaries."""
import sys
import zipfile
from pathlib import Path

p = Path(sys.argv[1])
text = ('The updated <code>Tools</code> class now works with surrounding words. '
        'Extra sentences wrap across several lines without changing the source. ')
listing = 'sample code line\n' * 30
edges = ('<p id="code-leading"><code>Benchmark</code> dataclass. The <code>Benchmark</code> contains information.</p>'
         '<p id="code-alone"><code>Benchmark</code></p>'
         '<p id="code-break">perform automated evaluation on your <code>TinyAgent</code>.<br/>'
         '<code>Benchmark</code> dataclass. The <code>Benchmark</code> contains information.</p>'
         '<p id="code-wrap"><code>' + 'Benchmark evaluation with multiple words ' * 4 + '</code> tail.</p>'
         '<p id="code-nested"><code><em>Bench</em>mark</code> dataclass. The <code>Benchmark</code> contains information.</p>')
seams = ('<p id="seams-final">The agent saw that it calculated all the necessary steps and returned the final with the <code>final_answer</code> tool.</p>'
         '<p id="seams-steps">The <code>max_steps</code> parameter is used to decide the maximum number of steps your '
         '<code>TinyAgent</code> will get to perform its autonomous behavior. The agent can still decide to stop early, '
         'but this prevents it from being stuck in a loop.</p>'
         '<p id="seams-plain">Least-to-most prompting and plan-and-solve prompting decompose a given problem into '
         'subtasks that are solved sequentially. These work great for LLMs, but not yet for agents, who sequence '
         'actions autonomously one after the other. Specifically, after the agent decides the (sub)goals it needs '
         'to achieve, it then determines a sequence of actions to continuously transition it from its current state '
         'to the desired goal state. This process is called <em>action sequencing</em> and requires not only a careful '
         'plan but also an efficient set of steps for the agent to follow in solitude.</p>')
with zipfile.ZipFile(p, 'w', zipfile.ZIP_DEFLATED) as book:
    book.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
    book.writestr('META-INF/container.xml', '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
    book.writestr('book.opf', '<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">highlight-qa</dc:identifier><dc:title>Highlight QA</dc:title><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>')
    book.writestr('chapter.xhtml', f'<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Highlight geometry</h1><p id="flow">{text * 14}</p><p id="following">A separate paragraph keeps its own margin. 中文与 emoji 🌿 remain intact.</p><pre><code>{listing}</code></pre>{edges}{seams}</body></html>')
