"""Aggregate samply profile leaf frames to find hotspots.

Usage: python tools/analyze_profile.py <profile.json> [--top N] [--process SUBSTR]
"""
import json
import sys
from collections import Counter

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

def resolve(thread, idx):
    """Resolve a stack frame index to a full function name string."""
    stack_table = thread["stackTable"]
    frame_table = thread["frameTable"]
    func_table = thread["funcTable"]
    strings = thread["stringArray"]
    # samply stackTable/frameTable/funcTable use column arrays
    frame_idx = stack_table["frame"][idx]
    func_idx = frame_table["func"][frame_idx]
    name_idx = func_table["name"][func_idx]
    return strings[name_idx]

def resolve_stack(thread, idx):
    """Walk the linked stack table to build the full stack (root->leaf)."""
    stack_table = thread["stackTable"]
    frame_table = thread["frameTable"]
    func_table = thread["funcTable"]
    strings = thread["stringArray"]
    frames = []
    cur = idx
    seen = 0
    while cur is not None and seen < 200:
        frame_idx = stack_table["frame"][cur]
        func_idx = frame_table["func"][frame_idx]
        name_idx = func_table["name"][func_idx]
        frames.append(strings[name_idx])
        cur = stack_table["prefix"][cur]
        seen += 1
    frames.reverse()
    return frames

def main():
    path = sys.argv[1]
    top = 40
    proc_filter = None
    args = sys.argv[2:]
    i = 0
    while i < len(args):
        if args[i] == "--top":
            top = int(args[i + 1]); i += 2
        elif args[i] == "--process":
            proc_filter = args[i + 1]; i += 2
        else:
            i += 1

    prof = load(path)
    leaf_counter = Counter()
    stack_counter = Counter()
    total_weight = 0
    process_samples = 0
    thread_counts = Counter()
    thread_leaf = {}
    for thread in prof["threads"]:
        pname = thread.get("processName", "")
        if proc_filter and proc_filter not in pname:
            continue
        samples = thread["samples"]
        stacks = samples["stack"]
        weights = samples["weight"]
        tname = thread.get("name", "?")
        for i in range(len(stacks)):
            stack_idx = stacks[i]
            w = weights[i]
            total_weight += w
            process_samples += 1
            thread_counts[tname] += w
            if stack_idx is not None:
                leaf = resolve(thread, stack_idx)
                leaf_counter[leaf] += w
                thread_leaf.setdefault(tname, Counter())[leaf] += w
                if "--dump-stack" in sys.argv:
                    stack_counter[tuple(resolve_stack(thread, stack_idx))] += w

    print(f"process filter: {proc_filter!r}")
    print(f"threads matching: {sum(1 for t in prof['threads'] if not proc_filter or proc_filter in t.get('processName',''))}")
    print(f"samples (weight): {process_samples}, total weight: {total_weight}")
    print()
    print(f"Top {top} leaf frames (hot CPU at top of stack):")
    print(f"{'%':>8} {'weight':>8}  function")
    print("-" * 80)
    for name, cnt in leaf_counter.most_common(top):
        pct = cnt / total_weight * 100 if total_weight else 0
        print(f"{pct:7.2f}% {cnt:8d}  {name}")

    if "--dump-stack" in sys.argv:
        print()
        print("Per-thread weight (top 12):")
        for tname, cnt in thread_counts.most_common(12):
            pct = cnt / total_weight * 100 if total_weight else 0
            top_leaf = thread_leaf[tname].most_common(1)[0][0] if thread_leaf.get(tname) else "?"
            print(f"  {pct:6.2f}% {cnt:8d}  {tname}  [leaf: {top_leaf}]")
        print()
        print("Top 5 full call stacks:")
        for stack, cnt in stack_counter.most_common(5):
            pct = cnt / total_weight * 100 if total_weight else 0
            print(f"\n=== {pct:.2f}% ({cnt} samples) ===")
            for frame in stack:
                print(f"  {frame}")

if __name__ == "__main__":
    main()