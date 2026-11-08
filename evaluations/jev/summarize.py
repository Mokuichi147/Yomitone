"""自作評価文の結果を集計する。標準ライブラリだけで実行できる。"""
import json
import math
import statistics
import sys
from pathlib import Path


def percentile(values, p):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * p) - 1)] if ordered else None


def summarize(result):
    events = [event for case in result["cases"] for event in case["events"]]
    scored = [event for event in events if event["correct"] is not None]
    successful = [event for event in events if not event["fallback"]]
    durations = [event["seconds"] for event in successful if event["seconds"] is not None]
    all_durations = [event["seconds"] for event in events if event["seconds"] is not None]
    categories = {}
    for case in result["cases"]:
        group = categories.setdefault(case["category"], {"scored": 0, "correct": 0, "fallbacks": 0})
        for event in case["events"]:
            group["scored"] += int(event["correct"] is not None)
            group["correct"] += int(event["correct"] is True)
            group["fallbacks"] += int(event["fallback"])
    thresholds = {}
    for threshold in [0.5, 0.7, 0.9]:
        accepted = [e for e in scored if e["confidence"] is not None and e["confidence"] >= threshold]
        thresholds[str(threshold)] = {
            "accepted": len(accepted),
            "correct": sum(e["correct"] for e in accepted),
        }
    return {
        "scored": len(scored),
        "correct": sum(e["correct"] for e in scored),
        "fallbacks": sum(e["fallback"] for e in events),
        "fatal_cases": sum(c["fatal_error"] is not None for c in result["cases"]),
        "elapsed_seconds": result["elapsed_seconds"],
        "successful_mean_seconds": statistics.mean(durations) if durations else None,
        "successful_median_seconds": statistics.median(durations) if durations else None,
        "successful_p95_seconds": percentile(durations, 0.95),
        "all_mean_seconds": statistics.mean(all_durations) if all_durations else None,
        "categories": categories,
        "confidence_thresholds": thresholds,
    }


if __name__ == "__main__":
    for filename in sys.argv[1:]:
        path = Path(filename)
        print(path.name)
        print(json.dumps(summarize(json.loads(path.read_text())), ensure_ascii=False, indent=2))
