"""Toggleable free-fly camera driven by ue.on_tick — Ride build.

Run once to arm it, then press . (period) in-game to enter/exit freecam.

    Controls
      Period   toggle freecam
      WASD     move (relative to where you look)
      Space    move up (world vertical)
      Shift    move down (world vertical)
      Mouse    look
      Wheel    adjust fly speed
      F        toggle camera light

Ported from the FSD / Abiotic / Canyon freecams. Two game-specific bits:

  * Newer engine build: BeginDeferredActorSpawnFromClass / FinishSpawningActor
    take a trailing ESpawnActorScaleMethod arg (2 = SelectDefaultAtRuntime).
  * Look sensitivity lives on the custom RGGameUserSettings as a per-axis
    Vector2D `MouseSensitivity` (X=yaw, Y=pitch) plus a `bInvertLook` flag. The
    game feeds those through AddController{Yaw,Pitch}Input, which applies the
    controller InputYaw/PitchScale on top, so the game's own deg-per-mouse-unit
    is simply scale * sensitivity — matched here directly, no fudge factor.
"""

import math

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

MOVE_SPEED = 1200.0     # uu/s, starting fly speed
SPEED_STEP = 1.15       # per scroll-wheel notch
LIGHT_INTENSITY = 5.0
LIGHT_RADIUS = 2500.0
LIGHT_FALLOFF = 8.0
LIGHT_COLOR = (255, 248, 236)  # soft warm-white

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)
PC = gi.LocalPlayers[0].PlayerController
PCM = PC.PlayerCameraManager
GS = ue.find('/Script/Engine.Default__GameplayStatics')
KML = ue.find('/Script/Engine.Default__KismetMathLibrary')
GUS = next((o for o in ue.objects_of_class('/Script/Engine.GameUserSettings') if not o._flags & TEMPLATE), None)


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
    if _prev.get('cam') is not None:
        _prev['cam'].K2_DestroyActor()


_xform = KML.MakeTransform(PCM.GetCameraLocation(), PCM.GetCameraRotation(), {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
# Trailing arg (both calls) is ESpawnActorScaleMethod; 2 = SelectDefaultAtRuntime
# (this game ships the newer engine build that added it).
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
    'sens_yaw': 2.0, 'sens_pitch': 2.0, 'pitch_dir': -1.0,
}


def _compute_sens():
    """Look factors (deg per mouse-delta unit) matching the game's own feel: the
    per-axis MouseSensitivity Vector2D times the controller input scale — the same
    product the engine applies to AddController{Yaw,Pitch}Input."""
    yaw_scale = abs(PC.InputYawScale) or 2.5
    pitch_scale = abs(PC.InputPitchScale) or 2.5
    mx = my = 0.8   # RGGameUserSettings default
    invert = False
    if GUS is not None:
        try:
            m = GUS.MouseSensitivity
            mx, my = m.X, m.Y
        except Exception:
            pass
        invert = bool(getattr(GUS, 'bInvertLook', False))
    state['sens_yaw'] = yaw_scale * mx
    state['sens_pitch'] = pitch_scale * my
    state['pitch_dir'] = 1.0 if invert else -1.0


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


def _on_tick(world, dt):
    if PC.WasInputKeyJustPressed(_key('Period')):
        _exit() if state['active'] else _enter()
    if not state['active']:
        return

    # toggle the camera light
    if PC.WasInputKeyJustPressed(_key('F')):
        state['light_on'] = not state['light_on']
        state['light'].SetVisibility(state['light_on'], False)

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


state['tick'] = ue.on_tick(_on_tick)
globals()['_freecam'] = state
print('freecam armed')
