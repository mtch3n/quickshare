#!/usr/bin/env python3
"""Generates the icon SVGs in this directory: two swooshes chasing each other."""
import math
from pathlib import Path

SPAN = 150  # degrees each swoosh covers
BLUE, TINT = "#1a73e8", "#5e97f6"


def swoosh(cx, cy, r, start, width, n=48):
    """Swoosh along a circle of radius r, from a sharp tail at `start` to a round head."""
    outer, inner = [], []
    for i in range(n + 1):
        t = i / n
        a = math.radians(start + SPAN * t)
        w = width * t**0.8
        outer.append((cx + (r + w / 2) * math.cos(a), cy + (r + w / 2) * math.sin(a)))
        inner.append((cx + (r - w / 2) * math.cos(a), cy + (r - w / 2) * math.sin(a)))
    pt = lambda p: f"{p[0]:.2f} {p[1]:.2f}"
    d = "M" + " L".join(pt(p) for p in outer)
    d += f" A{width / 2:.2f} {width / 2:.2f} 0 0 1 {pt(inner[-1])} L"
    return d + " L".join(pt(p) for p in reversed(inner[:-1])) + "Z"


def app_icon():
    a, b = (swoosh(256, 256, 112, s, 94) for s in (200, 20))
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <rect x="16" y="16" width="480" height="480" rx="108" fill="#ffffff"/>
  <rect x="17" y="17" width="478" height="478" rx="107" fill="none" stroke="#dde1e6" stroke-width="2"/>
  <path fill="{BLUE}" d="{a}"/>
  <path fill="{TINT}" d="{b}"/>
</svg>
"""


def symbolic(attention):
    a, b = (swoosh(50, 50, 31, s, 30) for s in (200, 20))
    mask = ""
    group = '<g class="ColorScheme-Text" style="fill:currentColor">'
    dot = ""
    if attention:
        mask = """  <mask id="cut">
    <rect width="100" height="100" fill="#fff"/>
    <circle cx="85" cy="15" r="19" fill="#000"/>
  </mask>
"""
        group = group[:-1] + ' mask="url(#cut)">'
        dot = '\n  <circle class="error ColorScheme-NegativeText" cx="85" cy="15" r="12" fill="#e01b24"/>'
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 100 100">
  <style>.ColorScheme-Text {{ color: #2e3436; }}</style>
{mask}  {group}
    <path d="{a}"/>
    <path d="{b}"/>
  </g>{dot}
</svg>
"""


here = Path(__file__).parent
(here / "rquickshare.svg").write_text(app_icon())
(here / "rquickshare-symbolic.svg").write_text(symbolic(False))
(here / "rquickshare-attention-symbolic.svg").write_text(symbolic(True))
