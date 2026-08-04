"""Resolve samply frame addresses (relative to module base) to symbols using the .syms.json sidecar.

Usage: python tools/resolve_syms.py <profile.json> <syms.json> [--top N]
"""
import json
import sys
from collections import Counter

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

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

def resolve(thread, idx):
    stack_table = thread["stackTable"]
    frame_table = thread["frameTable"]
    func_table = thread["funcTable"]
    strings = thread["stringArray"]
    frame_idx = stack_table["frame"][idx]
    func_idx = frame_table["func"][frame_idx]
    name_idx = func_table["name"][func_idx]
    return strings[name_idx]

def main():
    path = sys.argv[1]
    syms_path = sys.argv[2]
    top = 40
    if "--top" in sys.argv:
        top = int(sys.argv[sys.argv.index("--top") + 1])

    prof = load(path)
    sym_map = build_sym_map(load(syms_path))

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
    for addr, cnt in leaf_counter.most_common(top):
        rva = int(addr, 16)
        # find nearest known address <= rva
        best = None
        best_rva = None
        for k in sorted(sym_map.keys()):
            if k <= rva:
                best = sym_map[k]
                best_rva = k
            else:
                break
        pct = cnt / total * 100 if total else 0
        label = best if best is not None else "???"
        print(f"{pct:7.2f}% {cnt:8d}  {addr:>10}  {label}  (base rva {best_rva})")

if __name__ == "__main__":
    main()