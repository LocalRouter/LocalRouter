#!/usr/bin/env python3
"""Summarize saved runs; optionally render a descriptive routing-budget curve."""
import argparse
import json
import math
import statistics
from benchmark import HERE, metrics, read_jsonl

RUNS = ["routellm", "laya-difficulty", "laya-pair", "kev-difficulty", "kev-pair", "laya-terse", "kev-terse"]


def at_budget(cases, rows, fraction):
    """Rank the full historical sample; this is descriptive, not held-out calibration."""
    historical = {c["id"]: c for c in cases if "weak_correct" in c}
    scored = {r["id"]: r for r in rows if r["id"] in historical and "score" in r}
    if set(scored) != set(historical):
        raise ValueError("Budget comparisons require a score for every historical case")
    if not 0 <= fraction <= 1:
        raise ValueError("Invalid strong fraction")
    ranked = sorted(scored, key=lambda i: (-scored[i]["score"], i))
    count = round(fraction * len(ranked))
    strong = set(ranked[:count])
    quality = sum(c["strong_correct"] if i in strong else c["weak_correct"] for i, c in historical.items()) / len(ranked)
    return {"strong_count": count, "n": len(ranked), "correctness": quality}


def order_effect(original, reversed_rows, threshold=.5):
    a = {r["id"]: r for r in original if "score" in r}
    b = {r["id"]: r for r in reversed_rows if "score" in r}
    ids = sorted(a.keys() & b.keys())
    return {"n": len(ids), "flips": [i for i in ids if (a[i]["score"] >= threshold) != (b[i]["score"] >= threshold)],
            "mean_absolute_score_change": statistics.mean(abs(a[i]["score"] - b[i]["score"]) for i in ids)}


def analyze():
    cases = read_jsonl(HERE / "cases.jsonl")
    output = {}
    for name in RUNS:
        rows = read_jsonl(HERE / (name + ".jsonl"))
        assert len(rows) == len(cases), (name, len(rows))
        assert len({r["id"] for r in rows}) == len(rows)
        by_id = {r["id"]: r for r in rows}
        labelled = [c for c in cases if c.get("policy_label")]
        correct = sum("score" in by_id[c["id"]] and
                      (by_id[c["id"]]["score"] >= .5) == (c["policy_label"] == "strong") for c in labelled)
        output[name] = {"thresholds": [metrics(cases, rows, t / 10) for t in range(1, 10)],
                        "half_budget": at_budget(cases, rows, .5),
                        "subjective_policy_at_half": {"correct": correct, "total": len(labelled),
                            "errors": sum("error" in by_id[c["id"]] for c in labelled)},
                        "curve": [at_budget(cases, rows, t / 100) for t in range(0, 101, 5)]}
    for model in ("laya", "kev"):
        for policy, suffix in (("difficulty", "reversed"), ("terse", "terse-reversed")):
            output[model + "-" + policy]["order_effect"] = order_effect(
                read_jsonl(HERE / (model + "-" + policy + ".jsonl")),
                read_jsonl(HERE / (model + "-" + suffix + ".jsonl")))
        repeat = read_jsonl(HERE / (model + "-repeat.jsonl"))
        groups = {}
        for r in repeat:
            if "score" in r:
                groups.setdefault(r["id"], []).append(r["score"])
        output[model + "-difficulty"]["repeatability"] = {
            "repeats": 3, "valid_cases": len(groups),
            "max_score_spread": max(max(v) - min(v) for v in groups.values()),
            "route_flips": sum(len({s >= .5 for s in v}) > 1 for v in groups.values()),
            "errors": sum("error" in r for r in repeat)}
    (HERE / "summary.json").write_text(json.dumps(output, indent=2) + "\n")
    return output


def plot(summary):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 10, "svg.fonttype": "none"})
    fig, ax = plt.subplots(figsize=(9, 5.7))
    colors = ["#334155", "#b45309", "#d97706", "#0369a1", "#38bdf8", "#15803d", "#6d28d9"]
    for name, color in zip(RUNS, colors):
        curve = summary[name]["curve"]
        ax.plot([100 * r["strong_count"] / r["n"] for r in curve],
                [100 * r["correctness"] for r in curve], label=name, color=color,
                linewidth=2, linestyle="--" if name.endswith("pair") else "-")
    ax.plot([0, 100], [62.5, 83], label="Random at matched strong fraction", color="#737373", linestyle=":", linewidth=2)
    ax.axhline(92.5, color="#9ca3af", linestyle="--", linewidth=1, label="Per-item correctness oracle")
    ax.set(xlabel="Requests sent to GPT-4 (%) — not dollar cost", ylabel="Correctness from historical labels (%)",
           title="Routing on the repository's 200 GSM8K examples", xlim=(0, 100), ylim=(58, 96))
    ax.grid(alpha=.18)
    ax.legend(loc="lower right", fontsize=8)
    fig.text(.1, .015, "Full-sample score ranking, not held-out calibration. Weak: Mixtral-8x7B; strong: GPT-4-1106-preview.\nNo new answers generated. Terse prompts are a post-hoc exploratory ablation.", fontsize=8, color="#475569")
    fig.tight_layout(rect=(0, .07, 1, 1))
    svg = HERE / "routing-curve.svg"
    fig.savefig(svg)
    svg.write_text('\n'.join(line.rstrip() for line in svg.read_text().splitlines()) + '\n')
    fig.savefig(HERE / "routing-curve.png", dpi=160)
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--plot", action="store_true")
    args = parser.parse_args()
    result = analyze()
    if args.plot:
        plot(result)
    for name in RUNS:
        print(name, "50% strong:", result[name]["half_budget"], "policy:", result[name]["subjective_policy_at_half"])
