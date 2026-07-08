"""Toggleable free-fly camera driven by ue.on_tick — DRG on the older 4.25 build.

Same freecam as `freecam.py`, but the camera light is added the way this 4.25
build supports. `AActor::AddComponentByClass` doesn't exist yet, and building a
component by hand + relying on the actor's bulk registration never gives the
light a render proxy (it emits nothing). So we add the light exactly the way
Blueprint's "Add Component" node does: inject a component *template* into a
BlueprintGeneratedClass and call the reflected `AActor::AddComponent` UFUNCTION,
which duplicates the template, attaches it, and `RegisterComponent()`s it — the
step that actually creates the render state (see `_add_component`). The light is
then a real child component of the camera, so it follows for free. (This also
works on newer builds, so the script isn't 4.25-specific in that respect.)

Run once to arm it, then press . (period) in-game to enter/exit freecam.

    Controls
      Period   toggle freecam
      Comma    teleport character to the camera, then exit freecam
      WASD     move (relative to where you look)
      Space    move up (world vertical)
      Shift    move down (world vertical)
      Mouse    look
      Wheel    adjust fly speed
      F        toggle camera light
      G        toggle "clear view" bundle (freecam only)

The G bundle rolls two cave-visibility tweaks into one reversible toggle:
  1. Global fill light   — a SkyLight actor forced onto a bright *specified*
                           cubemap (not scene capture) so it floods the whole
                           world with ambient light instead of capturing the
                           level's own (often dark) lighting.
  2. View distance 20x   — r.ViewDistanceScale, to lift the render-distance cap
                           so distant cave geometry stops popping out.
Toggling off destroys the light and restores the view-distance cvar.

The camera survives level transitions: on a world reload it re-resolves the
PlayerController from the persistent GameInstance and rebuilds the spawned
CameraActor + light (and re-applies the global light if it was on) in the new
world.
"""

import math

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

MOVE_SPEED = 1200.0     # uu/s, starting fly speed
SPEED_STEP = 1.15       # per scroll-wheel notch
LIGHT_INTENSITY = 5.0
LIGHT_RADIUS = 2500.0
LIGHT_FALLOFF = 8.0
LIGHT_COLOR = (255, 248, 236)  # soft warm-white
SENS_DIVISOR = 1000.0

# "Clear view" bundle (G key)
GLOBAL_LIGHT_KEY = 'G'
VIEW_DISTANCE_SCALE = 20.0
SKYLIGHT_INTENSITY = 3.0
# DRG ships the ambient cubemap under /Game (the engine's /Script/MapTemplates
# path the RC build used doesn't exist here — ue.load returned None, leaving the
# SpecifiedCubemap skylight with no cubemap and thus no light).
SKY_CUBEMAP = '/Game/Art/Skyspheres/UEAmbientCubemaps/DaylightAmbientCubemap.DaylightAmbientCubemap'

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)
PC = gi.LocalPlayers[0].PlayerController
PCM = PC.PlayerCameraManager
GS = ue.find('/Script/Engine.Default__GameplayStatics')
GUS = next((o for o in ue.objects_of_class('/Script/FSD.FSDGameUserSettings')), None)
KML = ue.find('/Script/Engine.Default__KismetMathLibrary')
KSL = ue.find('/Script/Engine.Default__KismetSystemLibrary')


# ── runtime component-add: the Blueprint "Add Component" path ─────────────────
# UE 4.25 has no AActor::AddComponentByClass, and a component built by hand +
# registered via the actor's bulk pass never gets a render proxy here. So we add
# components exactly the way Blueprint does: seed a component *template* into a
# BlueprintGeneratedClass's ComponentTemplates array, then call the reflected
# AActor::AddComponent UFUNCTION — it duplicates the template, attaches it, and
# calls RegisterComponent() (which creates the render/physics state). `gi`
# (a BP GameInstance) hosts the template; a native CameraActor can't host one.

_IDENT = {'Translation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0},
          'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
          'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}
_add_ctr = [0]


def _add_component(actor, comp_path, props=None):
    """Add a registered, rendering component of `comp_path` to `actor`, its raw
    properties seeded from `props`. Returns the new component (an independent
    duplicate of the throwaway template)."""
    host = gi._class          # a BlueprintGeneratedClass (BP_GameInstance_C)
    _add_ctr[0] += 1
    name = 'PyFreecam_%d' % _add_ctr[0]
    tmpl = ue.new_object(comp_path, outer=host, name=name,
                         flags=ue.EObjectFlags.RF_Public | ue.EObjectFlags.RF_ArchetypeObject)
    for k, v in (props or {}).items():
        setattr(tmpl, k, v)
    tmpl.add_to_root()
    orig = list(host.ComponentTemplates)
    host.ComponentTemplates = orig + [tmpl]   # make it findable by FindComponentTemplateByName
    try:
        return actor.AddComponent(name, False, _IDENT, gi)   # gi's class == host
    finally:
        host.ComponentTemplates = orig        # restore; the returned component is independent
        tmpl.remove_from_root()


def _key(name):
    return {'KeyName': name}


def _qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz)


def _qaxis(ax, ay, az, ang):
    s = math.sin(ang * 0.5)
    return (ax * s, ay * s, az * s, math.cos(ang * 0.5))


def _qrot(q, v):
    """Rotate vector v by quaternion q."""
    x, y, z, w = q
    vx, vy, vz = v
    tx = 2.0 * (y * vz - z * vy)
    ty = 2.0 * (z * vx - x * vz)
    tz = 2.0 * (x * vy - y * vx)
    return (vx + w * tx + (y * tz - z * ty),
            vy + w * ty + (z * tx - x * tz),
            vz + w * tz + (x * ty - y * tx))


def _qnorm(q):
    x, y, z, w = q
    m = math.sqrt(x * x + y * y + z * z + w * w) or 1.0
    return (x / m, y / m, z / m, w / m)


_prev = globals().get('_freecam')
if _prev is not None:
    try:
        _prev['tick'].remove()
    except Exception:
        pass
    if _prev.get('active'):
        if _prev.get('pawn') is not None:
            _prev['pawn'].EnableInput(PC)
        PC.ResetIgnoreMoveInput(); PC.ResetIgnoreLookInput()
        PC.SetViewTargetWithBlend(PC.K2_GetPawn(), 0.1, 0, 0.0, False)
    for _k in ('skylight', 'cam'):   # the light is a child of 'cam' and dies with it
        if _prev.get(_k) is not None:
            try:
                _prev[_k].K2_DestroyActor()
            except Exception:
                pass


state = {
    'active': False, 'q': (0.0, 0.0, 0.0, 1.0), 'prev_view': None,
    'pawn': None, 'cam': None, 'tick': None, 'speed': MOVE_SPEED,
    'light': None, 'light_on': False,
    'sens_yaw': 0.75, 'sens_pitch': 0.525, 'pitch_dir': -1.0,
    'global_light': False, 'skylight': None,
    'pc': None,   # PlayerController identity we last rebuilt against
}


# ── "clear view" bundle (G) ──────────────────────────────────────────────────
# A SkyLight actor forced onto a bright *specified* cubemap (not scene capture),
# so it floods the whole world with ambient light — spawned as an actor (not a
# component) so K2_DestroyActor can cleanly remove it later — plus an
# r.ViewDistanceScale bump so distant cave geometry stops popping out.

def _cmd(c):
    KSL.ExecuteConsoleCommand(PC, c, None)


def _spawn_skylight():
    xf = KML.MakeTransform({'X': 0.0, 'Y': 0.0, 'Z': 0.0},
                           {'Pitch': 0.0, 'Yaw': 0.0, 'Roll': 0.0},
                           {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
    sky = GS.BeginDeferredActorSpawnFromClass(PC, ue.find('/Script/Engine.SkyLight'), xf, 1, None)
    slc = sky.LightComponent
    slc.SourceType = 1   # ESkyLightSourceType::SLS_SpecifiedCubemap
    slc.Cubemap = ue.load(SKY_CUBEMAP)
    slc.Intensity = SKYLIGHT_INTENSITY
    slc.bLowerHemisphereIsBlack = False
    sky = GS.FinishSpawningActor(sky, xf)
    slc.RecaptureSky()
    return sky


def _global_light_on(st):
    if st.get('skylight') is None:
        st['skylight'] = _spawn_skylight()
    _cmd('r.ViewDistanceScale %g' % VIEW_DISTANCE_SCALE)
    st['global_light'] = True


def _global_light_off(st):
    if st.get('skylight') is not None:
        try:
            st['skylight'].K2_DestroyActor()
        except Exception:
            pass
        st['skylight'] = None
    _cmd('r.ViewDistanceScale 1')
    st['global_light'] = False


# ── level-transition survival ────────────────────────────────────────────────
# On a level change the world is torn down and rebuilt: the PlayerController,
# its camera manager, and our spawned CameraActor + light all get destroyed and
# their proxies go stale. The persistent anchors are the GameInstance (`gi`) and
# its LocalPlayer, whose `.PlayerController` field is repointed at the fresh PC.
# So each tick we re-resolve the PC from `gi`; when it changes identity (or the
# camera has gone stale) we know a transition happened and rebuild in the new
# world.

def _add_flashlight(cam):
    """Add the freecam flashlight as a real point-light *component* of the camera
    (so it tracks the camera for free) via the Blueprint AddComponent path. Its
    component is what the F toggle shows/hides."""
    light = _add_component(cam, '/Script/Engine.PointLightComponent', props={
        'Mobility': 2,                 # EComponentMobility::Movable (runtime dynamic light)
        'Intensity': LIGHT_INTENSITY,
        'AttenuationRadius': LIGHT_RADIUS,
        'bUseInverseSquaredFalloff': False,
        'IntensityUnits': 0,           # ELightUnits::Unitless
        'LightFalloffExponent': LIGHT_FALLOFF,
        'LightColor': {'R': LIGHT_COLOR[0], 'G': LIGHT_COLOR[1], 'B': LIGHT_COLOR[2], 'A': 255},
        'bVisible': False,             # starts off; F toggles it
    })
    light.SetCastShadows(False)
    return light


def _build_cam():
    """(Re)spawn the freecam CameraActor + its child flashlight component in the
    current world and publish them to the module globals the tick loop reads.
    Destroys any prior camera first (which takes its child light with it) so a
    transition rebuild can't leak them."""
    global _cam, _light
    old = state.get('cam')
    if old is not None:
        try:
            old.K2_DestroyActor()
        except Exception:
            pass
    xf = KML.MakeTransform(PCM.GetCameraLocation(), PCM.GetCameraRotation(), {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
    cam = GS.BeginDeferredActorSpawnFromClass(PC, ue.find('/Script/Engine.CameraActor'), xf, 1, None)  # 1 = AlwaysSpawn
    cam = GS.FinishSpawningActor(cam, xf)
    light = _add_flashlight(cam)
    _cam = cam
    _light = light
    state['cam'] = cam
    state['light'] = light


def _resolve_pc():
    """Current PlayerController via the persistent GameInstance, or None if the
    world is mid-transition (no local player / controller yet)."""
    try:
        pc = gi.LocalPlayers[0].PlayerController
        if pc is None:
            return None
        _ = pc._name          # touch it: a stale proxy raises here
        return pc
    except Exception:
        return None


def _cam_valid():
    try:
        _ = _cam._name
        return True
    except Exception:
        return False


def _rebuild(pc):
    """Adopt the freshly-spawned PC and rebuild everything world-bound. Called on
    the first tick after a level transition. Freecam is forced back to inactive —
    the old view target / pawn are gone — so the user simply re-enters with `.`."""
    global PC, PCM
    PC = pc
    PCM = pc.PlayerCameraManager
    # The old world's objects are gone; drop any references to them before we
    # touch state again. Don't try to restore views/input on the dead PC.
    state['active'] = False
    state['pawn'] = None
    state['prev_view'] = None
    _build_cam()
    # If the global light was on, re-spawn it in the new world so it survives the
    # transition too. Its captured actor ref is stale, so clear it first and let
    # _global_light_on recreate from scratch.
    if state.get('global_light'):
        state['skylight'] = None
        try:
            _global_light_on(state)
        except Exception:
            state['global_light'] = False
    state['pc'] = pc


def _compute_sens():
    """Closed-form look factors (deg per mouse-delta unit) from game settings:
    InputYawScale * slider/divisor, matching the game's own conversion."""
    yaw_scale = abs(PC.InputYawScale) or 2.5
    pitch_scale = abs(PC.InputPitchScale) or 1.75
    mx = my = 300.0
    sep = invert = False
    if GUS is not None:
        mx = GUS.MouseXSensitivity
        my = GUS.MouseYSensitivity
        sep = GUS.UseSeperateSensetivity
        invert = GUS.InvertMouse
    state['sens_yaw'] = yaw_scale * (mx / SENS_DIVISOR)
    state['sens_pitch'] = pitch_scale * ((my if sep else mx) / SENS_DIVISOR)
    state['pitch_dir'] = 1.0 if invert else -1.0


def _enter():
    pov = PCM.CameraCachePrivate.POV
    state['prev_view'] = PC.GetViewTarget()
    _compute_sens()
    _cam.K2_SetActorLocationAndRotation(pov.Location, pov.Rotation, False, True)
    qr = _cam.GetTransform().Rotation
    state['q'] = (qr.X, qr.Y, qr.Z, qr.W)

    cc = _cam.CameraComponent
    cc.bConstrainAspectRatio = pov.bConstrainAspectRatio
    cc.AspectRatio = pov.AspectRatio
    cc.SetFieldOfView(pov.FOV)

    PC.SetViewTargetWithBlend(_cam, 0.0, 0, 0.0, False)
    PC.SetIgnoreMoveInput(True)
    PC.SetIgnoreLookInput(True)
    state['pawn'] = PC.K2_GetPawn()
    if state['pawn'] is not None:
        state['pawn'].DisableInput(PC)
    state['light'].SetVisibility(state['light_on'], False)
    state['active'] = True


def _exit():
    if state['pawn'] is not None:
        state['pawn'].EnableInput(PC)
    PC.ResetIgnoreMoveInput()
    PC.ResetIgnoreLookInput()
    PC.SetViewTargetWithBlend(state['prev_view'] or PC.K2_GetPawn(), 0.0, 0, 0.0, False)
    state['light'].SetVisibility(False, False)   # don't leave a light floating in the level
    state['active'] = False


def _teleport_and_exit():
    """Drop the character wherever the freecam is looking from, align the view,
    then leave freecam. K2_TeleportTo does a collision-aware capsule move so the
    pawn isn't left stuck in geometry."""
    loc = _cam.K2_GetActorLocation()
    rot = _cam.K2_GetActorRotation()
    pawn = state['pawn'] or PC.K2_GetPawn()
    if pawn is not None:
        # TeleportTo fits the capsule (won't leave you stuck), but it refuses
        # when the spot can't fit — so fall back to a hard placement so flying
        # into open air / tight nooks still drops you exactly at the camera.
        if not pawn.K2_TeleportTo(loc, rot):
            pawn.K2_SetActorLocation(loc, False, True)
    PC.SetControlRotation(rot)   # exit view resumes facing where the camera looked
    _exit()


def _on_tick(world, dt):
    # Survive level transitions: re-resolve the PC from the persistent
    # GameInstance and rebuild the world-bound bits if it changed or our camera
    # was destroyed. Skip the frame entirely while the world is still loading.
    pc = _resolve_pc()
    if pc is None:
        return
    if pc != state['pc'] or not _cam_valid():
        try:
            _rebuild(pc)
        except Exception:
            return   # world not ready yet; try again next frame

    if PC.WasInputKeyJustPressed(_key('Period')):
        _exit() if state['active'] else _enter()
    if not state['active']:
        return

    # bring the character up to the camera, then drop out of freecam
    if PC.WasInputKeyJustPressed(_key('Comma')):
        _teleport_and_exit()
        return

    # toggle the camera light
    if PC.WasInputKeyJustPressed(_key('F')):
        state['light_on'] = not state['light_on']
        state['light'].SetVisibility(state['light_on'], False)

    # toggle the global fill light (freecam only)
    if PC.WasInputKeyJustPressed(_key(GLOBAL_LIGHT_KEY)):
        _global_light_off(state) if state['global_light'] else _global_light_on(state)

    mdx, mdy = PC.GetInputMouseDelta()
    q = state['q']
    if mdx:
        q = _qmul(_qaxis(0.0, 0.0, 1.0, math.radians(mdx * state['sens_yaw'])), q)
    if mdy:
        q = _qmul(q, _qaxis(0.0, 1.0, 0.0, math.radians(mdy * state['sens_pitch'] * state['pitch_dir'])))
    q = _qnorm(q)
    state['q'] = q

    if PC.WasInputKeyJustPressed(_key('MouseScrollUp')):
        state['speed'] *= SPEED_STEP
    if PC.WasInputKeyJustPressed(_key('MouseScrollDown')):
        state['speed'] /= SPEED_STEP
    speed = state['speed']

    fwd = _qrot(q, (1.0, 0.0, 0.0))
    right = _qrot(q, (0.0, 1.0, 0.0))
    dx = dy = dz = 0.0
    if PC.IsInputKeyDown(_key('W')): dx += fwd[0]; dy += fwd[1]; dz += fwd[2]
    if PC.IsInputKeyDown(_key('S')): dx -= fwd[0]; dy -= fwd[1]; dz -= fwd[2]
    if PC.IsInputKeyDown(_key('D')): dx += right[0]; dy += right[1]; dz += right[2]
    if PC.IsInputKeyDown(_key('A')): dx -= right[0]; dy -= right[1]; dz -= right[2]
    if PC.IsInputKeyDown(_key('SpaceBar')): dz += 1.0
    if PC.IsInputKeyDown(_key('LeftShift')): dz -= 1.0

    loc = _cam.K2_GetActorLocation()
    xform = {
        'Translation': {'X': loc.X + dx * speed * dt,
                        'Y': loc.Y + dy * speed * dt,
                        'Z': loc.Z + dz * speed * dt},
        'Rotation': {'X': q[0], 'Y': q[1], 'Z': q[2], 'W': q[3]},
        'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0},
    }
    _cam.K2_SetActorTransform(xform, False, True)


_build_cam()
state['pc'] = PC   # seed identity so the first tick doesn't spuriously rebuild
state['tick'] = ue.on_tick(_on_tick)
globals()['_freecam'] = state
print('freecam armed (4.25 build)')
