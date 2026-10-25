"""Owned PDF exercising scripts displaced in the legacy span stream and CMEX P.
P is the TeX CMEX summation slot, deliberately without a Unicode cmap. Native
source must remain P while the visual glyph is a summation, never a Roman P.
"""
import pathlib
import sys
content = b"""BT /F1 12 Tf 40 230 Td (A forest and solitude in the example.) Tj ET
BT /F1 12 Tf 40 200 Td (Keys have dimension ) Tj ET
BT /F2 12 Tf 145 200 Td (d) Tj ET
BT /F4 8 Tf 151 198.5 Td (k) Tj ET
BT /F1 12 Tf 159 200 Td (, values have dimension ) Tj ET
BT /F2 12 Tf 280 200 Td (d) Tj ET
BT /F4 8 Tf 286 198.5 Td (v) Tj ET
BT /F1 12 Tf 295 200 Td (. They are projected.) Tj ET
BT /F1 12 Tf 40 155 Td (Their product is ) Tj ET
BT /F2 12 Tf 130 155 Td (q) Tj ET
BT /F1 12 Tf 142 155 Td (=) Tj ET
BT /F3 12 Tf 158 163 Td (P) Tj ET
BT /F4 8 Tf 172 163 Td (n) Tj ET
BT /F4 8 Tf 172 149 Td (i) Tj ET
BT /F1 8 Tf 176 149 Td (=1) Tj ET
BT /F2 12 Tf 192 155 Td (z) Tj ET
BT /F4 8 Tf 198 153.5 Td (i) Tj ET
BT /F1 12 Tf 215 155 Td (. End of example.) Tj ET
BT /F1 12 Tf 40 110 Td (A forest has steady line spacing in this paragraph.) Tj ET
BT /F1 12 Tf 40 90 Td (The scale is ) Tj ET
BT /F1 8 Tf 120 93 Td (1) Tj ET
BT /F2 8 Tf 120 86 Td (n) Tj ET
BT /F4 6 Tf 125 84.5 Td (i) Tj ET
0.4 w 118 91 m 132 91 l S
BT /F1 12 Tf 139 90 Td ( in this sentence without changing the leading.) Tj ET
BT /F1 12 Tf 40 70 Td (The following line keeps the same steady spacing.) Tj ET
"""
widths = b" ".join(b"250" if i == 32 else b"1000" if i == 80 else b"500" for i in range(32,127))
objects = [
 b"<< /Type /Catalog /Pages 2 0 R >>",
 b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
 b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 280] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R /F4 7 0 R >> >> /Contents 8 0 R >>",
 b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
 b"<< /Type /Font /Subtype /Type1 /BaseFont /CMMI10 /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths ["+widths+b"] >>",
 b"<< /Type /Font /Subtype /Type1 /BaseFont /CMEX10 /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths ["+widths+b"] >>",
 b"<< /Type /Font /Subtype /Type1 /BaseFont /CMMI7 /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths ["+widths+b"] >>",
 b"<< /Length "+str(len(content)).encode()+b" >>\nstream\n"+content+b"endstream",
]
out = bytearray(b"%PDF-1.4\n")
offsets = [0]
for i,obj in enumerate(objects,1):
 offsets.append(len(out));out.extend(f"{i} 0 obj\n".encode()+obj+b"\nendobj\n")
xref=len(out)
out.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
for offset in offsets[1:]:out.extend(f"{offset:010d} 00000 n \n".encode())
out.extend(f"trailer\n<< /Root 1 0 R /Size {len(offsets)} >>\nstartxref\n{xref}\n%%EOF\n".encode())
pathlib.Path(sys.argv[1]).write_bytes(out)
