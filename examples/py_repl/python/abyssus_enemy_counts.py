CONCURRENT = 80
TOTAL_MIN = 200
TOTAL_MAX = 250

AREAS = [
    '/Game/Data/LevelData/Submarine/DA_Submarine_AreaData.DA_Submarine_AreaData',
    '/Game/Data/LevelData/Boatyard/DA_Boatyard_AreaData.DA_Boatyard_AreaData',
    '/Game/Data/LevelData/Gardens/DA_Gardens_AreaData.DA_Gardens_AreaData',
    '/Game/Data/LevelData/Gardens/DA_GardensDemo_AreaData.DA_GardensDemo_AreaData',
    '/Game/Data/LevelData/Sanctuary/DA_Sanctuary_AreaData.DA_Sanctuary_AreaData',
    '/Game/Data/LevelData/Void/DA_Void_AreaData.DA_Void_AreaData',
]
CURVE = '/Game/PrimaryAssets/DifficultySettings/C_Normal_%sEnemyLimitsPerPlayerCurve.C_Normal_%sEnemyLimitsPerPlayerCurve'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

def live(cls):
    return (o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE)

gi = next(live('/Script/Engine.GameInstance'))

def pin(path):
    obj = ue.load(path)
    refs = gi.ReferencedObjects
    if obj not in refs:
        refs.append(obj)
    return obj

for path in AREAS:
    pin(path).MaxAmount = CONCURRENT
for kind, value in (('Min', TOTAL_MIN), ('Max', TOTAL_MAX)):
    pin(CURVE % (kind, kind)).FloatCurve.Keys[0].Value = float(value)
