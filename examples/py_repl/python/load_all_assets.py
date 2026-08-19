# Force-load every asset the Asset Registry knows about, synchronously.
#   python repl.py --script load_all_assets.py
#
# One blocking pass on the game thread: GetAllAssets() enumerates the registry,
# then each asset is loaded with ue.load() (StaticLoadObject). The game is frozen
# for the duration (tens of seconds, GBs of memory), so this is a deliberate
# "load the world" tool, not something to leave armed.
#
# IMPORTANT -- why not GetAsset()/ObjectPath for Blueprints:
#   A Blueprint asset's registry ObjectPath is its editor-only UBlueprint (X.X),
#   which is STRIPPED in a cooked build. Loading that path is a failed/partial
#   load: the package loads but the requested object doesn't exist, leaving the
#   Event-Driven Loader's linker state inconsistent. A later GC then dereferences
#   a null import and fatals on the async loading thread
#   ("Could not find SuperStruct ..."). Loading the real generated class instead
#   (X.X_C, the BlueprintGeneratedClass that IS cooked) is a clean load and
#   sidesteps the crash entirely -- verified crash-free across full-set loads.
# So: Blueprint-type assets are loaded via ObjectPath + "_C"; everything else by
# its ObjectPath directly.

import time

import ue

HELPERS = '/Script/AssetRegistry.Default__AssetRegistryHelpers'


def load_path(asset):
    """The path to actually load for an asset. UBlueprint-derived asset classes
    (Blueprint, WidgetBlueprint, AnimBlueprint, ...) end in 'Blueprint' and have
    their UBlueprint stripped when cooked -- load the generated class instead."""
    path = str(asset.ObjectPath)
    if str(asset.AssetClass).endswith('Blueprint'):
        return path + '_C'
    return path


def main():
    helpers = ue.find(HELPERS)
    reg = helpers.GetAssetRegistry()

    ok, assets = reg.GetAllAssets(True)
    total = len(assets)
    print('force-loading %d assets synchronously...' % total)

    loaded = already = failed = 0
    fail_by_class = {}
    t0 = time.monotonic()
    for a in assets:
        path = load_path(a)
        try:
            was = ue.find(path) is not None
            obj = ue.load(path)               # synchronous StaticLoadObject
            obj.add_to_root()
            if obj is None:
                failed += 1
                c = str(a.AssetClass)
                fail_by_class[c] = fail_by_class.get(c, 0) + 1
            elif was:
                already += 1
            else:
                loaded += 1
        except Exception:
            failed += 1
            c = '%s!' % a.AssetClass        # trailing ! == raised, not just null
            fail_by_class[c] = fail_by_class.get(c, 0) + 1
    dt = time.monotonic() - t0

    print('done in %.1fs' % dt)
    print('  newly loaded  : %d' % loaded)
    print('  already loaded: %d' % already)
    print('  failed/null   : %d' % failed)
    if fail_by_class:
        print('  failures by class (top 15):')
        for c, n in sorted(fail_by_class.items(), key=lambda kv: -kv[1])[:15]:
            print('    %6d  %s' % (n, c))

    return 'loaded %d/%d (%d already, %d null) in %.1fs' % (
        loaded, total, already, failed, dt)


print(main())
