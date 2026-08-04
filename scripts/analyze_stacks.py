"""Resolve samply profile stacks to real symbols using nativeSymbols and dump hot callsites.

Usage: python scripts/analyze_stacks.py <profile.json> [--process SUBSTR] [--top N] [--stacks N]
"""
import json
import sys
from collections import Counter


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def main():
    path = sys.argv[1]
    top = 40
    stacks = 8
    proc_filter = None
    args = sys.argv[2:]
    i = 0
    while i < len(args):
        if args[i] == "--process":
            proc_filter = args[i + 1]
            i += 2
        elif args[i] == "--top":
            top = int(args[i + 1])
            i += 2
        elif args[i] == "--stacks":
            stacks = int(args[i + 1])
            i += 2
        else:
            i += 1

    prof = load(path)

    for thread in prof["threads"]:
        pname = thread.get("processName", "")
        if proc_filter and proc_filter not in pname:
            continue
        print(f"=== process {pname!r} thread {thread.get('name')} tid={thread.get('tid')} ===")

        sa = thread["stringArray"]
        ft = thread["frameTable"]
        func = thread["funcTable"]
        st = thread["stackTable"]
        ns = thread["nativeSymbols"]

        # Build address -> symbol name map from nativeSymbols
        # nativeSymbols.address / name are parallel arrays; name is a stringArray index
        sym_by_addr = {}
        for j in range(ns["length"]):
            addr = ns["address"][j]
            name_idx = ns["name"][j]
            sym_by_addr[addr] = sa[name_idx] if name_idx < len(sa) else f"<ns{name_idx}>"

        def func_name(func_idx):
            full = sa[func["name"][func_idx]]
            # func name is a full string like "0x165914" or a module name
            if full.startswith("0x"):
                try:
                    addr = int(full, 16)
                except ValueError:
                    return full
                return sym_by_addr.get(addr, full)
            return full

        def resolve_stack(stack_idx):
            """Walk prefix chain, return (root->leaf) list of (funcname, addr)."""
            frames = []
            cur = stack_idx
            seen = 0
            while cur is not None and cur != -1 and seen < 300:
                frame_idx = st["frame"][cur]
                func_idx = ft["func"][frame_idx]
                frames.append(func_name(func_idx))
                cur = st["prefix"][cur]
                seen += 1
            frames.reverse()
            return frames

        samples = thread["samples"]
        leaves = samples["stack"]
        weights = samples["weight"]
        leaf_counter = Counter()
        stack_counter = Counter()
        depth_counter = Counter()
        total = 0
        for k in range(len(leaves)):
            w = weights[k]
            total += w
            s = leaves[k]
            if s is None or s == -1:
                continue
            leaf_counter[func_name(ft["func"][st["frame"][s]])] += w
            stack_counter[tuple(resolve_stack(s))] += w
            depth_counter[len(resolve_stack(s))] += w

        print(f"  total weight: {total}")
        print(f"  top {top} leaf frames:")
        for name, cnt in leaf_counter.most_common(top):
            pct = cnt / total * 100 if total else 0
            print(f"    {pct:7.2f}% {cnt:8d}  {name}")

        print(f"  top {stacks} full stacks:")
        for stack, cnt in stack_counter.most_common(stacks):
            pct = cnt / total * 100 if total else 0
            print(f"\n    === {pct:.2f}% ({cnt} samples) ===")
            for f in stack:
                print(f"      {f}")
        print()


if __name__ == "__main__":
    main()