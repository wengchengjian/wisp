"""Shared helpers for samply profile analysis scripts.

Provides the common JSON loading, stack-frame resolution, and stack-walking
functions used by analyze_profile.py, analyze_stacks.py and resolve_syms.py.
"""
import json


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


def resolve_stack(thread, idx, max_depth=300):
    """Walk the linked stack table to build the full stack (root->leaf)."""
    stack_table = thread["stackTable"]
    frame_table = thread["frameTable"]
    func_table = thread["funcTable"]
    strings = thread["stringArray"]
    frames = []
    cur = idx
    seen = 0
    while cur is not None and cur != -1 and seen < max_depth:
        frame_idx = stack_table["frame"][cur]
        func_idx = frame_table["func"][frame_idx]
        name_idx = func_table["name"][func_idx]
        frames.append(strings[name_idx])
        cur = stack_table["prefix"][cur]
        seen += 1
    frames.reverse()
    return frames