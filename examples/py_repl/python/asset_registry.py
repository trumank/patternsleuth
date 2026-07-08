# Drive the Asset Registry from Python: find, filter, inspect and load assets.
#   python repl.py --script asset_registry.py
#
# The registry is an IAssetRegistry, reached via the static
# UAssetRegistryHelpers::GetAssetRegistry(). That returns a TScriptInterface,
# which the bridge now hands back as the implementing UObject (UAssetRegistryImpl)
# -- and the native interface UFUNCTIONs (GetAllAssets, GetAssetsByClass, ...) are
# callable on it even though they live on the interface UClass, not the impl's
# own function map.
#
# Each query function takes its results through an `out` TArray<FAssetData>, so
# the bridge returns a tuple: (bool_return, [FAssetData, ...]). FAssetData is a
# struct -- read ObjectPath / AssetClass / PackageName / AssetName off it.

import ue

HELPERS = '/Script/AssetRegistry.Default__AssetRegistryHelpers'


def desc(a):
    return '%-22s  %s' % (a.AssetClass, a.ObjectPath)


def main():
    helpers = ue.find(HELPERS)
    reg = helpers.GetAssetRegistry()
    print('registry:', repr(reg))
    print('IsLoadingAssets():', reg.IsLoadingAssets())

    # ── every asset the registry knows about ──────────────────────────────────
    ok, allassets = reg.GetAllAssets(True)
    print('\nGetAllAssets(onDisk=True) -> ok=%s, %d assets' % (ok, len(allassets)))
    for a in allassets[:5]:
        print('  ', desc(a))

    # Tally by class so we can pick real values to filter on, no hardcoding.
    by_class = {}
    for a in allassets:
        c = str(a.AssetClass)
        by_class[c] = by_class.get(c, 0) + 1
    top = sorted(by_class.items(), key=lambda kv: -kv[1])[:10]
    print('\ntop asset classes:')
    for c, n in top:
        print('  %5d  %s' % (n, c))

    # ── filter by class ───────────────────────────────────────────────────────
    pick_class = top[0][0]
    ok, byclass = reg.GetAssetsByClass(pick_class, False)
    print('\nGetAssetsByClass(%r) -> %d' % (pick_class, len(byclass)))
    for a in byclass[:3]:
        print('  ', desc(a))

    # ── filter by path (recursive under the package path of a sample asset) ───
    sample = allassets[0]
    pkg_path = str(sample.PackagePath)
    ok, bypath = reg.GetAssetsByPath(pkg_path, True, False)
    print('\nGetAssetsByPath(%r, recursive) -> %d' % (pkg_path, len(bypath)))
    for a in bypath[:3]:
        print('  ', desc(a))

    # ── single lookup by object path ──────────────────────────────────────────
    obj_path = str(sample.ObjectPath)
    one = reg.GetAssetByObjectPath(obj_path, False)
    print('\nGetAssetByObjectPath(%r) ->' % obj_path)
    print('  ', desc(one), ' valid=%s' % helpers.IsValid(one))

    # ── mount paths the registry has cached ───────────────────────────────────
    paths = reg.GetAllCachedPaths()
    print('\nGetAllCachedPaths() -> %d paths, e.g.:' % len(paths))
    for p in paths[:8]:
        print('  ', p)

    # ── load an asset off its FAssetData and confirm it's live ────────────────
    loaded = helpers.GetAsset(sample)
    print('\nGetAsset(%s) -> %r' % (sample.AssetName, loaded))
    if loaded is not None:
        print('   class:', loaded._class, ' loaded-now:', helpers.IsAssetLoaded(sample))

    return 'asset registry OK: %d total assets, %d classes' % (len(allassets), len(by_class))


print(main())
