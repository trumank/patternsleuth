# Unlock all 11 biomes on the DRG mission map.
#   run:    python repl.py --script unlock_all_biomes.py
#   revert: ue.unhook('/Script/FSD.MissionGenerationManager', 'GetAvailableMissions')

CLS = '/Script/FSD.MissionGenerationManager'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)
mgm = next(o for o in ue.objects_of_class(CLS) if not o._flags & TEMPLATE)

def missions(seed, rs):
    return mgm.GetMissions({'MapKey': seed.MapKey, 'RandomSeed': seed.RandomSeed + rs, 'Season': seed.Season})

all_biomes = {b for b in ue.objects_of_class('/Script/FSD.Biome') if not b._flags & TEMPLATE and not b.IsTutorialBiome}

CAP = 7

def unlock(this, params):
    roster = params.original()
    seed = gi.GetGlobalMissionSeedNew()
    counts = {}
    for m in roster:
        counts[m.Biome] = counts.get(m.Biome, 0) + 1
    for rs in range(1, 100):
        if all(counts.get(b, 0) >= CAP for b in all_biomes):
            break
        for m in missions(seed, rs):
            if m.Biome and counts.get(m.Biome, 0) < CAP:
                roster.append(m)
                counts[m.Biome] = counts.get(m.Biome, 0) + 1

try:
    ue.unhook(CLS, 'GetAvailableMissions')
except RuntimeError:
    pass
ue.hook(CLS, 'GetAvailableMissions', unlock)
