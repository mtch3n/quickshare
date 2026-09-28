#!/usr/bin/env python3
"""Generates the icon SVGs in this directory: a ring split into two arrows that swap."""
import math
from pathlib import Path

BLUE = "#2a68f5"
# Proportions as fractions of the ring radius r: shaft offset from the centre
# line, arrowhead arm length along each axis, and the angle (degrees) where each
# arc stops short of the other arrow's shaft. Small sizes spread the arrows
# apart and shorten the heads so they don't blur into each other.
LARGE = dict(lane=0.18, head=0.28, end=355)
SMALL = dict(lane=0.26, head=0.24, end=352)


def mark(cx, cy, r, lane, head, end):
    """Two strokes, each a shaft with an arrowhead that bends into a half ring.

    The second is the first turned half a turn, so the upper arrow points right
    and the lower one left, with their heads meeting on the vertical centre line.
    """
    start = 180 + math.degrees(math.asin(lane))
    end = math.radians(end)
    paths = []
    for s in (1, -1):
        p = lambda x, y: f"{cx + s * x:.2f} {cy + s * y:.2f}"
        y, h = -lane * r, head * r
        paths.append(
            f"M{p(-h, y - h)} L{p(0, y)} L{p(-h, y + h)}"
            f" M{p(0, y)} L{p(r * math.cos(math.radians(start)), y)}"
            f" A{r:.2f} {r:.2f} 0 0 1 {p(r * math.cos(end), r * math.sin(end))}"
        )
    return paths


def strokes(paths, width, attrs=""):
    return "\n".join(
        f'    <path d="{d}"{attrs} fill="none" stroke-width="{width}"'
        ' stroke-linecap="round" stroke-linejoin="round"/>'
        for d in paths
    )


def app_icon(r, width, shape):
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <rect x="16" y="16" width="480" height="480" rx="108" fill="#ffffff"/>
  <rect x="17" y="17" width="478" height="478" rx="107" fill="none" stroke="#dde1e6" stroke-width="2"/>
  <g stroke="{BLUE}">
{strokes(mark(256, 256, r, **shape), width)}
  </g>
</svg>
"""


def symbolic(attention):
    mask = ""
    group = '<g class="ColorScheme-Text" style="stroke:currentColor">'
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
{strokes(mark(50, 50, 40, **SMALL), 10)}
  </g>{dot}
</svg>
"""


here = Path(__file__).parent
(here / "rquickshare.svg").write_text(app_icon(140, 23, LARGE))
# Rendered at 32px and below only.
(here / "rquickshare-small.svg").write_text(app_icon(146, 34, SMALL))
(here / "rquickshare-symbolic.svg").write_text(symbolic(False))
(here / "rquickshare-attention-symbolic.svg").write_text(symbolic(True))
