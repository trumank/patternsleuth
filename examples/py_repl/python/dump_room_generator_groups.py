# Dump every RoomGeneratorGroup asset's properties to JSON, fully expanded.
#   python repl.py --script dump_room_generator_groups.py > room_groups.json
#
# Finds all URoomGeneratorGroup assets through the Asset Registry, force-loads
# each one, then walks its reflected properties into a plain JSON tree (one
# object per asset, keyed by ObjectPath). The JSON document is the *only* thing
# on stdout, so `> room_groups.json` yields a clean, parseable file; the human
# summary (counts / any load failures) goes to stderr.
#
# A RoomGeneratorGroup is thin: a `Rooms` array of references to RoomGenerator
# assets (a curated set the level builder draws from). Those are EXTERNAL assets,
# not owned subobjects, so they're emitted as leaf paths -- each one cross-refs an
# entry in dump_room_generators.py's output. The serializer is the same one used
# there (owned subobjects expand, external refs stay leaves); groups simply have
# no owned subobjects, so every Rooms entry is a path.
#
# Container expansion (bridge wrapper types, discriminated by type(x).__name__):
#   Struct  -> {field: value}        (fields via dir(), recursed)
#   Array   -> [value, ...]          (iterable, recursed)
#   Set     -> [value, ...]          (iterable, recursed)
#   Map     -> {str(key): value}     (.items(), recursed)
#   UObject -> expanded dict if an owned subobject, else "<obj path>" (leaf)
# Enums arrive as 'EEnum::Value' strings and soft refs as path strings, so both
# fall through as plain scalars.

import json
import sys

import ue

HELPERS = '/Script/AssetRegistry.Default__AssetRegistryHelpers'
ROOM_GENERATOR_GROUP = 'RoomGeneratorGroup'
MAX_DEPTH = 32  # subobject trees are shallow; `seen` is the real cycle guard


def is_owned(path, root):
    """True if `path` names a subobject contained in the asset rooted at `root`
    (UE writes contained objects as 'RootPath:Outer.Inner')."""
    return path.startswith(root + ':')


def conv(x, root, seen, depth=0):
    """Recursively convert a bridge value into a JSON-serializable tree."""
    if x is None or isinstance(x, (bool, int, float, str)):
        return x
    tn = type(x).__name__
    if depth > MAX_DEPTH:
        return '<max-depth %s>' % tn
    if tn == 'Struct':
        out = {}
        for f in dir(x):
            if f.startswith('_'):
                continue
            try:
                out[f] = conv(getattr(x, f), root, seen, depth + 1)
            except Exception as e:
                out[f] = '<err %s>' % e
        return out
    if tn == 'Array' or tn == 'Set':
        try:
            return [conv(e, root, seen, depth + 1) for e in x]
        except Exception as e:
            return '<iter-err %s>' % e
    if tn == 'Map':
        return {str(conv(k, root, seen, depth + 1)): conv(v, root, seen, depth + 1)
                for k, v in x.items()}
    if tn == 'UObject':
        try:
            p = x._path
        except Exception:
            return repr(x)
        if is_owned(p, root) and p not in seen:   # contained subobject -> expand once
            seen.add(p)
            return dump_obj(x, root, seen, depth + 1)
        return p                                   # external ref / already seen -> leaf
    return repr(x)                                 # unknown kind -> repr, never dropped


def dump_obj(obj, root, seen, depth=0):
    out = {'__class__': obj._class._path, '__path__': obj._path}
    for a in dir(obj):
        if a.startswith('_'):
            continue
        try:
            out[a] = conv(getattr(obj, a), root, seen, depth + 1)
        except Exception as e:
            out[a] = '<err %s>' % e
    return out


def main():
    reg = ue.find(HELPERS).GetAssetRegistry()
    ok, assets = reg.GetAssetsByClass(ROOM_GENERATOR_GROUP, False)
    paths = sorted(str(a.ObjectPath) for a in assets)

    result = {}
    loaded = failed = 0
    for p in paths:
        obj = ue.load(p)                  # synchronous; RoomGeneratorGroup is a UObject asset
        if obj is None:
            result[p] = None
            failed += 1
            continue
        result[p] = dump_obj(obj, p, {p})  # seed `seen` with the root path
        loaded += 1

    print(json.dumps(result, indent=2, sort_keys=True))   # stdout: the JSON only
    sys.stderr.write('dumped %d/%d RoomGeneratorGroup assets (%d failed to load)\n'
                     % (loaded, len(paths), failed))       # stderr: human summary


main()
