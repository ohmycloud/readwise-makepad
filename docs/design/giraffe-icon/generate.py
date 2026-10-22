#!/usr/bin/env python3
"""Generate ReadWise's giraffe icon colour studies (not production app assets).

Requires rsvg-convert on PATH. Run: python3 docs/design/giraffe-icon/generate.py
All paths are relative to this script; no application resources are changed.
"""
from html import escape
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
PALETTES = [
    dict(id="a-honey", name="A · 蜂蜜奶油", en="Honey & cream", bg="#FFF3DC", face="#F3B44C", muzzle="#FFF3DC", ink="#443125", note="最接近参考的温暖长颈鹿，亲切、轻松。"),
    dict(id="b-forest", name="B · 森林米白", en="Forest & ivory", bg="#1F6F63", face="#F5F1E6", muzzle="#F5F1E6", ink="#143F36", note="延续 ReadWise 现有墨绿，安静、耐看。推荐。"),
    dict(id="c-cobalt", name="C · 钴蓝冰川", en="Cobalt & ice", bg="#294DB5", face="#D5E8FF", muzzle="#D5E8FF", ink="#182A56", note="冷静清晰，偏现代工具型应用。"),
    dict(id="d-terracotta", name="D · 陶土蜜桃", en="Terracotta & peach", bg="#F8DDD2", face="#DF8267", muzzle="#F8DDD2", ink="#57332D", note="柔和、有纸张感，适合人文阅读气质。"),
    dict(id="e-lavender", name="E · 薰衣草雾", en="Lavender & mist", bg="#EEE7FA", face="#B4A0DD", muzzle="#EEE7FA", ink="#46365C", note="轻盈、温柔，增加个人化与陪伴感。"),
    dict(id="f-midnight", name="F · 午夜麦金", en="Midnight & gold", bg="#172C3C", face="#F3C969", muzzle="#F3C969", ink="#172C3C", note="深底浅形，轮廓醒目、桌面辨识度高。"),
]


def symbol(p):
    """256-unit grid; 7-unit round outline, mirrored ears, long tapering face.

    Reading is a quiet association in the two page-like ears, not an extra book.
    Separate muzzle stays legible when the face and muzzle share one colour.
    """
    ink, face, muzzle = p["ink"], p["face"], p["muzzle"]
    return f'''<g id="giraffe" stroke="{ink}" stroke-width="7" stroke-linecap="round" stroke-linejoin="round">
  <path fill="{face}" d="M43 78 C64 73 83 79 94 98 C70 107 51 100 43 78 Z"/>
  <path fill="{face}" d="M213 78 C192 73 173 79 162 98 C186 107 205 100 213 78 Z"/>
  <path fill="{face}" d="M100 52 H114 V80 H100 Z M142 52 H156 V80 H142 Z"/>
  <circle cx="107" cy="49" r="10" fill="{face}"/>
  <circle cx="149" cy="49" r="10" fill="{face}"/>
  <path fill="{face}" d="M102 76 H154 C168 76 175 85 173 99 L161 211 Q160 221 150 221 H106 Q96 221 95 211 L83 99 C81 85 88 76 102 76 Z"/>
  <path d="M107 49 V81 M149 49 V81" fill="none" stroke="{face}" stroke-width="7"/>
  <circle cx="108" cy="122" r="6" fill="{ink}" stroke="none"/>
  <circle cx="148" cy="122" r="6" fill="{ink}" stroke="none"/>
  <rect x="75" y="158" width="106" height="49" rx="24.5" fill="{muzzle}"/>
  <ellipse cx="96" cy="182.5" rx="4.5" ry="6.5" fill="{ink}" stroke="none"/>
  <ellipse cx="160" cy="182.5" rx="4.5" ry="6.5" fill="{ink}" stroke="none"/>
</g>'''


def icon(p):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256" role="img" aria-labelledby="title">
<title id="title">ReadWise giraffe icon study — {escape(p['en'])}</title>
<rect width="256" height="256" rx="56" fill="{p['bg']}"/>
{symbol(p)}
</svg>
'''


def text(x, y, content, size=16, fill="#303B35", weight=400):
    return f'<text x="{x}" y="{y}" fill="{fill}" font-size="{size}" font-weight="{weight}">{escape(content)}</text>'


def board():
    # Type is presentation-only. Icon source SVGs contain no live text or filters.
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" width="1440" height="1300" viewBox="0 0 1440 1300">',
             '<rect width="1440" height="1300" fill="#F5F3EE"/>',
             '<g font-family="PingFang SC, Noto Sans CJK SC, sans-serif">',
             text(64, 65, "READWISE / ICON COLOUR STUDIES", 14, "#607066", 600),
             text(64, 118, "一位安静的阅读伙伴。", 38, weight=600),
             text(64, 156, "正面长颈鹿 · 圆润线条 · 统一造型，六种性格", 17, "#6A746C"),
             text(1376, 70, "01 — 06", 14, "#607066") .replace('y="70"', 'y="70" text-anchor="end"')]
    for index, p in enumerate(PALETTES):
        x, y = 64 + (index % 3) * 448, 194 + (index // 3) * 498
        stroke = "#1F6F63" if index == 1 else "#E4E5DF"
        parts.append(f'<rect x="{x}" y="{y}" width="416" height="474" rx="24" fill="#FFFFFF" stroke="{stroke}" stroke-width="{2 if index == 1 else 1}"/>')
        parts.append(text(x + 26, y + 40, p['name'], 20, weight=600))
        if index == 1:
            parts.append(text(x + 390, y + 39, "推荐", 12, "#1F6F63", 600).replace(f'x="{x+390}"', f'x="{x+390}" text-anchor="end"'))
        parts.append(f'<svg x="{x+92}" y="{y+65}" width="232" height="232" viewBox="0 0 256 256"><rect width="256" height="256" rx="56" fill="{p["bg"]}"/>{symbol(p)}</svg>')
        parts.append(text(x + 26, y + 331, p['en'], 15, "#69756D"))
        colours = list(dict.fromkeys([p['bg'], p['face'], p['muzzle'], p['ink']]))
        for i, color in enumerate(colours):
            cx = x + 26 + i * 126
            parts.append(f'<circle cx="{cx+6}" cy="{y+358}" r="6" fill="{color}" stroke="#D6DAD3" stroke-width="0.6"/>')
            parts.append(text(cx + 19, y + 363, color, 12, "#69756D"))
        for i, size in enumerate([48, 32, 16]):
            sx = x + [26, 113, 185][i]
            parts.append(f'<svg x="{sx}" y="{y+386+48-size}" width="{size}" height="{size}" viewBox="0 0 256 256"><rect width="256" height="256" rx="56" fill="{p["bg"]}"/>{symbol(p)}</svg>')
            parts.append(text(sx, y + 455, f"{size} px", 10, "#788279"))
    parts += [text(64, 1244, "方向建议  /  B 森林米白：延续现有品牌；A 蜂蜜奶油：最贴近参考。", 17),
              text(64, 1277, "配色概念稿 · 未替换应用资源 · 图形已重绘；参考素材的授权与商标近似性仍需正式核查。", 13, "#788279"), '</g></svg>']
    return '\n'.join(parts)


def html():
    cards = []
    for p in PALETTES:
        colours = list(dict.fromkeys([p['bg'], p['face'], p['muzzle'], p['ink']]))
        swatches = ''.join(f'<span><i style="background:{c}"></i><code>{c}</code></span>' for c in colours)
        sizes = ''.join(f'<span><img src="{p["id"]}.svg" width="{s}" height="{s}" alt="{s}px 图标"><small>{s}px</small></span>' for s in [64, 32, 24, 16])
        cards.append(f'''<article class="{'recommended' if p['id']=='b-forest' else ''}"><h2>{p['name']}</h2>
<img class="hero" src="{p['id']}.svg" alt="{p['name']}长颈鹿图标">
<p>{p['note']}</p><div class="swatches">{swatches}</div><div class="sizes">{sizes}</div>
<a href="{p['id']}.svg" download>下载 SVG 概念稿 ↗</a></article>''')
    return '''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>ReadWise · 长颈鹿图标配色方案</title><style>
*{box-sizing:border-box}body{margin:0;background:#f5f3ee;color:#303b35;font:16px/1.7 -apple-system,BlinkMacSystemFont,"PingFang SC",sans-serif}
main{max-width:1320px;margin:auto;padding:48px 28px}header{margin-bottom:32px}header small{letter-spacing:.18em;color:#607066}h1{font-size:clamp(26px,4vw,44px);line-height:1.3}header p{color:#69756d}.grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:24px}article{padding:24px;background:white;border:1px solid #e1e4dd;border-radius:24px}article.recommended{border:2px solid #1f6f63;padding:23px}h2{font-size:20px;margin:0 0 24px}.hero{display:block;width:70%;height:auto;margin:0 auto 24px}article p{min-height:3.4em;color:#69756d}.swatches{display:flex;flex-wrap:wrap;gap:12px;font-size:12px}.swatches span{display:flex;align-items:center;gap:6px}.swatches i{width:14px;height:14px;border-radius:50%;border:1px solid #ddd}.sizes{display:flex;gap:28px;align-items:flex-end;margin:28px 0}.sizes span{display:flex;flex-direction:column;gap:8px}.sizes small{font-size:11px;color:#69756d}a{color:#1f6f63;text-underline-offset:4px}footer{margin:32px 0;color:#69756d;font-size:14px}footer strong{color:#303b35}@media(max-width:960px){.grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:560px){.grid{grid-template-columns:1fr}main{padding:28px 18px}.hero{max-width:256px}}
</style><main><header><small>READWISE / ICON COLOUR STUDIES</small><h1>一位安静的阅读伙伴。</h1><p>同一款正面长颈鹿，六组配色。保留长脸、大耳朵与呆萌表情，不额外堆叠书本、字母或 AI 符号。</p></header><section class="grid">''' + '\n'.join(cards) + '''</section><footer><p><strong>建议：</strong>B 延续现有 ReadWise 的墨绿米白；A 最接近参考，F 适合偏爱深色桌面的人。</p><p>这里是供选择的配色概念稿，尚未替换应用资源。16px 主要判断轮廓，五官细节仍需在选定后制作微型专用版。圆角仅用于方案预览；正式 iOS / Android 资源应按平台遮罩规范另行导出。</p><p>参考：用户提供的 cop.svg（含 SVG Repo 注释，未载明具体许可证）。本稿重新绘制路径，不是素材授权或商标无冲突保证，发布前仍需核查。</p><p><a href="overview.png">查看高清总览</a> · <a href="tests.html">查看尺寸 / 灰度 / 翻转测试页</a></p></footer></main></html>'''


def main():
    renderer = shutil.which('rsvg-convert')
    if not renderer:
        raise SystemExit('rsvg-convert is required to render and inspect these studies.')
    for p in PALETTES:
        (ROOT / f'{p["id"]}.svg').write_text(icon(p), encoding='utf-8')
    (ROOT / 'overview.svg').write_text(board(), encoding='utf-8')
    (ROOT / 'index.html').write_text(html(), encoding='utf-8')
    subprocess.run([renderer, str(ROOT / 'overview.svg'), '-o', str(ROOT / 'overview.png')], check=True)
    print(f'Generated six SVG studies, overview.svg/png, and index.html in {ROOT}')


if __name__ == '__main__':
    main()
