# Dump every EnemyDescriptor asset's properties to JSON, fully expanded.
#   python repl.py --script dump_enemy_descriptors.py > enemy_descriptors.json
#
# Finds all UEnemyDescriptor assets through the Asset Registry, force-loads each
# one, then walks its reflected properties into a plain JSON tree (one object per
# asset, keyed by ObjectPath). The JSON document is the *only* thing on stdout,
# so `> enemy_descriptors.json` yields a clean, parseable file; the human summary
# (counts / any load failures) goes to stderr.
#
# EnemyDescriptor is a UDataAsset, not a Blueprint, so it loads by its ObjectPath
# directly -- none of the "_C generated class" handling that load_all_assets.py
# needs for cooked Blueprint assets.
#
# Container expansion (the bridge surfaces these as distinct wrapper types,
# discriminated here by type(x).__name__):
#   Struct  -> {field: value}      (fields via dir(), recursed)
#   Array   -> [value, ...]        (iterable, recursed)
#   Set     -> [value, ...]        (iterable, recursed)
#   Map     -> {str(key): value}   (.items(), recursed)
#   UObject -> "<obj path>"        (LEAF: a reference is emitted as its _path,
#              not recursed -- otherwise we'd drag in the whole asset graph and
#              risk cycles. Covers ObjectProperty/ClassProperty/InterfaceProperty
#              and any object fields nested inside structs/arrays.)
# Enums arrive as 'EEnum::Value' strings and soft refs as path strings, so both
# fall through as plain scalars. Verified against ue reflection (UClass.props()):
# every EnemyDescriptor field is one of the kinds above.
#
# Known data quirk, faithfully preserved: SpawnRarityModifiers holds 4 structs of
# uninitialized memory (subnormal junk floats) on descriptors where
# UsesSpawnRarityModifiers is False -- the game never reads them. Not a dump bug.

import json
import sys

import ue

HELPERS = '/Script/AssetRegistry.Default__AssetRegistryHelpers'
ENEMY_DESCRIPTOR = 'EnemyDescriptor'
MAX_DEPTH = 12  # struct nesting guard; object refs are leaves so this is plenty


def conv(x, depth=0):
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
                out[f] = conv(getattr(x, f), depth + 1)
            except Exception as e:
                out[f] = '<err %s>' % e
        return out
    if tn == 'Array' or tn == 'Set':
        try:
            return [conv(e, depth + 1) for e in x]
        except Exception as e:
            return '<iter-err %s>' % e
    if tn == 'Map':
        return {str(conv(k, depth + 1)): conv(v, depth + 1) for k, v in x.items()}
    if tn == 'UObject':
        try:
            return x._path                # reference -> clean object path (leaf)
        except Exception:
            return repr(x)
    return repr(x)                        # unknown kind -> repr, never silently dropped


def dump_obj(obj):
    out = {}
    for a in dir(obj):
        if a.startswith('_'):
            continue
        try:
            out[a] = conv(getattr(obj, a))
        except Exception as e:
            out[a] = '<err %s>' % e
    return out


def main():
    reg = ue.find(HELPERS).GetAssetRegistry()
    ok, assets = reg.GetAssetsByClass(ENEMY_DESCRIPTOR, False)
    paths = sorted(str(a.ObjectPath) for a in assets)

    result = {}
    loaded = failed = 0
    for p in paths:
        obj = ue.load(p)                  # synchronous; EnemyDescriptor is a DataAsset
        if obj is None:
            result[p] = None
            failed += 1
            continue
        result[p] = dump_obj(obj)
        loaded += 1

    print(json.dumps(result, indent=2, sort_keys=True))   # stdout: the JSON only
    sys.stderr.write('dumped %d/%d EnemyDescriptor assets (%d failed to load)\n'
                     % (loaded, len(paths), failed))       # stderr: human summary


main()
