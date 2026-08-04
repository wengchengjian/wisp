"""Resolve samply frame addresses (relative to module base) to symbols using the .syms.json sidecar.

Usage: python scripts/resolve_syms.py <profile.json> <syms.json> [--top N]
"""
import argparse
import bisect
from collections import Counter

from samply_common import load, resolve


def build_sym_map(syms):
    """Return {rva: symbol_name} for the novel_profiler.pdb module.

    known_addresses entries are [rva, symbol_table_index]; the symbol_table_index
    points into symbol_table, whose `symbol` field is the string_table index.
    """
    strings = syms["string_table"]
    result = {}
    for data in syms["data"]:
        if data["debug_name"] != "novel_profiler.pdb":
            continue
        sym_table = data["symbol_table"]
        # known_addresses: [rva, symbol_table_index]
        for rva, sym_idx in data["known_addresses"]:
            if sym_idx < len(sym_table):
                str_idx = sym_table[sym_idx]["symbol"]
                name = strings[str_idx] if str_idx < len(strings) else f"<str{str_idx}>"
            else:
                name = f"<sym{sym_idx}>"
            result[rva] = name
    return result


def main():
    parser = argparse.ArgumentParser(description="Resolve samply frame addresses to symbols via a .syms.json sidecar")
    parser.add_argument("profile", help="path to profile.json")
    parser.add_argument("syms", help="path to the .syms.json sidecar")
    parser.add_argument("--top", type=int, default=40, help="number of top items to show")
    args = parser.parse_args()

    prof = load(args.profile)
    sym_map = build_sym_map(load(args.syms))
    keys = sorted(sym_map.keys())

    # Collect all leaf addresses (address strings) and their weights
    leaf_counter = Counter()
    for thread in prof["threads"]:
        samples = thread["samples"]
        stacks = samples["stack"]
        weights = samples["weight"]
        for i in range(len(stacks)):
            stack_idx = stacks[i]
            w = weights[i]
            if stack_idx is not None:
                leaf = resolve(thread, stack_idx)
                leaf_counter[leaf] += w

    total = sum(leaf_counter.values())
    print(f"total weight: {total}")
    print(f"{'%':>8} {'weight':>8}  rva(hex)        symbol")
    print("-" * 90)
    for addr, cnt in leaf_counter.most_common(args.top):
        rva = int(addr, 16)
        # nearest known address <= rva via binary search
        idx = bisect.bisect_right(keys, rva) - 1
        if idx >= 0:
            best_rva = keys[idx]
            best = sym_map[best_rva]
        else:
            best_rva = None
            best = None
        pct = cnt / total * 100 if total else 0
        label = best if best is not None else "???"
        print(f"{pct:7.2f}% {cnt:8d}  {addr:>10}  {label}  (base rva {best_rva})")


if __name__ == "__main__":
    main()