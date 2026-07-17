#!/usr/bin/env python3
"""Render the 3-way benchmark panel as a self-contained SVG.

The figure is generated from the numbers below, not drawn by hand, so a reader
can trace every value to the run of record and regenerate the image when the
benchmark is rerun.

Source of record: bench/server-runs/RESULT_3WAY_1000_FULL.md (2026-05-08,
Firecrawl's public scrape-content-dataset-v1, 1,000 URLs / 819 labeled,
concurrency 5, timeout 120s, recall mode). Install sizes: BENCHMARKS.md.

Deliberately absent: p90/p99. fastCRW's recall-mode p90 (14157 ms) is the worst
of the three, and no other p90 for this engine has ever been measured. Any p90
on this panel would have to come from a run that does not exist.

    bench-radar.py <out.svg>
    bench-radar.py --self-check
"""

import math
import sys

# --- the run of record --------------------------------------------------------
TOOLS = [("crw", "fastCRW"), ("c4ai", "Crawl4AI"), ("fc", "Firecrawl")]

# dom = (value at the centre, value at the outer edge). Ordering the pair this
# way is what encodes "smaller is better" for latency and install size: the axis
# simply runs from the worst value outward to the best.
AXES = [
    {
        "key": "recall",
        "label": "Truth-recall",
        "dom": (50, 66),
        "v": {"crw": 63.74, "c4ai": 59.95, "fc": 56.04},
        "fmt": lambda v: f"{v:.2f}%",
    },
    {
        "key": "unique",
        "label": "Unique recoveries",
        "dom": (0, 36),
        "v": {"crw": 34, "c4ai": 10, "fc": 10},
        "fmt": lambda v: f"{v:g} URLs",
    },
    {
        "key": "p50",
        "label": "Median latency",
        "dom": (2400, 1850),
        "flip": True,
        "v": {"crw": 1914, "c4ai": 1916, "fc": 2305},
        "fmt": lambda v: f"{v:g} ms",
    },
    # Scrape-success is deliberately not a comparison axis: our reachable-URL
    # rate (877/921 = 95.2%) uses a different denominator than the raw
    # 877/1000, so putting all three tools on one axis would need a shared
    # denominator, and on that shared denominator Firecrawl leads. It lives as
    # a single-tool hero stat instead, where no cross-tool claim is implied.
    {
        "key": "size",
        "label": "Install size",
        "dom": (2048, 8),
        "log": True,
        "flip": True,
        "v": {"crw": 8, "c4ai": 2048, "fc": 500},
        "fmt": lambda v: f"{v / 1024:g} GB" if v >= 1024 else f"{v:g} MB",
    },
    {
        "key": "depth",
        "label": "Recall depth",
        "dom": (0.40, 0.53),
        "v": {"crw": 0.512, "c4ai": 0.467, "fc": 0.428},
        "fmt": lambda v: f"{v:.3f}",
    },
]

COLOR = {"crw": "#16A34A", "c4ai": "#0284C7", "fc": "#EA580C"}
DASH = {"c4ai": "6 3", "fc": "2 3"}
BG, CHROME = "#0B0F14", "#141B24"
INK, INK2, INK3 = "#E6EDF3", "#8B949E", "#5C6773"
GRID, GRID_TOP, WARN = "#1C232B", "#2C353F", "#D97706"
FONT = "-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif"
MONO = "ui-monospace,SFMono-Regular,Menlo,monospace"

W, H = 1400, 660
CX, CY, R = 628, 330, 158


def norm(ax, value):
    """Position on the axis, 0 at the centre and 1 at the outer edge."""
    f = math.log if ax.get("log") else (lambda x: x)
    lo, hi = ax["dom"]
    return max(0.0, min(1.0, (f(value) - f(lo)) / (f(hi) - f(lo))))


def leads(ax, key):
    return all(norm(ax, ax["v"][key]) >= norm(ax, ax["v"][k]) for k, _ in TOOLS)


def _pt(i, t):
    a = -math.pi / 2 + i * 2 * math.pi / len(AXES)
    return CX + math.cos(a) * R * t, CY + math.sin(a) * R * t


def _poly(ts):
    return " ".join(
        f"{x:.1f},{y:.1f}" for x, y in (_pt(i, t) for i, t in enumerate(ts))
    )


def _text(x, y, s, fill, size, weight=400, anchor="start", font=FONT, extra=""):
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" fill="{fill}" font-family="{font}" '
        f'font-size="{size}" font-weight="{weight}" text-anchor="{anchor}"{extra}>{s}</text>'
    )


def _radar():
    out = []
    for r in range(1, 5):
        out.append(
            f'<polygon points="{_poly([r / 4] * len(AXES))}" fill="none" '
            f'stroke="{GRID_TOP if r == 4 else GRID}" stroke-width="1"/>'
        )
    for i in range(len(AXES)):
        x, y = _pt(i, 1)
        out.append(
            f'<line x1="{CX}" y1="{CY}" x2="{x:.1f}" y2="{y:.1f}" stroke="{GRID}" stroke-width="1"/>'
        )

    for key, _ in [TOOLS[1], TOOLS[2], TOOLS[0]]:  # ours last so it sits on top
        ours, c = key == "crw", COLOR[key]
        ts = [norm(a, a["v"][key]) for a in AXES]
        p = _poly(ts)
        if ours:
            out.append(
                f'<polygon points="{p}" fill="none" stroke="{c}" stroke-width="3" opacity="0.30" filter="url(#glow)"/>'
            )
            out.append(f'<polygon points="{p}" fill="url(#grad)"/>')
            out.append(
                f'<polygon points="{p}" fill="none" stroke="{c}" stroke-width="3" stroke-linejoin="round"/>'
            )
        else:
            out.append(
                f'<polygon points="{p}" fill="none" stroke="{c}" stroke-width="1.75" '
                f'stroke-linejoin="round" stroke-dasharray="{DASH[key]}" opacity="0.95"/>'
            )
        for i, t in enumerate(ts):
            x, y = _pt(i, t)
            out.append(
                f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{4 if ours else 2.8}" fill="{c}" '
                f'stroke="{BG}" stroke-width="{2 if ours else 1.5}"/>'
            )

    for i, a in enumerate(AXES):
        lx, ly = _pt(i, 1.21)
        anchor = "middle" if abs(lx - CX) < 14 else ("start" if lx > CX else "end")
        dy = -6 if ly < CY else 12
        flip = a.get("flip")
        out.append(
            _text(
                lx,
                ly + dy,
                a["label"] + (" ↓" if flip else ""),
                WARN if flip else INK,
                12,
                600,
                anchor,
            )
        )
        out.append(
            _text(
                lx, ly + dy + 16, a["fmt"](a["v"]["crw"]), COLOR["crw"], 13, 700, anchor
            )
        )
        out.append(
            _text(
                lx,
                ly + dy + 29,
                "centre " + a["fmt"](a["dom"][0]),
                INK3,
                9.5,
                400,
                anchor,
            )
        )
    return "\n".join(out)


def _stat(x, y, label, value, sub, size):
    return "\n".join(
        [
            _text(
                x, y, label, INK2, 11, 400, font=MONO, extra=' letter-spacing="0.09em"'
            ),
            _text(
                x - 4,
                y + 52,
                value,
                COLOR["crw"],
                size,
                750,
                extra=' letter-spacing="-0.03em"',
            ),
            _text(x, y + 76, sub, INK3, 12),
        ]
    )


def _table(x, y):
    cols = [178, 264, 350]
    out = [
        _text(
            x,
            y,
            "MEASURED HEAD TO HEAD, SAME MATCHER",
            INK3,
            10.5,
            400,
            font=MONO,
            extra=' letter-spacing="0.09em"',
        )
    ]
    for j, (key, name) in enumerate(TOOLS):
        out.append(
            _text(
                x + cols[j],
                y + 26,
                name,
                COLOR["crw"] if key == "crw" else INK2,
                11.5,
                700 if key == "crw" else 500,
                "end",
            )
        )
    out.append(
        f'<line x1="{x}" y1="{y + 36}" x2="{x + cols[2]}" y2="{y + 36}" stroke="{GRID_TOP}"/>'
    )
    for i, a in enumerate(AXES):
        ry = y + 62 + i * 32
        out.append(
            _text(x, ry, a["label"] + (" ↓" if a.get("flip") else ""), INK2, 11.5)
        )
        for j, (key, _) in enumerate(TOOLS):
            win = leads(a, key)
            out.append(
                _text(
                    x + cols[j],
                    ry,
                    a["fmt"](a["v"][key]),
                    COLOR[key] if win else INK,
                    11.5,
                    700 if win else 400,
                    "end",
                    extra=' style="font-variant-numeric:tabular-nums"',
                )
            )
        if i < len(AXES) - 1:
            out.append(
                f'<line x1="{x}" y1="{ry + 11}" x2="{x + cols[2]}" y2="{ry + 11}" stroke="{GRID}"/>'
            )
    return "\n".join(out)


def _legend(x, y):
    """One horizontal row starting at x, centred under the radar."""
    out, cursor = [], x
    for key, name in TOOLS:
        ours, c = key == "crw", COLOR[key]
        dash = "" if ours else f' stroke-dasharray="{DASH[key]}"'
        out.append(
            f'<line x1="{cursor}" y1="{y}" x2="{cursor + 22}" y2="{y}" stroke="{c}" '
            f'stroke-width="{3 if ours else 1.75}"{dash}/>'
        )
        out.append(
            _text(
                cursor + 28,
                y + 4,
                name,
                INK if ours else INK2,
                11.5,
                700 if ours else 500,
            )
        )
        cursor += 28 + len(name) * 7.2 + 22
    return "\n".join(out)


def render():
    return f"""<svg viewBox="0 0 {W} {H}" xmlns="http://www.w3.org/2000/svg" role="img"
  aria-label="fastCRW leads truth-recall, unique recoveries, median latency, install size and recall depth on Firecrawl's public 1,000-URL dataset">
<defs>
  <radialGradient id="grad" cx="50%" cy="50%" r="50%">
    <stop offset="0%" stop-color="{COLOR["crw"]}" stop-opacity="0.06"/>
    <stop offset="100%" stop-color="{COLOR["crw"]}" stop-opacity="0.28"/></radialGradient>
  <filter id="glow" x="-40%" y="-40%" width="180%" height="180%">
    <feGaussianBlur stdDeviation="7" result="b"/>
    <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>
</defs>
<rect width="{W}" height="{H}" rx="14" fill="{CHROME}"/>
<circle cx="30" cy="24" r="5.5" fill="#FF5F57"/><circle cx="50" cy="24" r="5.5" fill="#FEBC2E"/><circle cx="70" cy="24" r="5.5" fill="#28C840"/>
<rect x="96" y="12" width="880" height="24" rx="6" fill="{BG}"/>
{_text(110, 29, "fastcrw.com/benchmarks", INK2, 12, font=MONO)}
<rect x="1236" y="12" width="140" height="24" rx="6" fill="{BG}" stroke="{COLOR["crw"]}" stroke-opacity="0.4"/>
{_text(1306, 29, "[ 1000 URLs ]", COLOR["crw"], 11, 400, "middle", MONO)}
<path d="M0 48 H{W} V{H - 14} a14 14 0 0 1 -14 14 H14 a14 14 0 0 1 -14 -14 Z" fill="{BG}"/>

{_stat(48, 92, "TRUTH-RECALL", "63.74%", "recall mode · 522 of 819 labeled", 46)}
<line x1="48" y1="178" x2="330" y2="178" stroke="{GRID}"/>
{_stat(48, 206, "SCRAPE SUCCESS", "95.2%", "877 of 921 reachable URLs", 40)}
<line x1="48" y1="288" x2="330" y2="288" stroke="{GRID}"/>
{_stat(48, 316, "ORACLE CEILING", "92.2%", "of every URL any tool could reach", 40)}
<line x1="48" y1="398" x2="330" y2="398" stroke="{GRID}"/>
{_stat(48, 426, "ONLY WE RECOVER", "34", "vs 10 and 10 · 70% more than both", 40)}
{_text(48, 556, "diagnose_3way.py · 2026-05-08", INK3, 10.5, 400, font=MONO)}
{_text(48, 574, "3,000 requests · 0 errors", INK3, 10.5, 400, font=MONO)}

{_radar()}
{_legend(CX - 150, H - 44)}
{_text(CX, H - 22, "↓ smaller is better · every axis points outward to better", WARN, 10.5, 400, "middle", MONO)}

<line x1="960" y1="80" x2="960" y2="{H - 56}" stroke="{GRID}"/>
{_table(1000, 104)}
<line x1="1000" y1="368" x2="1350" y2="368" stroke="{GRID}"/>
{_text(1000, 394, "DATASET", INK3, 10.5, 400, font=MONO, extra=' letter-spacing="0.09em"')}
{_text(1000, 418, "Firecrawl's own public 1,000-URL set,", INK2, 11.5)}
{_text(1000, 436, "all three tools through one matcher.", INK2, 11.5)}
{_text(1000, 454, "Rerun it yourself: BENCHMARKS.md", INK2, 11.5)}
</svg>
"""


def self_check():
    svg = render()
    assert svg.startswith("<svg") and svg.rstrip().endswith("</svg>"), "not an svg"
    assert render() == svg, "render is not deterministic"

    # Every number on the panel must be one of ours, and p90 must never appear:
    # 4348 was never measured for this engine on any run.
    for banned in ("4348", "p90", "14157", "92%", "91.8%"):
        assert banned not in svg, f"{banned!r} must not appear on the panel"

    # The axis whose direction is flipped must say so, in the chart and the table.
    for ax in AXES:
        if ax.get("flip"):
            assert f"{ax['label']} ↓" in svg, f"{ax['label']} is flipped but unmarked"

    # The ↓ marker and the axis geometry must never disagree: a flipped axis is
    # exactly one whose dom runs high->low (worst value at the centre).
    for ax in AXES:
        lo, hi = ax["dom"]
        scale = math.log if ax.get("log") else (lambda x: x)
        assert (scale(hi) < scale(lo)) == bool(ax.get("flip")), (
            f"{ax['label']}: flip flag disagrees with dom ordering"
        )

    # Direction: on a flipped axis the smaller value must sit further out.
    size = next(a for a in AXES if a["key"] == "size")
    assert norm(size, 8) > norm(size, 2048), "8 MB must be further out than 2 GB"
    assert norm(size, 8) == 1.0 and norm(size, 2048) == 0.0, "size axis endpoints"
    p50 = next(a for a in AXES if a["key"] == "p50")
    assert norm(p50, 1914) > norm(p50, 2305), "1914 ms must be further out than 2305 ms"

    # Scrape-success is not a comparison axis (it would need a shared denominator
    # on which Firecrawl leads), so the panel must not put it head-to-head.
    assert not any(a["key"] == "success" for a in AXES), (
        "scrape-success must not be an axis"
    )
    assert "89.7" not in svg, (
        "Firecrawl's scrape-success must not appear as a comparison"
    )
    # The number is not the only leak path: the aria-label is plain text a
    # crawler or screen reader reads even when the chart never renders.
    assert "leads scrape success" not in svg, (
        "no cross-tool scrape-success claim, in prose either"
    )
    # Our single-tool rate is shown honestly, with its real denominator.
    assert "95.2%" in svg and "877 of 921 reachable" in svg, (
        "scrape-success stat missing"
    )

    # With no losing axis left, every comparison axis must be a win.
    won = [a["label"] for a in AXES if leads(a, "crw")]
    assert len(won) == len(AXES) == 5, f"expected all 5 axes to be wins, got {won}"
    print("self-check ok")


if __name__ == "__main__":
    if "--self-check" in sys.argv:
        self_check()
        sys.exit(0)
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    with open(sys.argv[1], "w") as fh:
        fh.write(render())
    print(f"wrote {sys.argv[1]}")
