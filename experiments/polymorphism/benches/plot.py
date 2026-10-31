#!/usr/bin/env python3
"""Parse benches/results/vtable.txt and plot it to benches/results/vtable.png.

Usage:
    cargo bench --bench vtable 2>/dev/null > benches/results/vtable.txt
    .venv/bin/python benches/plot.py
"""
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = Path(__file__).parent
src = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "results" / "vtable.txt"
out = Path(sys.argv[2]) if len(sys.argv) > 2 else HERE / "results" / "vtable.png"

scenario = None
data = {}  # scenario -> {label: ns}
for line in src.read_text().splitlines():
    m = re.match(r"^(\d+) objects?, (.*?):?$", line)
    if m:
        n = int(m.group(1))
        scenario = "1 object\n(hot loop)" if n == 1 else f"{n:,} objects"
        data[scenario] = {}
        continue
    m = re.match(r"^\s+(.+?)\s+([\d.]+) ns/call", line)
    if m and scenario:
        data[scenario][m.group(1)] = float(m.group(2))

if not data:
    sys.exit(f"no results parsed from {src}")

# draw order: small -> large, hot loop first
order = sorted(data, key=lambda s: 0 if s.startswith("1 object") else int(s.split()[0].replace(",", "")))
all_labels = list(next(iter(data.values())).keys())
labels = [l for l in all_labels if not l.startswith("map:")]
map_labels = [l for l in all_labels if l.startswith("map:")]
base = labels[0]  # compiler vtable
map_base = map_labels[0] if map_labels else None
map_order = [s for s in order if map_base and map_base in data[s]]

fig = plt.figure(figsize=(17, 11))
gs = fig.add_gridspec(2, 2, width_ratios=[3, 2], height_ratios=[1.25, 1])
ax1 = fig.add_subplot(gs[0, 0])
ax2 = fig.add_subplot(gs[0, 1])
ax3 = fig.add_subplot(gs[1, 0])
ax4 = fig.add_subplot(gs[1, 1])

cmap = plt.get_cmap("tab20")
w = 0.8 / len(labels)
for j, lab in enumerate(labels):
    xs = [i + (j - (len(labels) - 1) / 2) * w for i in range(len(order))]
    ys = [data[s][lab] for s in order]
    ax1.bar(xs, ys, w, label=lab, color=cmap(j), edgecolor="black", linewidth=0.5,
            hatch="//" if lab == base else None)
    for x, y in zip(xs, ys):
        ax1.text(x, y, f"{y:.2f}", ha="center", va="bottom", fontsize=6, rotation=90)
ax1.set_xticks(range(len(order)))
ax1.set_xticklabels(order)
ax1.set_yscale("log")
ax1.set_ylabel("ns per call (log scale, lower is better)")
ax1.set_title("Absolute time per virtual call")
ax1.grid(axis="y", which="both", alpha=0.3)
ax1.legend(fontsize=7, loc="upper left", ncol=2)

# ratios vs compiler vtable
for j, lab in enumerate(labels[1:], start=1):
    ax2.plot(order, [data[s][lab] / data[s][base] for s in order], marker="o", color=cmap(j), label=lab)
ax2.axhline(1.0, color="black", linestyle="--", linewidth=1, label=f"{base} (=1.0)")
ax2.set_yscale("log")
ax2.set_ylabel("time relative to compiler vtable (log, lower is better)")
ax2.set_title("Relative to Box<dyn>")
ax2.grid(alpha=0.3, which="both")
ax2.legend(fontsize=7)

# order-preserving map benchmark
if map_labels:
    w3 = 0.8 / len(map_labels)
    for j, lab in enumerate(map_labels):
        xs = [i + (j - (len(map_labels) - 1) / 2) * w3 for i in range(len(map_order))]
        ys = [data[s][lab] for s in map_order]
        ax3.bar(xs, ys, w3, label=lab, edgecolor="black", linewidth=0.5)
        for x, y in zip(xs, ys):
            ax3.text(x, y, f"{y:.2f}", ha="center", va="bottom", fontsize=8)
    ax3.set_xticks(range(len(map_order)))
    ax3.set_xticklabels(map_order)
    ax3.set_ylabel("ns per object")
    ax3.set_title("Order-preserving map: out[i] = work(object i)")
    ax3.grid(axis="y", alpha=0.3)
    ax3.legend(fontsize=8)
    for lab in map_labels[1:]:
        ax4.plot(map_order, [data[s][lab] / data[s][map_base] for s in map_order],
                 marker="o", label=lab)
    ax4.axhline(1.0, color="black", linestyle="--", linewidth=1, label=f"{map_base} (=1.0)")
    ax4.set_ylabel("time relative to Box<dyn> map")
    ax4.set_title("Map: relative time")
    ax4.grid(alpha=0.3)
    ax4.legend(fontsize=8)
else:
    ax3.axis("off")
    ax4.axis("off")

fig.suptitle("Compiler-generated vtable vs manual vtables and batch techniques", fontsize=14)
fig.tight_layout()
fig.savefig(out, dpi=130)
print(f"wrote {out}")
