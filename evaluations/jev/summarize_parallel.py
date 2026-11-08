"""同時送信数の比較結果を集計する。要求数が不足した条件は比較から外す。"""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

from benchmark_parallel import percentile


def summarize(result):
    groups = defaultdict(list)
    excluded = 0
    for batch in result["batches"]:
        if batch["observed_max_inflight"] < batch["client_concurrency"]:
            excluded += 1
            continue
        groups[batch["workload"], batch["client_concurrency"]].append(batch)
    rows = []
    for (workload, concurrency), batches in sorted(groups.items()):
        items = [item for batch in batches for item in batch["items"]]
        durations = [item["seconds"] for item in items]
        valid = sum(item["valid"] for item in items)
        elapsed = sum(batch["wall_seconds"] for batch in batches)
        rows.append({
            "workload": workload, "client_concurrency": concurrency, "rounds": len(batches),
            "requests": len(items), "valid": valid, "correct": sum(item["correct"] for item in items),
            "wall_seconds": elapsed, "valid_per_second": valid / elapsed,
            "mean_seconds": statistics.mean(durations), "p50_seconds": statistics.median(durations),
            "p95_seconds": percentile(durations, 0.95), "max_seconds": max(durations),
            "over_app_timeout": sum(value > 10 for value in durations),
            "min_input_tokens": min((item.get("usage", {}).get("input_tokens", 0) for item in items), default=0),
            "max_input_tokens": max((item.get("usage", {}).get("input_tokens", 0) for item in items), default=0),
        })
    return {"server_before": result["server_before"], "server_after": result.get("server_after"),
            "started_unix_seconds": result["started_unix_seconds"],
            "finished_unix_seconds": result.get("finished_unix_seconds"),
            "excluded_underfilled_batches": excluded, "rows": rows}


if __name__ == "__main__":
    print(json.dumps({Path(name).name: summarize(json.loads(Path(name).read_text()))
                      for name in sys.argv[1:]}, ensure_ascii=False, indent=2))
