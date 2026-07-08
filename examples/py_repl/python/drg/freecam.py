"""Toggleable free-fly camera driven by ue.on_tick — DRG (FSD) + Rogue Core.

Run once to arm it, then press . (period) in-game to enter/exit freecam. This one
script covers both Deep Rock titles: base DRG (`/Script/FSD`, older UE4 build) and
Rogue Core (`/Script/RogueCore`, newer UE5 build). The game and engine are detected
at runtime — nothing here is hard-coded to one build:

  * GameUserSettings module — /Script/{FSD,RogueCore}.FSDGameUserSettings, whichever
    is present, so mouse sensitivity comes off the game's own slider either way.
  * Actor-spawn arity — the newer engine's BeginDeferredActorSpawnFromClass /
    FinishSpawningActor take a trailing ESpawnActorScaleMethod arg; we probe once
    (the wrong count raises cleanly before anything spawns) and cache the result.
  * Sky cubemap — the ambient cubemap lives under a different path per build; we
    try the known candidates and use the first that loads.

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
      H        toggle terrain debris (freecam only; Rogue Core only)
      T        open the spawn menu (freecam only; needs drg/spawn_menu.py loaded)
      LMB      spawn the selected actor at the ghost (while in placement mode)
      RMB/Esc  cancel placement mode
      Q / E    rotate the placement ghost

The G "clear view" bundle rolls several cave-visibility tweaks into one reversible
toggle. Four are engine-generic and run on both builds:
  1. Height fog off        (SetVisibility(False) on every ExponentialHeightFog)
  2. View distance 20x     (r.ViewDistanceScale, lifts the render-distance cap)
  3. Global fill light      (a SkyLight actor forced onto a bright *specified*
                            cubemap — not scene capture — so it floods the whole
                            world with ambient light instead of capturing the
                            level's own often-dark lighting)
  4. SSAO off              (r.AmbientOcclusionLevels 0)
A fifth runs on Rogue Core only:
  5. Post-process overlays off (zeroes every PostProcess blendable weight — kills
                            RC's colored-outline "xray" pass and tutorial dither
                            overlay). Skipped on base DRG, whose blendables carry
                            its intended color grading rather than overlays.
Toggling off restores each effect to the value captured when it was toggled on.

The H toggle hides the cosmetic debris the Rogue Core level generator stamps onto
the cave (DebrisInstances HISM components — rubble, crystals, fungee, pillars). Base
DRG has no such class, so H is a no-op there.

The T spawn menu (picker UI in drg/spawn_menu.py) hands back a chosen actor class,
then placement mode rides a ghost of its meshes on a camera line-trace: LMB spawns
the real actor there, RMB/Esc cancels, Q/E rotate. The ghost is preview-only (a bare
Actor carrying copies of the class's mesh templates — no gameplay); the real class
is instantiated only on the LMB click, so its begin-play/AI never fires while you
aim. Load drg/spawn_menu.py first; without it, T just prints a hint.

The camera survives level transitions: on a world reload it re-resolves the
PlayerController from the persistent GameInstance and rebuilds the spawned
CameraActor + light in the new world, re-applying the clear-view bundle and debris
hide if they were on.

Everything lives inside _install() so the script leaks nothing into the shared REPL
namespace except the single persistence handle `_freecam`.
"""


def _install():
    import math

    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    MOVE_SPEED = 1200.0     # uu/s, starting fly speed
    SPEED_STEP = 1.15       # per scroll-wheel notch
    LIGHT_INTENSITY = 5.0
    LIGHT_RADIUS = 2500.0
    LIGHT_FALLOFF = 8.0
    LIGHT_COLOR = (255, 248, 236)  # soft warm-white
    SENS_DIVISOR = 1000.0

    # Spawn menu (T) — picker UI lives in drg/spawn_menu.py
    SPAWN_MENU_KEY = 'T'
    # "Clear view" bundle (G) and terrain-debris toggle (H)
    ENHANCE_KEY = 'G'
    DEBRIS_KEY = 'H'
    VIEW_DISTANCE_SCALE = 20.0
    SKYLIGHT_INTENSITY = 3.0
    # The ambient cubemap ships under different roots per build (DRG bundles its own
    # under /Game; the newer build has the engine's /Engine/MapTemplates one). Try
    # both and use the first that loads — a missing one leaves the skylight dark.
    SKY_CUBEMAPS = (
        '/Engine/MapTemplates/Sky/DaylightAmbientCubemap.DaylightAmbientCubemap',
        '/Game/Art/Skyspheres/UEAmbientCubemaps/DaylightAmbientCubemap.DaylightAmbientCubemap',
    )
    DEBRIS_CLASS = '/Script/RogueCore.DebrisInstances'

    # Placement mode — after a spawn-menu pick, a ghost of the chosen class's mesh
    # components rides a camera line-trace; LMB spawns the real actor there, RMB / Esc
    # cancels, Q/E rotate. The ghost is preview-only (a bare Actor carrying copies of
    # the class's mesh templates — no gameplay); the real class is instantiated only
    # on the LMB click, so its begin-play/AI never fires while you're just aiming.
    GHOST_MATERIAL = '/Game/Art/Materials/_BaseMaterials/MB_HologramMesh.MB_HologramMesh'
    PLACE_MAX_DIST = 20000.0   # camera-forward trace length
    PLACE_YAW_STEP = 15.0      # degrees per Q/E tap

    gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)
    PC = gi.LocalPlayers[0].PlayerController
    PCM = PC.PlayerCameraManager
    GS = ue.find('/Script/Engine.Default__GameplayStatics')
    KML = ue.find('/Script/Engine.Default__KismetMathLibrary')
    KSL = ue.find('/Script/Engine.Default__KismetSystemLibrary')

    # Game detection: FSDGameUserSettings lives in the game module, /Script/FSD on
    # base DRG and /Script/RogueCore on Rogue Core. Whichever class resolves tells
    # us both where to read sensitivity from and which build we're on (IS_RC gates
    # the Rogue-Core-only post-process effect and the debris toggle's usefulness).
    GUS_CLASS = next((p for p in ('/Script/RogueCore.FSDGameUserSettings',
                                  '/Script/FSD.FSDGameUserSettings') if ue.find(p)), None)
    IS_RC = GUS_CLASS is not None and 'RogueCore' in GUS_CLASS
    GUS = next((o for o in ue.objects_of_class(GUS_CLASS) if not o._flags & TEMPLATE), None) \
        if GUS_CLASS else None

    _cam = None    # freecam CameraActor  (bound by _build_cam via nonlocal)
    _light = None  # its child point light
    _scale_arg = None  # spawn arity: None=unprobed, True=needs ESpawnActorScaleMethod, False=not

    _ident = {'Translation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0},
              'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
              'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}

    # ── engine-version-agnostic actor spawning ───────────────────────────────
    # The newer engine build (Rogue Core) added a trailing ESpawnActorScaleMethod
    # arg to BeginDeferredActorSpawnFromClass / FinishSpawningActor; the older DRG
    # build doesn't have it. Probe once — the wrong arg count raises TypeError
    # before anything is spawned — then cache the answer for every later spawn.

    def _begin_spawn(cls, xf):
        nonlocal _scale_arg
        if _scale_arg is None:
            try:
                a = GS.BeginDeferredActorSpawnFromClass(PC, cls, xf, 1, None, 2)
                _scale_arg = True
                return a
            except TypeError:
                _scale_arg = False
        if _scale_arg:
            return GS.BeginDeferredActorSpawnFromClass(PC, cls, xf, 1, None, 2)
        return GS.BeginDeferredActorSpawnFromClass(PC, cls, xf, 1, None)   # 1 = AlwaysSpawn

    def _finish_spawn(actor, xf):
        if _scale_arg:
            return GS.FinishSpawningActor(actor, xf, 2)   # 2 = SelectDefaultAtRuntime
        return GS.FinishSpawningActor(actor, xf)

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

    # ── "clear view" bundle (G) ──────────────────────────────────────────────
    # Every effect stores what it changed on `st` so it can be undone precisely.
    # Four effects are engine-generic (run on both builds); the post-process one is
    # Rogue Core only — zeroing base DRG's blendables would kill its color grading.

    def _cmd(c):
        KSL.ExecuteConsoleCommand(PC, c, None)

    def _load_cubemap():
        for p in SKY_CUBEMAPS:
            cm = ue.load(p)
            if cm is not None:
                return cm
        return None

    def _spawn_skylight():
        """A SkyLight actor forced onto a bright *specified* cubemap (not scene
        capture), so it floods the whole cave with ambient light. Spawned as an
        actor (not a component) so K2_DestroyActor can cleanly remove it later."""
        xf = KML.MakeTransform({'X': 0.0, 'Y': 0.0, 'Z': 0.0},
                               {'Pitch': 0.0, 'Yaw': 0.0, 'Roll': 0.0},
                               {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
        sky = _begin_spawn(ue.find('/Script/Engine.SkyLight'), xf)
        slc = sky.LightComponent
        slc.SourceType = 1   # ESkyLightSourceType::SLS_SpecifiedCubemap
        slc.Cubemap = _load_cubemap()
        slc.Intensity = SKYLIGHT_INTENSITY
        slc.bLowerHemisphereIsBlack = False
        sky = _finish_spawn(sky, xf)
        slc.RecaptureSky()
        return sky

    def _enhance_on(st):
        # 1. height fog off
        st['fogs'] = []
        for fog in ue.objects_of_class('/Script/Engine.ExponentialHeightFogComponent'):
            if fog._flags & TEMPLATE:
                continue
            try:
                st['fogs'].append((fog, fog.IsVisible()))
                fog.SetVisibility(False, False)
            except Exception:
                pass
        # 2. post-process overlays off — Rogue Core only. RC uses PostProcess
        # blendables for the colored-outline "xray" pass and tutorial dither; base
        # DRG uses them for its intended color grading, so leave DRG's alone.
        st['blendables'] = []
        if IS_RC:
            for pp in ue.objects_of_class('/Script/Engine.PostProcessComponent'):
                if pp._flags & TEMPLATE:
                    continue
                try:
                    arr = pp.Settings.WeightedBlendables.Array
                    prev = [e.Weight for e in arr]
                    for e in arr:
                        e.Weight = 0.0
                    st['blendables'].append((pp, prev))
                except Exception:
                    pass
        # 3. global fill light
        if st.get('skylight') is None:
            st['skylight'] = _spawn_skylight()
        # 4. view distance, 5. SSAO off
        _cmd('r.ViewDistanceScale %g' % VIEW_DISTANCE_SCALE)
        _cmd('r.AmbientOcclusionLevels 0')
        st['enhanced'] = True

    def _enhance_off(st):
        for fog, vis in st.get('fogs', []):
            try:
                fog.SetVisibility(vis, False)
            except Exception:
                pass
        st['fogs'] = []
        for pp, prev in st.get('blendables', []):
            try:
                arr = pp.Settings.WeightedBlendables.Array
                for i, w in enumerate(prev):
                    if i < len(arr):
                        arr[i].Weight = w
            except Exception:
                pass
        st['blendables'] = []
        if st.get('skylight') is not None:
            try:
                st['skylight'].K2_DestroyActor()
            except Exception:
                pass
            st['skylight'] = None
        _cmd('r.ViewDistanceScale 1')
        _cmd('r.AmbientOcclusionLevels -1')
        st['enhanced'] = False

    # ── terrain debris toggle (H) ────────────────────────────────────────────
    # The Rogue Core level generator stamps cosmetic meshes onto the cave as
    # DebrisInstances — HISM components holding tens of thousands of
    # rubble/crystal/fungee/pillar instances. Hiding the component hides every
    # instance. Base DRG has no such class, so this no-ops there.

    def _debris_hide(st):
        st['debris'] = []
        st['debris_hidden'] = True
        if ue.find(DEBRIS_CLASS) is None:
            return
        for c in ue.objects_of_class(DEBRIS_CLASS):
            if c._flags & TEMPLATE:
                continue
            try:
                st['debris'].append((c, c.IsVisible()))
                c.SetVisibility(False, False)
            except Exception:
                pass

    def _debris_show(st):
        for c, vis in st.get('debris', []):
            try:
                c.SetVisibility(vis, False)
            except Exception:
                pass
        st['debris'] = []
        st['debris_hidden'] = False

    # ── level-transition survival ────────────────────────────────────────────
    # On a level change the world is torn down and rebuilt: the PlayerController,
    # its camera manager, our spawned CameraActor + light, and every clear-view
    # actor all get destroyed and their proxies go stale. The persistent anchors
    # are the GameInstance (`gi`) and its LocalPlayer, whose `.PlayerController`
    # field is repointed at the fresh PC. So each tick we re-resolve the PC from
    # `gi`; when it changes identity (or the camera has gone stale) we know a
    # transition happened and rebuild everything in the new world.

    def _build_cam():
        """(Re)spawn the freecam CameraActor + its child point light in the
        current world and publish them to the enclosing state the tick loop reads.
        Destroys any prior camera first so a transition rebuild can't leak the old
        one."""
        nonlocal _cam, _light
        old = state.get('cam')
        if old is not None:
            try:
                old.K2_DestroyActor()
            except Exception:
                pass
        xf = KML.MakeTransform(PCM.GetCameraLocation(), PCM.GetCameraRotation(), {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
        cam = _begin_spawn(ue.find('/Script/Engine.CameraActor'), xf)
        cam = _finish_spawn(cam, xf)
        light = cam.AddComponentByClass(ue.find('/Script/Engine.PointLightComponent'), False, _ident, False)
        light.bUseInverseSquaredFalloff = False
        light.IntensityUnits = 0   # ELightUnits::Unitless
        light.LightFalloffExponent = LIGHT_FALLOFF
        light.LightColor = {'R': LIGHT_COLOR[0], 'G': LIGHT_COLOR[1], 'B': LIGHT_COLOR[2], 'A': 255}
        light.SetCastShadows(False)
        light.SetAttenuationRadius(LIGHT_RADIUS)
        light.SetIntensity(LIGHT_INTENSITY)
        light.SetVisibility(False, False)
        _cam = cam
        _light = light
        state['cam'] = cam
        state['light'] = light

    def _resolve_pc():
        """Current PlayerController via the persistent GameInstance, or None if
        the world is mid-transition (no local player / controller yet)."""
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
        """Adopt the freshly-spawned PC and rebuild everything world-bound. Called
        on the first tick after a level transition. Freecam is forced back to
        inactive — the old view target / pawn are gone — so the user simply
        re-enters with `.`."""
        nonlocal PC, PCM
        PC = pc
        PCM = pc.PlayerCameraManager
        # The old world's objects are gone; drop any references to them before we
        # touch state again. Don't try to restore views/input on the dead PC.
        state['active'] = False
        state['pawn'] = None
        state['prev_view'] = None
        state['place_mode'] = False   # ghost died with the old world; drop its ref
        state['ghost'] = None
        _build_cam()
        # If the "clear view" bundle was on, re-apply it in the new world so it
        # survives the transition too. Its captured fog/blendable/skylight refs are
        # stale, so clear them first and let _enhance_on recapture from scratch.
        if state.get('enhanced'):
            state['fogs'] = []
            state['blendables'] = []
            state['skylight'] = None
            try:
                _enhance_on(state)
            except Exception:
                state['enhanced'] = False
        # Same for the debris toggle: the new world's DebrisInstances are fresh and
        # visible, so re-hide them (dropping the stale captured refs first).
        if state.get('debris_hidden'):
            state['debris'] = []
            try:
                _debris_hide(state)
            except Exception:
                state['debris_hidden'] = False
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
        _close_spawn_menu()   # never leave the picker up after leaving freecam
        _exit_place_mode()    # tear down the placement ghost too (placed copies stay)
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

    # ── spawn menu (T) ───────────────────────────────────────────────────────
    # The picker UI lives in drg/spawn_menu.py, which publishes spawn_menu_open()
    # into the shared REPL globals — load that script once before this one. While
    # the menu is open the tick loop suspends freecam (look / move / keys) so the
    # mouse drives the cursor; picking a class enters placement mode. The menu is
    # detected as open via the `_spawn_menu` global it maintains, so a Close-button
    # dismissal (which has no callback) is noticed too.

    def _on_pick(entry):
        cls = ue.load(entry['class_path'])
        state['selected_class'] = cls
        state['selected_name'] = entry['name']
        print('freecam: selected %s -> %s' % (entry['name'], cls))
        if cls is not None:
            _enter_place_mode(cls)

    def _open_spawn_menu():
        opener = globals().get('spawn_menu_open')
        if opener is None:
            print('freecam: spawn menu not loaded — run drg/spawn_menu.py first')
            return
        opener(PC, on_select=_on_pick)

    def _close_spawn_menu():
        closer = globals().get('spawn_menu_close')
        if closer is not None and globals().get('_spawn_menu') is not None:
            try:
                closer()
            except Exception:
                pass

    # ── placement mode ───────────────────────────────────────────────────────
    # The ghost is a bare AActor mirroring the picked class's mesh component
    # templates (read out of its SCS nodes + native CDO components — the real class
    # is never instantiated for the preview, so no begin-play/AI side effects fire
    # while you aim). It re-parents to the camera trace each tick; LMB spawns the
    # real actor at that spot and keeps ghosting for the next one, RMB/Esc cancels.

    _SMC = ue.find('/Script/Engine.StaticMeshComponent')
    _SKMC = ue.find('/Script/Engine.SkeletalMeshComponent')
    _SCENE = ue.find('/Script/Engine.SceneComponent')
    _BLACK = {'R': 0.0, 'G': 0.0, 'B': 0.0, 'A': 0.0}

    def _sattr(o, a):
        try:
            return getattr(o, a)
        except Exception:
            return None

    def _isa(cls, base):
        if cls is None:
            return False
        try:
            return cls == base or any(p == base for p in cls.parents())
        except Exception:
            return False

    def _mesh_templates(cls):
        """Every static/skeletal mesh component template on the class chain, as
        {kind, mesh, loc, rot, scale}. Pure read-only introspection of the SCS
        nodes and native CDO components — nothing is spawned."""
        out = []

        def add(t):
            tc = t._class
            if _isa(tc, _SKMC):
                kind, mesh = 'skel', _sattr(t, 'SkeletalMesh')
            elif _isa(tc, _SMC):
                kind, mesh = 'static', _sattr(t, 'StaticMesh')
            else:
                return
            if mesh is None:
                return
            rl, rr, rs = t.RelativeLocation, t.RelativeRotation, t.RelativeScale3D
            out.append({'kind': kind, 'mesh': mesh,
                        'loc': (rl.X, rl.Y, rl.Z),
                        'rot': (rr.Pitch, rr.Yaw, rr.Roll),
                        'scale': (rs.X, rs.Y, rs.Z)})

        for k in [cls] + list(cls.parents()):
            scs = _sattr(k, 'SimpleConstructionScript')
            if not scs:
                continue
            for n in (_sattr(scs, 'AllNodes') or []):
                t = _sattr(n, 'ComponentTemplate')
                if t is not None:
                    add(t)
        pkg = cls._path.rsplit('.', 1)[0]
        cdo = ue.find(pkg + '.Default__' + cls._name)
        if cdo is not None:
            for base in (_SMC, _SKMC):
                try:
                    comps = cdo.K2_GetComponentsByClass(base)
                except Exception:
                    comps = []
                for c in comps:
                    add(c)
        return out

    def _build_ghost(templates, yaw):
        """Build the preview ghost: a bare Actor mirroring `templates`, every mesh
        under the hologram material with collision off. Runs no gameplay — the real
        class is only ever instantiated later, on commit."""
        xf = KML.MakeTransform({'X': 0.0, 'Y': 0.0, 'Z': 0.0},
                               {'Pitch': 0.0, 'Yaw': yaw, 'Roll': 0.0},
                               {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
        a = _begin_spawn(ue.find('/Script/Engine.Actor'), xf)
        a = _finish_spawn(a, xf)
        a.AddComponentByClass(_SCENE, False, _ident, False)   # identity root at origin
        holo = ue.load(GHOST_MATERIAL)   # None on builds without it → default mats
        for t in templates:
            rt = KML.MakeTransform({'X': t['loc'][0], 'Y': t['loc'][1], 'Z': t['loc'][2]},
                                   {'Pitch': t['rot'][0], 'Yaw': t['rot'][1], 'Roll': t['rot'][2]},
                                   {'X': t['scale'][0], 'Y': t['scale'][1], 'Z': t['scale'][2]})
            base = _SKMC if t['kind'] == 'skel' else _SMC
            c = a.AddComponentByClass(base, False, rt, False)
            try:
                if t['kind'] == 'skel':
                    c.SetSkeletalMesh(t['mesh'], True)   # (NewMesh, bReinitPose)
                else:
                    c.SetStaticMesh(t['mesh'])
            except Exception as e:
                print('freecam: ghost mesh set failed (%s): %s' % (t['kind'], e))
            try:
                c.SetCollisionEnabled(0)   # ECollisionEnabled::NoCollision
            except Exception:
                pass
            if holo is not None:
                for i in range(c.GetNumMaterials()):
                    try:
                        c.SetMaterial(i, holo)
                    except Exception:
                        pass
        a.SetActorEnableCollision(False)
        a.SetActorTickEnabled(False)
        return a

    def _destroy_ghost():
        g = state.get('ghost')
        if g is not None:
            try:
                g.K2_DestroyActor()
            except Exception:
                pass
        state['ghost'] = None

    def _enter_place_mode(cls):
        _exit_place_mode()
        templates = _mesh_templates(cls)
        if not templates:
            print('freecam: %s has no static/skeletal meshes to preview' % cls._name)
            return
        state['selected_class'] = cls   # the class _commit_placement will spawn
        state['ghost_templates'] = templates
        state['place_yaw'] = 0.0
        state['ghost'] = _build_ghost(templates, state['place_yaw'])
        state['place_mode'] = True
        print('freecam: placing %s (%d meshes) — LMB place · RMB/Esc cancel · Q/E rotate'
              % (cls._name, len(templates)))

    def _exit_place_mode():
        _destroy_ghost()
        state['place_mode'] = False
        state['ghost_templates'] = None

    def _place_point():
        """Impact point of a camera-forward line trace, or a point out in front
        when nothing is hit. Ignores the ghost and the freecam camera actor."""
        loc = PCM.GetCameraLocation()
        fv = KML.GetForwardVector(PCM.GetCameraRotation())
        start = {'X': loc.X, 'Y': loc.Y, 'Z': loc.Z}
        end = {'X': loc.X + fv.X * PLACE_MAX_DIST,
               'Y': loc.Y + fv.Y * PLACE_MAX_DIST,
               'Z': loc.Z + fv.Z * PLACE_MAX_DIST}
        ignore = [a for a in (state.get('ghost'), state.get('cam')) if a is not None]
        try:
            hit, out = KSL.LineTraceSingle(PC, start, end, 0, False, ignore, 0, True,
                                           _BLACK, _BLACK, 0.0)
        except Exception:
            hit = False
        if hit:
            ip = out.ImpactPoint
            return {'X': ip.X, 'Y': ip.Y, 'Z': ip.Z}
        return end

    def _commit_placement(pt):
        """Spawn the *real* selected actor at the ghost's spot. The ghost is only a
        preview; the actual class — with all its gameplay — is instantiated only
        here, on an explicit click, never while the preview is following your aim."""
        cls = state.get('selected_class')
        if cls is None:
            return
        xf = KML.MakeTransform(pt, {'Pitch': 0.0, 'Yaw': state['place_yaw'], 'Roll': 0.0},
                               {'X': 1.0, 'Y': 1.0, 'Z': 1.0})
        actor = _begin_spawn(cls, xf)
        actor = _finish_spawn(actor, xf)
        state['placed'].append(actor)
        print('freecam: spawned %s #%d at (%.0f, %.0f, %.0f)'
              % (state.get('selected_name'), len(state['placed']), pt['X'], pt['Y'], pt['Z']))

    def _tick_placement():
        if not state['place_mode']:
            return
        if PC.WasInputKeyJustPressed(_key('RightMouseButton')) or \
           PC.WasInputKeyJustPressed(_key('Escape')):
            _exit_place_mode()
            print('freecam: placement cancelled')
            return
        if PC.WasInputKeyJustPressed(_key('Q')):
            state['place_yaw'] -= PLACE_YAW_STEP
        if PC.WasInputKeyJustPressed(_key('E')):
            state['place_yaw'] += PLACE_YAW_STEP
        pt = _place_point()
        g = state.get('ghost')
        if g is not None:
            g.K2_SetActorLocationAndRotation(
                pt, {'Pitch': 0.0, 'Yaw': state['place_yaw'], 'Roll': 0.0}, False, False)
        if PC.WasInputKeyJustPressed(_key('LeftMouseButton')):
            _commit_placement(pt)

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

        # spawn menu (freecam only): while it's open, the mouse drives the cursor, so
        # suspend the rest of freecam until the user picks a class or hits Close.
        if globals().get('_spawn_menu') is not None:
            return
        if PC.WasInputKeyJustPressed(_key(SPAWN_MENU_KEY)):
            _open_spawn_menu()
            return

        # placement ghost rides the camera trace; runs alongside normal fly controls
        _tick_placement()

        # bring the character up to the camera, then drop out of freecam
        if PC.WasInputKeyJustPressed(_key('Comma')):
            _teleport_and_exit()
            return

        # toggle the camera light
        if PC.WasInputKeyJustPressed(_key('F')):
            state['light_on'] = not state['light_on']
            state['light'].SetVisibility(state['light_on'], False)

        # toggle the "clear view" bundle (freecam only)
        if PC.WasInputKeyJustPressed(_key(ENHANCE_KEY)):
            _enhance_off(state) if state['enhanced'] else _enhance_on(state)

        # hide/show terrain debris (freecam only; Rogue Core only)
        if PC.WasInputKeyJustPressed(_key(DEBRIS_KEY)):
            _debris_show(state) if state['debris_hidden'] else _debris_hide(state)

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

    # ── tear down any previous arming before rebuilding ──────────────────────
    # Every step is guarded independently: after a level transition the prior
    # world's pawn/cam/ghost/skylight proxies are stale, and one stale reference
    # must not abort the whole re-arm — otherwise re-running the script (the way you
    # recover a stuck game) can never complete. We restore the *live* PC's
    # input/view unconditionally so a re-run always un-sticks the game even when the
    # old world's objects are dead.

    def _try(fn):
        try:
            fn()
        except Exception:
            pass

    _prev = globals().get('_freecam')
    if _prev is not None:
        _try(lambda: _prev['tick'].remove())
        _try(lambda: _close_spawn_menu())
        if _prev.get('ghost') is not None:
            _try(lambda: _prev['ghost'].K2_DestroyActor())
        if _prev.get('enhanced'):
            _try(lambda: _enhance_off(_prev))
        if _prev.get('debris_hidden'):
            _try(lambda: _debris_show(_prev))
        if _prev.get('skylight') is not None:
            _try(lambda: _prev['skylight'].K2_DestroyActor())
        if _prev.get('pawn') is not None:
            _try(lambda: _prev['pawn'].EnableInput(PC))
        if _prev.get('cam') is not None:
            _try(lambda: _prev['cam'].K2_DestroyActor())

    # Unconditionally hand control back to the live PC. ResetIgnore* is safe to call
    # even if input wasn't being ignored, so this doubles as a recovery path for a
    # game left stuck (input suppressed / view target on a dead camera).
    _try(lambda: PC.ResetIgnoreMoveInput())
    _try(lambda: PC.ResetIgnoreLookInput())
    _try(lambda: PC.SetViewTargetWithBlend(PC.K2_GetPawn(), 0.1, 0, 0.0, False))

    state = {
        'active': False, 'q': (0.0, 0.0, 0.0, 1.0), 'prev_view': None,
        'pawn': None, 'cam': None, 'tick': None, 'speed': MOVE_SPEED,
        'light': None, 'light_on': False,
        'sens_yaw': 0.75, 'sens_pitch': 0.525, 'pitch_dir': -1.0,
        'enhanced': False, 'skylight': None, 'fogs': [], 'blendables': [],
        'debris_hidden': False, 'debris': [],
        'pc': None,   # PlayerController identity we last rebuilt against
        'selected_class': None, 'selected_name': None,  # last spawn-menu pick
        'place_mode': False, 'ghost': None,             # placement ghost actor
        'ghost_templates': None, 'place_yaw': 0.0, 'placed': [],
    }

    _build_cam()
    state['pc'] = PC   # seed identity so the first tick doesn't spuriously rebuild
    state['tick'] = ue.on_tick(_on_tick)
    globals()['_freecam'] = state
    print('freecam armed (%s)' % ('Rogue Core' if IS_RC else 'DRG'))


_install()
del _install
