# Dump every RoomGenerator asset's properties to JSON, fully expanded.
#   python repl.py --script dump_room_generators.py > room_generators.json
#
# Finds all URoomGenerator assets through the Asset Registry, force-loads each
# one, then walks its reflected properties into a plain JSON tree (one object per
# asset, keyed by ObjectPath). The JSON document is the *only* thing on stdout,
# so `> room_generators.json` yields a clean, parseable file; the human summary
# (counts / any load failures) goes to stderr.
#
# RoomGenerator is a UObject asset (like a DataAsset), not a Blueprint, so it
# loads by its ObjectPath directly -- no "_C generated class" handling.
#
# DIFFERENCE FROM dump_enemy_descriptors.py -- owned subobjects are expanded.
# A RoomGenerator keeps almost all of its real data in a tree of RoomFeature
# subobjects (FloodFillLine / FloodFillPillar / EntranceFeature / ...), which the
# reflection surfaces as plain ObjectProperty references. Emitting those as bare
# paths (the EnemyDescriptor approach, where every UObject ref is a leaf) would
# throw away the entire generation definition. So here the walk EXPANDS a
# reference when it points at a subobject *contained in the asset being dumped*
# (its path is "<assetpath>:Sub.Sub..."), and keeps it as a leaf path otherwise
# (external assets, NativeClass, noise-override assets, etc.). A per-asset `seen`
# set breaks cycles: a subobject is expanded once; any further reference to it is
# emitted as its path. Expanded subobjects carry "__class__"/"__path__" so the
# concrete feature type (line vs pillar vs entrance) and identity are preserved.
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
ROOM_GENERATOR = 'RoomGenerator'
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
    ok, assets = reg.GetAssetsByClass(ROOM_GENERATOR, False)
    paths = sorted(str(a.ObjectPath) for a in assets)

    result = {}
    loaded = failed = 0
    for p in paths:
        obj = ue.load(p)                  # synchronous; RoomGenerator is a UObject asset
        if obj is None:
            result[p] = None
            failed += 1
            continue
        result[p] = dump_obj(obj, p, {p})  # seed `seen` with the root path
        loaded += 1

    print(json.dumps(result, indent=2, sort_keys=True))   # stdout: the JSON only
    sys.stderr.write('dumped %d/%d RoomGenerator assets (%d failed to load)\n'
                     % (loaded, len(paths), failed))       # stderr: human summary


main()
