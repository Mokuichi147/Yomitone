"""自作評価文で Jev API の同時送信数と性能を比較する。小説ファイルは読み込まない。"""
import argparse
import concurrent.futures
import copy
import json
import math
import statistics
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path


INSTRUCTIONS = (
    "会話の内容から `推測したいセリフの内容` がどの登場人物による発言かを推測してください。"
    "`今までの内容` と `後の内容` は前後の文脈です。"
    "誰のセリフとも考えられない場合はナレーターを選択してください。"
)


def payloads(fixture, model):
    jobs = []
    for case in json.loads(Path(fixture).read_text()):
        if case.get("source"):
            raise ValueError("このベンチマークは外部の小説ファイルを読み込みません")
        criteria = {"ナレーター": {"性別": "不明", "別名": [], "説明": "世界観の説明などキャラクターの発言ではない内容のナレーションを行う"}}
        for person in case["characters"]:
            criteria[person["name"]] = {
                "性別": person.get("gender") or "不明",
                "別名": person.get("aliases", []),
                "説明": person["portrait"],
            }
        previous = [
            {"セリフ": f"記録{i}。駅前の時計台には午後の日が当たり、広場には買い物帰りの人々が行き交っていた。遠くでは路面電車がゆっくりと通り過ぎ、街路樹の影が歩道に伸びていた。"}
            for i in range(case.get("prefix_repeat", 0))
        ]
        for index, line in enumerate(case["lines"]):
            if line["text"].startswith("「") and line["text"].endswith("」"):
                if line.get("expected") is None:
                    raise ValueError("自作のセリフに正解ラベルがありません")
                body = {
                    "state": {
                        # 各送信で変える識別子を先頭に置き、同じ入力の使い回しを抑える。
                        "評価用ID": "",
                        "今までの内容": copy.deepcopy(previous[-100:]),
                        "推測したいセリフの内容": line["text"],
                        "後の内容": [],
                    },
                    "model": model,
                    "questions": {"speaker": {"type": "choice", "criteria": criteria, "instructions": INSTRUCTIONS}},
                }
                jobs.append({"id": f"{case['id']}-{index}", "expected": line["expected"], "body": body})
            entry = {"セリフ": line["text"]}
            # 同じ章の推測を並列化せず、正解の履歴を与えた独立要求として比較する。
            if line.get("expected") and line["expected"] != "ナレーター":
                entry["話者"] = line["expected"]
            previous.append(entry)
    return jobs


def percentile(values, p):
    return sorted(values)[max(0, math.ceil(len(values) * p) - 1)] if values else None


def request(base_url, body=None, path="/v1/systemone", timeout=60):
    headers = {"Content-Type": "application/json", "Authorization": "Bearer local-evaluation"}
    req = urllib.request.Request(
        base_url + path,
        data=None if body is None else json.dumps(body, ensure_ascii=False).encode(),
        headers=headers,
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=timeout) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def model_config(base_url, model):
    status, raw = request(base_url, path="/api/v1/models")
    if status != 200:
        raise RuntimeError(raw)
    entry = next(m for m in json.loads(raw)["models"] if m["key"] == model)
    return {"type": entry["type"], "loaded_instances": entry["loaded_instances"]}


def run_batch(base_url, jobs, concurrency, run_id):
    lock = threading.Lock()
    active = 0
    peak = 0

    def send(indexed_item):
        nonlocal active, peak
        index, item = indexed_item
        body = copy.deepcopy(item["body"])
        body["state"]["評価用ID"] = f"{run_id}-{index}-{item['id']}"
        started = time.perf_counter()
        with lock:
            active += 1
            peak = max(peak, active)
        result = {"id": item["id"], "expected": item["expected"], "status": None, "valid": False, "correct": False}
        try:
            status, raw = request(base_url, body)
            result["status"] = status
            if status != 200:
                result["error"] = raw
            else:
                response = json.loads(raw)
                answer = response["answers"]["speaker"]
                confidence = answer["confidence"]
                choice = answer["choice"]
                valid = (
                    response["model"] == body["model"]
                    and isinstance(response["usage"], dict)
                    and answer["type"] == "choice"
                    and choice in body["questions"]["speaker"]["criteria"]
                    and isinstance(answer["probabilities"], dict)
                    and isinstance(confidence, (int, float))
                    and math.isfinite(confidence)
                    and 0 <= confidence <= 1
                )
                result.update({"valid": valid, "actual": choice, "confidence": confidence,
                               "correct": valid and choice == item["expected"], "usage": response["usage"]})
                if not valid:
                    result["error"] = "応答の形式が不正です"
        except Exception as error:
            result["error"] = str(error)
        finally:
            with lock:
                active -= 1
        result["seconds"] = time.perf_counter() - started
        return result

    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as executor:
        items = list(executor.map(send, enumerate(jobs)))
    elapsed = time.perf_counter() - start
    valid = sum(item["valid"] for item in items)
    durations = [item["seconds"] for item in items]
    return {
        "client_concurrency": concurrency, "observed_max_inflight": peak, "requests": len(items),
        "retries": 0, "valid": valid, "correct": sum(item["correct"] for item in items),
        "wall_seconds": elapsed, "valid_per_second": valid / elapsed,
        "over_app_timeout": sum(item["seconds"] > 10 for item in items),
        "mean_seconds": statistics.mean(durations), "p50_seconds": statistics.median(durations),
        "p95_seconds": percentile(durations, 0.95), "max_seconds": max(durations), "items": items,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://192.168.11.51:11433")
    parser.add_argument("--model", default="clef-artemis")
    parser.add_argument("--output", default="evaluations/jev/results/tuning/parallel.json")
    parser.add_argument("--concurrency", type=int, nargs="+", default=[1, 2, 4, 8])
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    if args.rounds < 1 or any(n < 1 or n > 8 for n in args.concurrency):
        parser.error("反復数は1以上、同時送信数は1〜8にしてください")
    short = payloads("evaluations/jev/synthetic.json", args.model)
    long = payloads("evaluations/jev/long.json", args.model)
    result = {"started_unix_seconds": time.time(), "base_url": args.base_url, "model": args.model,
              "history_mode": "gold", "previous_count": 100, "following_count": 0,
              "unique_request_ids": True, "server_before": model_config(args.base_url, args.model), "batches": []}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    # 先に1件で長文が通るか確認する。失敗した長文の並列負荷はかけない。
    probe = run_batch(args.base_url, long[:1], 1, "probe")
    result["long_probe"] = probe
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(f"長文の事前確認: 有効応答 {probe['valid']}/{probe['requests']}", flush=True)
    workloads = [("short", short)]
    if probe["valid"]:
        workloads.append(("long", long * 2))
    for workload, jobs in workloads:
        for round_index in range(args.rounds):
            levels = args.concurrency if round_index % 2 == 0 else list(reversed(args.concurrency))
            levels = [level for level in levels if level <= len(jobs)]
            for concurrency in levels:
                batch = run_batch(args.base_url, jobs, concurrency,
                                  f"{workload}-{round_index}-{concurrency}-{time.time_ns()}")
                batch.update({"workload": workload, "round": round_index + 1})
                result["batches"].append(batch)
                output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
                print(f"{workload} 反復{round_index + 1} 同時{concurrency}: "
                      f"有効{batch['valid']}/{batch['requests']} 正解{batch['correct']} "
                      f"{batch['wall_seconds']:.2f}秒 {batch['valid_per_second']:.2f}件/秒 "
                      f"p95={batch['p95_seconds']:.2f}秒", flush=True)
    result["server_after"] = model_config(args.base_url, args.model)
    result["finished_unix_seconds"] = time.time()
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
