"""Toggleable free-fly camera driven by ue.on_tick — Abyssus build (RGame, UE 5.6).

Run once to arm it, then press . (period) in-game to enter/exit freecam.

    Controls
      Period   toggle freecam
      WASD     move (relative to where you look)
      Space    move up (world vertical)
      Shift    move down (world vertical)
      Mouse    look
      Wheel    adjust fly speed
      F        toggle camera light
      G        toggle height fog (freecam only)
      B        toggle sky light boost — global fill light (freecam only)

G and B are the two halves of DRG's "clear view" bundle, split onto their own keys.
G hides every ExponentialHeightFog; B floods the world with ambient light. Both
remember exactly what they changed so toggling back restores it.

Abyssus streams its per-area lighting — the fog and the sky light both live in a
sublevel such as Boatyard_Lighting, not the persistent map — so walking into a new
area brings components that weren't around when you pressed the key. While either
toggle is on, the tick re-sweeps every RESCAN_INTERVAL seconds and applies the
effect to any newcomers (and re-applies it to anything the game switched back).

The B toggle deliberately diverges from DRG. DRG spawns its *own* SkyLight actor
forced onto the engine's DaylightAmbientCubemap. That can't work here for two
reasons: neither cubemap path DRG tries is cooked into Abyssus (ue.load returns
None for both, and DRG's own comment notes a missing cubemap leaves the skylight
dark), and this game already ships a SkyLight per area — UE only lets one win, so
a second would likely be ignored anyway. Instead B boosts the sky light already
there, keeping the area's authored cubemap and just raising Intensity to
max(authored * SKYLIGHT_BOOST, SKYLIGHT_MIN_INTENSITY), un-blacking the lower
hemisphere, and forcing it visible. Intensity is boosted relative to the authored
value so each area keeps its own relative look; the floor covers areas that author
a near-zero intensity, where a multiplier alone would do nothing.

Unlike DRG — which leaves its bundle applied after you leave freecam — both effects
are restored on exit and re-applied on re-entry, mirroring how the camera light
behaves. The `fog_off` / `sky_on` preferences are remembered across toggles. For
DRG's persist-after-exit behavior, drop the _fog_restore / _sky_restore calls in
_exit.

Game-specific notes:
  * Newer engine build (5.6.1): BeginDeferredActorSpawnFromClass /
    FinishSpawningActor take a trailing ESpawnActorScaleMethod arg
    (2 = SelectDefaultAtRuntime).
  * Look sensitivity lives on RGameUserSettings as `AimSensitivity`, with
    `bInvertXAxisAiming` / `bInvertYAxisAiming`. The pawn feeds the Enhanced Input
    IA_LookRight / IA_LookUp values through AddController*Input, which the
    controller scales by the deprecated InputYaw/PitchScale — so the effective
    factor is AimSensitivity * InputScale. MOUSE_GAIN is the single knob if it
    feels off; halve it if look comes out about 2x too fast.
  * Enhanced Input: action events live on BOTH the pawn (move/look/fire) and the
    controller (pause, emotes, chat), so input is disabled on both on enter. Our
    key polling reads UPlayerInput directly and is unaffected. Note this means the
    pause menu is inert while flying — press Period to get out.
  * Unlike the Tokyo Xtreme Racer build, no mouse grab is needed: the controller
    CDO has bShowMouseCursor=False, so gameplay already runs mouse-captured. (The
    cursor you see while the pause menu is up is CommonUI's, not the default.)
"""

import math

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

MOVE_SPEED = 1200.0     # uu/s, starting fly speed
SPEED_STEP = 1.15       # per scroll-wheel notch
MOUSE_GAIN = 1.0        # look sensitivity scale (deg/mouse-unit ~= gain*scale*AimSensitivity)
RESCAN_INTERVAL = 0.5   # s between sweeps for lighting streamed in after G/B was pressed
SKYLIGHT_BOOST = 4.0        # multiplier on the area's authored sky light intensity
SKYLIGHT_MIN_INTENSITY = 6.0    # floor, for areas that author a near-zero intensity
LIGHT_INTENSITY = 5.0
LIGHT_RADIUS = 2500.0
LIGHT_FALLOFF = 8.0
LIGHT_COLOR = (255, 248, 236)  # soft warm-white

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)
PC = gi.LocalPlayers[0].PlayerController
PCM = PC.PlayerCameraManager
GS = ue.find('/Script/Engine.Default__GameplayStatics')
KML = ue.find('/Script/Engine.Default__KismetMathLibrary')


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
    for _fog, _vis in (_prev.get('fogs') or {}).items():
        try:
            _fog.SetVisibility(_vis, False)
        except Exception:
            pass    # streamed out from under us; nothing left to restore
    for _sl, (_i, _lb, _v) in (_prev.get('skylights') or {}).items():
        try:
            _sl.SetIntensity(_i)
            _sl.bLowerHemisphereIsBlack = _lb
            _sl.SetVisibility(_v, False)
            _sl.RecaptureSky()
        except Exception:
            pass
    if _prev.get('active'):
        if _prev.get('pawn') is not None:
            _prev['pawn'].EnableInput(PC)
        PC.EnableInput(PC)
        PC.ResetIgnoreMoveInput(); PC.ResetIgnoreLookInput()
        PC.SetViewTargetWithBlend(PC.K2_GetPawn(), 0.1, 0, 0.0, False)
    if _prev.get('cam') is not None:
        _prev['cam'].K2_DestroyActor()


_xform = KML.MakeTransform(PCM.GetCameraLocation(), PCM.GetCameraRotation(), {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
# Trailing arg (both calls) is ESpawnActorScaleMethod; 2 = SelectDefaultAtRuntime.
_cam = GS.BeginDeferredActorSpawnFromClass(PC, ue.find('/Script/Engine.CameraActor'), _xform, 1, None, 2)  # 1 = AlwaysSpawn
_cam = GS.FinishSpawningActor(_cam, _xform, 2)

_ident = {'Translation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0},
          'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
          'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}
_light = _cam.AddComponentByClass(ue.find('/Script/Engine.PointLightComponent'), False, _ident, False)
_light.bUseInverseSquaredFalloff = False
_light.IntensityUnits = 0   # ELightUnits::Unitless
_light.LightFalloffExponent = LIGHT_FALLOFF
_light.LightColor = {'R': LIGHT_COLOR[0], 'G': LIGHT_COLOR[1], 'B': LIGHT_COLOR[2], 'A': 255}
_light.SetCastShadows(False)
_light.SetAttenuationRadius(LIGHT_RADIUS)
_light.SetIntensity(LIGHT_INTENSITY)
_light.SetVisibility(False, False)

state = {
    'active': False, 'q': (0.0, 0.0, 0.0, 1.0), 'prev_view': None,
    'pawn': None, 'cam': _cam, 'tick': None, 'speed': MOVE_SPEED,
    'light': _light, 'light_on': False,
    'sens_yaw': 1.0, 'sens_pitch': 1.0, 'pitch_dir': -1.0, 'yaw_dir': 1.0,
    'fog_off': False, 'fogs': {},
    'sky_on': False, 'skylights': {},
    'scan_t': 0.0,
}


def _fog_hide(st):
    """Hide every ExponentialHeightFog, remembering each one's visibility the first
    time we touch it. Safe to call repeatedly: already-tracked components are
    re-hidden (Abyssus re-shows an area's fog when you enter it) without clobbering
    the saved value, and fog that streamed in since the last sweep gets picked up.

    Saves bVisible rather than IsVisible(): SetVisibility writes that flag, and
    IsVisible() also folds in bHiddenInGame, so restoring an IsVisible() reading
    would force bVisible=False on any fog that was merely hidden-in-game."""
    for fog in ue.objects_of_class('/Script/Engine.ExponentialHeightFogComponent'):
        if fog._flags & TEMPLATE:
            continue
        try:
            if fog not in st['fogs']:
                st['fogs'][fog] = fog.bVisible
            if fog.bVisible:
                fog.SetVisibility(False, False)
        except Exception:
            pass


def _fog_restore(st):
    """Put every fog back the way we found it. Components whose sublevel streamed
    out resolve stale and raise — there's nothing to restore, so skip them."""
    for fog, vis in st['fogs'].items():
        try:
            fog.SetVisibility(vis, False)
        except Exception:
            pass
    st['fogs'] = {}


def _sky_boost(st):
    """Raise every SkyLight already in the world into a global fill light, keeping
    the area's authored cubemap. Records (Intensity, bLowerHemisphereIsBlack,
    bVisible) the first time each component is seen. Idempotent like _fog_hide, and
    only recaptures when something actually changed — RecaptureSky is not free, and
    the rescan runs twice a second."""
    for sl in ue.objects_of_class('/Script/Engine.SkyLightComponent'):
        if sl._flags & TEMPLATE:
            continue
        try:
            if sl not in st['skylights']:
                st['skylights'][sl] = (sl.Intensity, sl.bLowerHemisphereIsBlack, sl.bVisible)
            target = max(st['skylights'][sl][0] * SKYLIGHT_BOOST, SKYLIGHT_MIN_INTENSITY)
            changed = False
            if not sl.bVisible:
                sl.SetVisibility(True, False)
                changed = True
            if sl.bLowerHemisphereIsBlack:
                sl.bLowerHemisphereIsBlack = False
                changed = True
            if abs(sl.Intensity - target) > 1e-3:
                sl.SetIntensity(target)
                changed = True
            if changed:
                sl.RecaptureSky()
        except Exception:
            pass


def _sky_restore(st):
    """Put every sky light back the way we found it, recapturing so the change
    actually lands. Components whose sublevel streamed out resolve stale; skip."""
    for sl, (intensity, lower_black, vis) in st['skylights'].items():
        try:
            sl.SetIntensity(intensity)
            sl.bLowerHemisphereIsBlack = lower_black
            sl.SetVisibility(vis, False)
            sl.RecaptureSky()
        except Exception:
            pass
    st['skylights'] = {}


def _user_settings():
    """The live RGameUserSettings instance, or None if it hasn't been created."""
    try:
        return next(o for o in ue.objects_of_class('/Script/RGame.RGameUserSettings')
                    if not o._flags & TEMPLATE)
    except StopIteration:
        return None


def _compute_sens():
    """The pawn scales the Enhanced Input look value by AimSensitivity and feeds it
    to AddController*Input, which the controller multiplies by the deprecated
    InputYaw/PitchScale — so the effective look factor is AimSensitivity *
    InputScale. Inversion comes from the bInvert*AxisAiming flags."""
    yaw_scale = abs(PC.InputYawScale) or 2.5
    pitch_scale = abs(PC.InputPitchScale) or 2.5
    aim = 0.4
    invx = invy = False
    gus = _user_settings()
    if gus is not None:
        aim = getattr(gus, 'AimSensitivity', aim) or aim
        invx = bool(getattr(gus, 'bInvertXAxisAiming', False))
        invy = bool(getattr(gus, 'bInvertYAxisAiming', False))
    state['sens_yaw'] = MOUSE_GAIN * yaw_scale * aim
    state['sens_pitch'] = MOUSE_GAIN * pitch_scale * aim
    state['pitch_dir'] = 1.0 if invy else -1.0
    state['yaw_dir'] = -1.0 if invx else 1.0


def _enter():
    pov = PCM.CameraCachePrivate.POV
    state['prev_view'] = PC.GetViewTarget()
    state['pawn'] = PC.K2_GetPawn()
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
    # Pause/emote/chat actions are bound on the controller, move/look/fire on the
    # pawn, so disable both.
    PC.DisableInput(PC)
    if state['pawn'] is not None:
        state['pawn'].DisableInput(PC)
    state['light'].SetVisibility(state['light_on'], False)
    if state['fog_off']:
        _fog_hide(state)
    if state['sky_on']:
        _sky_boost(state)
    state['active'] = True


def _exit():
    if state['pawn'] is not None:
        state['pawn'].EnableInput(PC)
    PC.EnableInput(PC)
    PC.ResetIgnoreMoveInput()
    PC.ResetIgnoreLookInput()
    PC.SetViewTargetWithBlend(state['prev_view'] or PC.K2_GetPawn(), 0.0, 0, 0.0, False)
    state['light'].SetVisibility(False, False)   # don't leave a light floating in the level
    _fog_restore(state)   # ...nor the world's fog switched off while you're playing
    _sky_restore(state)   # ...nor its sky lights cranked up
    state['scan_t'] = 0.0
    state['active'] = False


def _on_tick(world, dt):
    if PC.WasInputKeyJustPressed(_key('Period')):
        _exit() if state['active'] else _enter()
    if not state['active']:
        return

    # toggle the camera light
    if PC.WasInputKeyJustPressed(_key('F')):
        state['light_on'] = not state['light_on']
        state['light'].SetVisibility(state['light_on'], False)

    # toggle height fog
    if PC.WasInputKeyJustPressed(_key('G')):
        state['fog_off'] = not state['fog_off']
        _fog_hide(state) if state['fog_off'] else _fog_restore(state)

    # toggle the sky light boost (global fill light)
    if PC.WasInputKeyJustPressed(_key('B')):
        state['sky_on'] = not state['sky_on']
        _sky_boost(state) if state['sky_on'] else _sky_restore(state)

    # Area lighting streams in as you move: re-apply whichever effects are on, both
    # to components that appeared since the last sweep and to any the game reset.
    if state['fog_off'] or state['sky_on']:
        state['scan_t'] += dt
        if state['scan_t'] >= RESCAN_INTERVAL:
            state['scan_t'] = 0.0
            if state['fog_off']:
                _fog_hide(state)
            if state['sky_on']:
                _sky_boost(state)

    mdx, mdy = PC.GetInputMouseDelta()
    q = state['q']
    if mdx:
        q = _qmul(_qaxis(0.0, 0.0, 1.0, math.radians(mdx * state['sens_yaw'] * state['yaw_dir'])), q)
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


state['tick'] = ue.on_tick(_on_tick)
globals()['_freecam'] = state
print('freecam armed')
