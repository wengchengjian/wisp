"""Aggregate samply profile leaf frames to find hotspots.

Usage: python scripts/analyze_profile.py <profile.json> [--top N] [--process SUBSTR] [--dump-stack]
"""
import argparse
import sys
from collections import Counter

from samply_common import load, resolve, resolve_stack


def main():
    parser = argparse.ArgumentParser(description="Find hot leaf frames in a samply profile")
    parser.add_argument("profile", help="path to profile.json")
    parser.add_argument("--top", type=int, default=40, help="number of top items to show")
    parser.add_argument("--process", default=None, help="only analyze threads whose processName contains this")
    parser.add_argument("--dump-stack", action="store_true", help="also dump full call stacks and per-thread stats")
    args = parser.parse_args()

    prof = load(args.profile)
    leaf_counter = Counter()
    stack_counter = Counter()
    total_weight = 0
    process_samples = 0
    thread_counts = Counter()
    thread_leaf = {}
    for thread in prof["threads"]:
        pname = thread.get("processName", "")
        if args.process and args.process not in pname:
            continue
        samples = thread["samples"]
        stacks = samples["stack"]
        weights = samples["weight"]
        tname = thread.get("name", "?")
        stack_cache = {}
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
                if args.dump_stack:
                    # many samples share the same stack index; resolve once per index
                    cached = stack_cache.get(stack_idx)
                    if cached is None:
                        cached = tuple(resolve_stack(thread, stack_idx))
                        stack_cache[stack_idx] = cached
                    stack_counter[cached] += w

    print(f"process filter: {args.process!r}")
    print(f"threads matching: {sum(1 for t in prof['threads'] if not args.process or args.process in t.get('processName',''))}")
    print(f"samples (weight): {process_samples}, total weight: {total_weight}")
    print()
    print(f"Top {args.top} leaf frames (hot CPU at top of stack):")
    print(f"{'%':>8} {'weight':>8}  function")
    print("-" * 80)
    for name, cnt in leaf_counter.most_common(args.top):
        pct = cnt / total_weight * 100 if total_weight else 0
        print(f"{pct:7.2f}% {cnt:8d}  {name}")

    if args.dump_stack:
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