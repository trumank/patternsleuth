//! Native Python proxies for UE reflection (RustPython edition).
//!
//! The headline use case is `obj.items[2].name = "new name"`:
//!   - `obj` is a `UObject` proxy holding an [`ue::FWeakObjectPtr`].
//!   - `.items` calls the `GetAttr` slot, which finds the FProperty on
//!     `obj.class()` and, since it's an FArrayProperty, returns an `Array`
//!     proxy aimed at the TArray inside `obj`.
//!   - `[2]` calls the mapping subscript, walks to element 2; the element is
//!     an FStructProperty, so we return a `Struct` proxy aimed at that struct.
//!   - `.name = "new name"` calls the `SetAttr` slot on the struct, which
//!     finds the FStrProperty inside the UStruct and writes the FString.
//!
//! Every proxy carries the *owning* UObject's weak pointer and revalidates it
//! on each access — so if the owner gets GC'd, we get a clean AttributeError
//! rather than a use-after-free.
//!
//! Threading: RustPython's `PyObjectRef` is `Rc`-based (single-threaded), and
//! all REPL execution + every engine callback (hooks, delegate sinks, begin-play,
//! tick, trace) runs on the engine/main thread. So all callback-registry state
//! lives in one thread-local, `REGISTRIES` — see its doc for the layout and the
//! never-dropped (`ManuallyDrop`) policy every game-thread TLS in this DLL
//! follows. The cross-thread exceptions are a binding's `SinkHeader` (read by
//! the Blueprint VM on worker threads; see the native sink table) and
//! `SAVED_NATIVES` (read by the trace handler on any thread running bytecode).

use rustpython_vm::pymodule;

pub(crate) use ue_module::{dispatch_begin_play, dispatch_world_tick, module_def};

#[pymodule(name = "ue")]
pub(crate) mod ue_module {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::rc::Rc;
    use std::sync::atomic::Ordering;

    use rustpython_vm::FromArgs;
    use rustpython_vm::builtins::{PyBaseExceptionRef, PyDict, PyFloat, PyModule, PyStr, PyType};
    use rustpython_vm::class::PyClassDef;
    use rustpython_vm::convert::ToPyObject;
    use rustpython_vm::function::{FuncArgs, OptionalArg, PyComparisonValue, PySetterValue};
    use rustpython_vm::protocol::{PyMappingMethods, PySequenceMethods};
    use rustpython_vm::types::{
        AsMapping, AsSequence, Callable, Comparable, Constructor, GetAttr, Hashable, Iterable,
        PyComparisonOp, Representable, SetAttr,
    };
    // Note: `pyfunction`/`pygetset`/`pymethod`/`pyattr`/`pyslot` are *helper*
    // attributes processed by the `#[pyclass]` / `#[pymodule]` macros — they are
    // not standalone importable macros (only `pyclass`/`pymodule` are).
    use rustpython_vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func, pyclass,
    };

    use crate::ue;
    use crate::ue::PropTrait;

    type PyHash = rustpython_vm::common::hash::PyHash;

    // ── tiny conveniences ───────────────────────────────────────────────────
    fn attr_err(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_attribute_error(msg.into())
    }
    fn type_err(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_type_error(msg.into())
    }
    fn value_err(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_value_error(msg.into())
    }
    fn index_err(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_index_error(msg.into())
    }
    fn runtime_err(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_runtime_error(msg.into())
    }
    fn key_err(vm: &VirtualMachine, msg: &str) -> PyBaseExceptionRef {
        vm.new_key_error(vm.ctx.new_str(msg).into())
    }

    /// Every proxy's `Debug` is just its type name (the payloads are raw pointers
    /// / weak refs that aren't usefully printable), so generate them in one shot.
    macro_rules! debug_name {
        ($($t:ident),+ $(,)?) => {
            $(impl std::fmt::Debug for $t {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(stringify!($t))
                }
            })+
        };
    }
    /// None of these proxies can be built from Python — every one wraps engine
    /// memory it has no way to conjure. Without this they inherit
    /// `object.__new__` and hand back an instance with no payload, which then
    /// fails deep inside some later slot ("unexpected payload for __repr__").
    /// Subclasses inherit the slot, so only the roots of each chain need it.
    macro_rules! unconstructible {
        ($($t:ident),+ $(,)?) => {
            $(impl Constructor for $t {
                type Args = FuncArgs;
                fn py_new(_cls: &Py<PyType>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
                    Err(type_err(
                        vm,
                        format!("cannot create ue.{} instances", <$t as PyClassDef>::NAME),
                    ))
                }
            })+
        };
    }
    unconstructible!(
        UObject,
        PyFProperty,
        Param,
        Function,
        Struct,
        Array,
        Map,
        Set,
        Text,
        Delegate,
        MulticastDelegate,
        DelegateBinding,
        HookParams,
        BeginPlayHook,
        TickHook,
        Trace,
        BoundMethod,
    );

    debug_name!(
        UObject,
        UStruct,
        UClass,
        PyFProperty,
        Param,
        Function,
        Struct,
        Array,
        Map,
        Set,
        Delegate,
        MulticastDelegate,
        DelegateBinding,
        HookParams,
        BeginPlayHook,
        TickHook,
        Trace,
        BoundMethod,
    );

    /// Build a Python `list` from any iterator of `ToPyObject`. RustPython only
    /// implements `ToPyObject` for `Vec<PyObjectRef>` / `Vec<u8>`, so any method
    /// returning a `Vec<T>` of something else has to funnel through here.
    fn to_list<T: ToPyObject>(
        vm: &VirtualMachine,
        items: impl IntoIterator<Item = T>,
    ) -> PyObjectRef {
        vm.ctx
            .new_list(items.into_iter().map(|x| x.to_pyobject(vm)).collect())
            .into()
    }

    /// Collect a Python iterable into `Vec<PyObjectRef>` via the iterator protocol.
    fn py_iter_collect(vm: &VirtualMachine, value: &PyObject) -> PyResult<Vec<PyObjectRef>> {
        let iter = value.get_iter(vm)?;
        let mut out = Vec::new();
        use rustpython_vm::protocol::PyIterReturn;
        loop {
            match iter.next(vm)? {
                PyIterReturn::Return(item) => out.push(item),
                PyIterReturn::StopIteration(_) => break,
            }
        }
        Ok(out)
    }

    // ── module-level functions ──────────────────────────────────────────────

    /// Resolve a class argument that's either a UClass proxy or a path string.
    pub(super) fn resolve_class(
        vm: &VirtualMachine,
        cls: &PyObject,
    ) -> PyResult<*const ue::UClass> {
        if let Some(uc) = cls.downcast_ref::<UClass>() {
            let obj = uc
                .weak()
                .get_even_if_pending_kill()
                .ok_or_else(|| value_err(vm, "stale UClass"))?;
            let cls = obj
                .cast::<ue::UClass>()
                .ok_or_else(|| type_err(vm, "UClass proxy no longer refers to a UClass"))?;
            Ok(cls as *const ue::UClass)
        } else if let Ok(path) = cls.try_to_value::<String>(vm) {
            let obj = ue::hash_tables()
                .find_object_by_path(&path)
                .ok_or_else(|| value_err(vm, format!("class not found: {path}")))?;
            let cls = obj
                .cast::<ue::UClass>()
                .ok_or_else(|| type_err(vm, format!("{path} is not a UClass")))?;
            Ok(cls as *const ue::UClass)
        } else {
            Err(type_err(vm, "class must be a UClass or a path string"))
        }
    }

    /// Args for `ue.objects_of_class`. `class_` (not `class`, which is a Python
    /// keyword, and not `cls`) matches `ue.objects` and `ue.trace`.
    /// `#[pyarg(any, …)]` makes each positional-or-keyword so the Python-side
    /// `include_subclasses=`/`max=` keywords work (RustPython's inline
    /// `#[pyfunction]` params are positional-only).
    #[derive(FromArgs)]
    struct ClassQueryArgs {
        #[pyarg(any, default = true)]
        include_subclasses: bool,
        #[pyarg(any, default)]
        max: Option<usize>,
    }

    /// Same, plus the class itself — `ue.objects_of_class` needs it as a real
    /// `#[pyarg]` so `class_=` works; `UClass.instances` gets it from `self`.
    #[derive(FromArgs)]
    struct ObjectsOfClassArgs {
        #[pyarg(any)]
        class_: PyObjectRef,
        #[pyarg(any, default = true)]
        include_subclasses: bool,
        #[pyarg(any, default)]
        max: Option<usize>,
    }

    /// Collect instances of `class_ptr` (optionally including subclasses),
    /// truncated to `args.max`, as a Python list. Each hit goes through
    /// `wrap_obj`, so looking up e.g. `/Script/CoreUObject.Class` yields real
    /// `UClass` proxies (with `props()` and friends) rather than bare `UObject`s.
    /// Shared by `ue.objects_of_class` and `UClass.instances`.
    fn instances_of(
        vm: &VirtualMachine,
        class_ptr: *const ue::UClass,
        args: &ClassQueryArgs,
    ) -> PyObjectRef {
        let mut hits = ue::hash_tables().objects_of_class(class_ptr, args.include_subclasses);
        if let Some(n) = args.max {
            hits.truncate(n);
        }
        let out: Vec<PyObjectRef> = hits.into_iter().map(|o| wrap_obj(vm, o)).collect();
        vm.ctx.new_list(out).into()
    }

    /// `ue.objects_of_class("/Script/Engine.Actor")` — fast class-indexed lookup.
    #[pyfunction]
    fn objects_of_class(args: ObjectsOfClassArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let class_ptr = resolve_class(vm, &args.class_)?;
        Ok(instances_of(
            vm,
            class_ptr,
            &ClassQueryArgs {
                include_subclasses: args.include_subclasses,
                max: args.max,
            },
        ))
    }

    /// Wrap a `UObjectBase` as a `UClass`, `UStruct`, or `UObject` proxy by
    /// runtime type — most-derived first, since `UClass` is a `UStruct`.
    pub(super) fn wrap_obj(vm: &VirtualMachine, obj: &ue::UObjectBase) -> PyObjectRef {
        let weak = ue::FWeakObjectPtr::new(obj);
        if obj.is::<ue::UClass>() {
            UClass::from_weak(weak).into_pyobject(vm)
        } else if obj.is::<ue::UStruct>() {
            UStruct::from_weak(weak).into_pyobject(vm)
        } else {
            UObject { weak }.into_pyobject(vm)
        }
    }

    /// `ue.find("/Game/Foo.Bar")` — lookup an already-loaded object by path.
    #[pyfunction]
    fn find(path: String, vm: &VirtualMachine) -> Option<PyObjectRef> {
        ue::hash_tables()
            .find_object_by_path(&path)
            .map(|o| wrap_obj(vm, o))
    }

    /// `ue.load("/Game/Foo.Bar")` — load the owning package on demand.
    #[pyfunction]
    fn load(path: String, vm: &VirtualMachine) -> Option<PyObjectRef> {
        ue::static_load_object(&path).map(|o| wrap_obj(vm, o))
    }

    /// Args for `ue.new_object` — all optional except the (positional) class.
    #[derive(FromArgs)]
    struct NewObjectArgs {
        #[pyarg(any, default)]
        outer: Option<PyObjectRef>,
        #[pyarg(any, default)]
        name: Option<String>,
        #[pyarg(any, default)]
        flags: Option<u32>,
    }

    /// `ue.new_object(cls, outer=None, name="", flags=0)` — construct a new
    /// UObject via the engine's `StaticConstructObject_Internal`.
    ///
    /// `cls` is a `UClass` proxy or a class path string. `outer` defaults to the
    /// transient package (null), `name` to an engine-assigned unique name, and
    /// `flags` to `RF_NoFlags` — pass `ue.EObjectFlags` values (e.g.
    /// `RF_Transient`). The new object is only weakly referenced by the returned
    /// proxy, so keep a reference (or root it) or it may be garbage-collected.
    #[pyfunction]
    fn new_object(
        cls: PyObjectRef,
        args: NewObjectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let class_ptr = resolve_class(vm, &cls)?;
        let outer: *const ue::UObjectBase = match &args.outer {
            None => std::ptr::null(),
            Some(o) if vm.is_none(o) => std::ptr::null(),
            Some(o) => {
                let u = o
                    .downcast_ref::<UObject>()
                    .ok_or_else(|| type_err(vm, "outer must be a UObject or None"))?;
                u.resolve(vm)? as *const ue::UObjectBase
            }
        };
        let name = match args.name.as_deref() {
            None | Some("") => ue::FName::none(),
            Some(s) => ue::FName::from_string(s, 1), // FNAME_Add
        };
        let flags = args.flags.unwrap_or(0);
        let obj = unsafe { ue::static_construct_object(class_ptr, outer, name, flags, 0) }
            .ok_or_else(|| runtime_err(vm, "new_object failed (engine returned null)"))?;
        Ok(wrap_obj(vm, obj))
    }

    /// Stream a chunk of output for an in-flight async run (see `interp`'s
    /// `_aio`) on stream `tag` (0 = stdout, 2 = stderr; see `server::TAG_*`).
    /// Internal: called by the async loop, not meant for direct use.
    #[pyfunction]
    fn _emit(token: u64, tag: u8, text: String) {
        crate::server::emit(token, tag, text);
    }

    /// Finish an in-flight async run with its final result text. Internal.
    #[pyfunction]
    fn _complete(token: u64, text: String) {
        crate::server::complete(token, text);
    }

    #[derive(FromArgs)]
    struct ObjectsArgs {
        #[pyarg(any, default)]
        class_: Option<String>,
        // `Option<usize>` rather than a bare `usize` so an explicit `max=None`
        // means "no cap" (matching `ClassQueryArgs`); omitting it still caps at
        // 1024, since this walks the whole GUObjectArray.
        #[pyarg(any, default = Some(1024))]
        max: Option<usize>,
    }

    /// `ue.objects(class="MyClass", max=100)` — iterate GUObjectArray.
    #[pyfunction]
    fn objects(args: ObjectsArgs, vm: &VirtualMachine) -> PyObjectRef {
        let class = args.class_;
        let max = args.max.unwrap_or(usize::MAX);
        let array = unsafe { &*ue::globals().guobject_array };
        let objs = array.objects();
        let mut out = Vec::new();
        for obj in objs.iter().flatten() {
            if let Some(filter) = &class {
                if obj.class().name_private().to_string() != *filter {
                    continue;
                }
            }
            out.push(wrap_obj(vm, obj));
            if out.len() >= max {
                break;
            }
        }
        vm.ctx.new_list(out).into()
    }

    // ──────────────────────────────────────────────────────────────────────
    // UObject
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "UObject", module = "ue")]
    #[derive(PyPayload)]
    pub struct UObject {
        pub(super) weak: ue::FWeakObjectPtr,
    }

    impl UObject {
        // No `from_ref` constructor: building a `UObject` proxy directly would
        // skip the most-derived wrapping, so every engine object entering Python
        // goes through `wrap_obj` instead.
        pub(super) fn resolve(&self, vm: &VirtualMachine) -> PyResult<&ue::UObjectBase> {
            self.weak
                .get_even_if_pending_kill()
                .ok_or_else(|| attr_err(vm, "stale UObject"))
        }
        fn addr(&self) -> Option<usize> {
            self.weak
                .get_even_if_pending_kill()
                .map(|o| o as *const _ as usize)
        }
    }

    #[pyclass(with(Constructor, GetAttr, SetAttr, Comparable, Hashable, Representable))]
    impl UObject {
        /// `obj._class` — introspection escapes start with `_` to dodge FProperty names.
        #[pygetset]
        fn _class(&self, vm: &VirtualMachine) -> PyResult<UClass> {
            let obj = self.resolve(vm)?;
            // &UClass coerces to &UObjectBase through the Deref chain.
            Ok(UClass::from_weak(ue::FWeakObjectPtr::new(obj.class())))
        }

        #[pygetset]
        fn _path(&self, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self.resolve(vm)?.object_path())
        }

        #[pygetset]
        fn _name(&self, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self.resolve(vm)?.name_private().to_string())
        }

        /// `obj._outer` — the object this one lives inside (its package, actor,
        /// or owning class), or `None` at the top of the chain. The counterpart
        /// of `ue.new_object(..., outer=...)`.
        #[pygetset]
        fn _outer(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let obj = self.resolve(vm)?;
            Ok(match unsafe { obj.outer_private().as_ref() } {
                Some(o) => wrap_obj(vm, o),
                None => vm.ctx.none(),
            })
        }

        /// Raw `EObjectFlags` bitmask — compare against `ue.EObjectFlags`.
        #[pygetset]
        fn _flags(&self, vm: &VirtualMachine) -> PyResult<u32> {
            Ok(self.resolve(vm)?.object_flags().bits())
        }

        /// Pin this object as a GC root (`UObject::AddToRoot`) so it survives
        /// garbage collection until `remove_from_root()`. Useful to keep a hooked
        /// function's owning class alive across level travel. Idempotent.
        #[pymethod]
        fn add_to_root(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.resolve(vm)?.add_to_root();
            Ok(())
        }

        /// Undo `add_to_root` (`UObject::RemoveFromRoot`), making the object
        /// collectable again. Idempotent.
        #[pymethod]
        fn remove_from_root(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.resolve(vm)?.remove_from_root();
            Ok(())
        }

        /// Whether the object is currently pinned in the GC root set.
        #[pymethod]
        fn is_rooted(&self, vm: &VirtualMachine) -> PyResult<bool> {
            Ok(self.resolve(vm)?.is_rooted())
        }

        /// If this UObject is a UEnum, return its `(entry_name, value)` pairs.
        #[pymethod]
        fn enum_entries(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let obj = self.resolve(vm)?;
            // Checked downcast (validates the class cast flags) replaces a manual
            // flag test + raw reinterpret; `None` if this object isn't a UEnum.
            let Some(en) = obj.cast::<ue::UEnum>() else {
                return Ok(vm.ctx.none());
            };
            Ok(to_list(
                vm,
                en.entries().iter().map(|e| (e.name.to_string(), e.value)),
            ))
        }

        /// If this UObject is a UEnum, the entry name for `value` (`None` if the
        /// value isn't one of the entries, or this isn't a UEnum). Saves a linear
        /// scan of `enum_entries()` for the lookup enum-valued property reads
        /// already do internally.
        #[pymethod]
        fn name_by_value(&self, value: i64, vm: &VirtualMachine) -> PyResult<Option<String>> {
            let obj = self.resolve(vm)?;
            let Some(en) = obj.cast::<ue::UEnum>() else {
                return Ok(None);
            };
            Ok(en.name_by_value(value).map(|n| n.to_string()))
        }

        /// The inverse: the value for an entry name. Accepts either the short
        /// name (`"VIS_MostAggressive"`) or the qualified one
        /// (`"EVisibilityAggressiveness::VIS_MostAggressive"`), matching what
        /// enum property reads produce and writes accept.
        #[pymethod]
        fn value_by_name(&self, name: String, vm: &VirtualMachine) -> PyResult<Option<i64>> {
            let obj = self.resolve(vm)?;
            let Some(en) = obj.cast::<ue::UEnum>() else {
                return Ok(None);
            };
            Ok(en.value_by_name_str(&name))
        }

        #[pymethod]
        fn __dir__(&self, vm: &VirtualMachine) -> PyObjectRef {
            // Property names *plus* the meta attributes — otherwise `dir()` (and
            // so the REPL's tab-completion) never surfaces `_class`/`_path`/…,
            // which are the first thing you reach for on an unknown object.
            let mut names: Vec<String> = ["_class", "_path", "_name", "_outer", "_flags"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
            names.extend(
                self.weak
                    .get_even_if_pending_kill()
                    .into_iter()
                    .flat_map(|o| o.class().iter_props().map(|f| f.name().to_string())),
            );
            to_list(vm, names)
        }
    }

    impl Representable for UObject {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.weak.get_even_if_pending_kill() {
                Some(o) => format!("<UObject {}>", o.path()),
                None => "<UObject stale>".into(),
            })
        }
    }

    impl Comparable for UObject {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                // A `UClass` shares `UObject`'s payload (it's a subclass), so this
                // downcast also covers class operands — identity compares uniformly.
                let a = zelf.addr();
                let b = other.downcast_ref::<UObject>().and_then(|u| u.addr());
                Ok(PyComparisonValue::Implemented(
                    matches!((a, b), (Some(x), Some(y)) if x == y),
                ))
            })
        }
    }

    impl Hashable for UObject {
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(zelf.addr().unwrap_or(0) as PyHash)
        }
    }

    impl GetAttr for UObject {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            if let Some(attr) = zelf.as_object().generic_getattr_opt(name, None, vm)? {
                return Ok(attr);
            }
            let name = name.to_string();
            let obj = zelf.resolve(vm)?;
            let fname = ue::FName::find(&name);
            for field in obj.class().iter_props() {
                if field.name() == fname {
                    let path = Path::root(zelf.weak).extend_field(fname);
                    return read_at_path(vm, path);
                }
            }
            // Functions on the class chain first, then ones reachable through an
            // implemented interface (native interface UFUNCTIONs live on the
            // interface UClass, not in the implementing class's own func map).
            if let Some(func) = obj
                .class()
                .find_function(fname)
                .or_else(|| obj.class().find_interface_function(fname))
            {
                return Ok(BoundMethod {
                    owner: zelf.weak,
                    function: ue::FWeakObjectPtr::new(func),
                    func_name: fname,
                }
                .into_pyobject(vm));
            }
            Err(attr_err(vm, name.to_string()))
        }
    }

    impl SetAttr for UObject {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    let path =
                        Path::root(zelf.weak).extend_field(ue::FName::find(&name.to_string()));
                    write_at_path(vm, path, v)
                }
                PySetterValue::Delete => Err(attr_err(vm, "cannot delete UObject attributes")),
            }
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // UStruct — a `UObject` specialized for reflection. Everything reflection
    // needs (properties, functions, the super chain) hangs off `UStruct`, not
    // `UClass`: a UScriptStruct and a UFunction are UStructs too, and both have
    // properties worth listing. So the reflection methods live here and `UClass`
    // (below) subclasses this, adding only the genuinely class-only extras.
    //
    // It's a genuine Python subclass of `UObject` (`isinstance(s, ue.UObject)`
    // holds) and reuses `UObject`'s payload, identity (`__eq__`/`__hash__`),
    // `_name`/`_path`/`_flags`, and attribute access. Sharing the payload means
    // a `UStruct`/`UClass` value still downcasts to `&Py<UObject>` on the Rust
    // side (`#[pyclass(base = ...)]` inherits `PAYLOAD_TYPE_ID`), so
    // object-argument coercion stays uniform down the whole chain.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "UStruct", module = "ue", base = UObject)]
    #[repr(transparent)]
    pub struct UStruct(UObject);

    impl UStruct {
        fn from_weak(weak: ue::FWeakObjectPtr) -> Self {
            UStruct(UObject { weak })
        }

        /// The live object reinterpreted as a `&UStruct`, or `None` if it's been
        /// GC'd. Callers decide how to surface staleness (empty list / `None`).
        /// Same caveat as `Function::resolve`: the ref is good for the current
        /// Python call only — don't hold it across engine re-entry (GC shares
        /// this thread and can otherwise collect the struct under it).
        fn as_struct(&self) -> Option<&ue::UStruct> {
            self.0
                .weak
                .get_even_if_pending_kill()
                .and_then(|o| o.cast::<ue::UStruct>())
        }
    }

    #[pyclass(with(Representable))]
    impl UStruct {
        /// List `(name, FProperty)` for each property in the super chain. The
        /// property object is the same one `prop_info` hands back — two words,
        /// so there's no reason to hand out a lossy `"StructProperty"` string
        /// here and make callers re-look-up by name for the element/target/
        /// struct type. `pt.kind` is that string if it's all you wanted.
        #[pymethod]
        fn props(&self, vm: &VirtualMachine) -> PyObjectRef {
            let Some(ustruct) = self.as_struct() else {
                return vm.ctx.new_list(vec![]).into();
            };
            let anchor = self.0.weak;
            let out: Vec<PyObjectRef> = ustruct
                .iter_props()
                .filter_map(|f| {
                    let prop = f.cast::<ue::FProperty>()?;
                    let pt = wrap_prop(vm, prop as *const _, anchor);
                    Some(vm.new_pyobj((f.name().to_string(), pt)))
                })
                .collect();
            vm.ctx.new_list(out).into()
        }

        /// List `(function_name, [param_name, ...])` for every UFunction on the chain.
        /// Empty for a UScriptStruct — script structs have properties, not methods.
        #[pymethod]
        fn functions(&self, vm: &VirtualMachine) -> PyObjectRef {
            let Some(ustruct) = self.as_struct() else {
                return vm.ctx.new_list(vec![]).into();
            };
            let out: Vec<PyObjectRef> = ustruct
                .iter_functions()
                .map(|func| {
                    let parms: Vec<String> = func.params().map(|p| p.name().to_string()).collect();
                    let name = func.name_private().to_string();
                    vm.new_pyobj((name, to_list(vm, parms)))
                })
                .collect();
            vm.ctx.new_list(out).into()
        }

        /// Structured `Function` object for every UFunction on this super chain.
        #[pymethod]
        fn functions_detailed(&self, vm: &VirtualMachine) -> PyObjectRef {
            let Some(ustruct) = self.as_struct() else {
                return vm.ctx.new_list(vec![]).into();
            };
            let out: Vec<PyRef<Function>> = ustruct
                .iter_functions()
                .map(|func| function_proxy(vm, func))
                .collect();
            to_list(vm, out)
        }

        /// Structured `Function` object for one UFunction by name. Walks the
        /// super chain.
        #[pymethod]
        fn function_info(&self, name: String, vm: &VirtualMachine) -> Option<PyRef<Function>> {
            let ustruct = self.as_struct()?;
            let fname = ue::FName::find(&name);
            if fname.comparison_index.value == 0 {
                return None;
            }
            let func = ustruct.find_function(fname)?;
            Some(function_proxy(vm, func))
        }

        /// The `FProperty` (as its most-derived subclass) for one property by
        /// name; walks the super chain. Same object `props()` yields.
        #[pymethod]
        fn prop_info(&self, name: String, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let ustruct = self.as_struct()?;
            let fname = ue::FName::find(&name);
            if fname.comparison_index.value == 0 {
                return None;
            }
            for field in ustruct.iter_props() {
                if field.name() == fname {
                    if let Some(prop) = field.cast::<ue::FProperty>() {
                        return Some(wrap_prop(vm, prop as *const _, self.0.weak));
                    }
                }
            }
            None
        }

        /// Walk the super chain, root last. Each entry is wrapped by its own
        /// runtime type, so a class's parents come back as `UClass` and a script
        /// struct's as `UStruct` — no filtering, the chain is reported whole.
        #[pymethod]
        fn parents(&self, vm: &VirtualMachine) -> PyObjectRef {
            let Some(ustruct) = self.as_struct() else {
                return vm.ctx.new_list(vec![]).into();
            };
            // `iter_super` yields self first; `skip(1)` gives ancestors only.
            let out: Vec<PyObjectRef> = ustruct
                .iter_super()
                .skip(1)
                .map(|s| wrap_obj(vm, s))
                .collect();
            vm.ctx.new_list(out).into()
        }
    }

    impl Representable for UStruct {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.0.weak.get_even_if_pending_kill() {
                Some(o) => format!("<UStruct {}>", o.object_path()),
                None => "<UStruct stale>".into(),
            })
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // UClass — a `UStruct` plus the reflection that only a class has: the
    // interface table and the class-indexed instance lookup. Everything else
    // (props, functions, prop_info, function_info, parents) is inherited.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "UClass", module = "ue", base = UStruct)]
    #[repr(transparent)]
    pub struct UClass(UStruct);

    impl UClass {
        fn from_weak(weak: ue::FWeakObjectPtr) -> Self {
            UClass(UStruct(UObject { weak }))
        }

        /// The underlying weak pointer, past the `UClass(UStruct(UObject))`
        /// newtype nesting the payload sharing requires. Returned by reference:
        /// `get_even_if_pending_kill` hands back a borrow of the pointer, so a
        /// by-value accessor would only ever yield a temporary to borrow from.
        fn weak(&self) -> &ue::FWeakObjectPtr {
            &self.0.0.weak
        }

        /// The live object reinterpreted as a `&UClass`, or `None` if it's been
        /// GC'd. Same borrow caveat as [`UStruct::as_struct`].
        fn as_class(&self) -> Option<&ue::UClass> {
            self.weak()
                .get_even_if_pending_kill()
                .and_then(|o| o.cast::<ue::UClass>())
        }
    }

    #[pyclass(with(Representable))]
    impl UClass {
        /// `(interface_class, vtable_offset, implemented_by_k2)` for each interface
        /// this class implements directly (UE's `UClass::Interfaces`).
        #[pymethod]
        fn interfaces(&self, vm: &VirtualMachine) -> PyObjectRef {
            let Some(class) = self.as_class() else {
                return vm.ctx.new_list(vec![]).into();
            };
            let out: Vec<PyObjectRef> = class
                .interfaces()
                .iter()
                .filter_map(|entry| {
                    let ic = unsafe { entry.class.as_ref() }?;
                    let cls = UClass::from_weak(ue::FWeakObjectPtr::new(ic));
                    Some(vm.new_pyobj((cls, entry.pointer_offset, entry.b_implemented_by_k2)))
                })
                .collect();
            vm.ctx.new_list(out).into()
        }

        /// `cls.instances()` — equivalent to `ue.objects_of_class(cls)`.
        #[pymethod]
        fn instances(&self, args: ClassQueryArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let cls = self
                .as_class()
                .ok_or_else(|| type_err(vm, "not a live UClass"))?;
            Ok(instances_of(vm, cls as *const ue::UClass, &args))
        }
    }

    // `__eq__`/`__hash__` are inherited from `UObject` (identity by address).
    impl Representable for UClass {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.weak().get_even_if_pending_kill() {
                Some(o) => format!("<UClass {}>", o.object_path()),
                None => "<UClass stale>".into(),
            })
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Reflection rendering helpers — the single string/role/flag renderers
    // behind the structured `FProperty` / `Param` / `Function` types.
    // ──────────────────────────────────────────────────────────────────────

    /// Format an FProperty's type as a human-readable string.
    fn type_name(prop: &ue::FProperty) -> String {
        let base = prop.class().name.to_string();

        if let Some(opb) = prop.cast_field::<ue::FObjectPropertyBase>() {
            if let Some(cls) = opb.property_class() {
                return format!("{base}<{}>", cls.name_private());
            }
            return base;
        }
        if let Some(sp) = prop.cast_field::<ue::FStructProperty>() {
            let st = sp.get_struct();
            return format!("{base}<{}>", st.name_private());
        }
        if let Some(ap) = prop.cast_field::<ue::FArrayProperty>() {
            return format!("{base}<{}>", type_name(ap.inner()));
        }
        if let Some(mp) = prop.cast_field::<ue::FMapProperty>() {
            return format!(
                "{base}<{}, {}>",
                type_name(mp.key_property()),
                type_name(mp.value_property())
            );
        }
        if let Some(sp) = prop.cast_field::<ue::FSetProperty>() {
            return format!("{base}<{}>", type_name(sp.element_property()));
        }
        if let Some(ep) = prop.cast_field::<ue::FEnumProperty>() {
            if let Some(en) = ep.enum_ptr() {
                return format!("{base}<{}>", en.name_private());
            }
            return base;
        }
        if let Some(bp) = prop.cast_field::<ue::FByteProperty>() {
            if let Some(en) = bp.enum_ptr() {
                return format!("{base}<{}>", en.name_private());
            }
            return base;
        }
        base
    }

    fn parm_role(p: &ue::FProperty) -> &'static str {
        let f = p.property_flags();
        use ue::EPropertyFlags as P;
        if f.contains(P::CPF_ReturnParm) {
            "ret"
        } else if f.contains(P::CPF_OutParm) && !f.contains(P::CPF_ConstParm) {
            if f.contains(P::CPF_ReferenceParm) {
                "out_ref"
            } else {
                "out"
            }
        } else if f.contains(P::CPF_ConstParm) {
            "in_const"
        } else {
            "in"
        }
    }

    /// `(entry_name, bits)` for every `EFunctionFlags` bit worth naming. One
    /// table drives both `flag_names()`/signature rendering and the
    /// `ue.EFunctionFlags` IntFlag, so they can't drift apart.
    fn function_flag_table() -> Vec<(&'static str, u64)> {
        use ue::EFunctionFlags as F;
        [
            (F::FUNC_Final, "Final"),
            (F::FUNC_Native, "Native"),
            (F::FUNC_Static, "Static"),
            (F::FUNC_Public, "Public"),
            (F::FUNC_Private, "Private"),
            (F::FUNC_Protected, "Protected"),
            (F::FUNC_Event, "Event"),
            (F::FUNC_BlueprintCallable, "BlueprintCallable"),
            (F::FUNC_BlueprintEvent, "BlueprintEvent"),
            (F::FUNC_BlueprintPure, "BlueprintPure"),
            (F::FUNC_BlueprintAuthorityOnly, "BlueprintAuthorityOnly"),
            (F::FUNC_BlueprintCosmetic, "BlueprintCosmetic"),
            (F::FUNC_Exec, "Exec"),
            (F::FUNC_Net, "Net"),
            (F::FUNC_NetReliable, "NetReliable"),
            (F::FUNC_NetRequest, "NetRequest"),
            (F::FUNC_NetResponse, "NetResponse"),
            (F::FUNC_NetMulticast, "NetMulticast"),
            (F::FUNC_NetServer, "NetServer"),
            (F::FUNC_NetClient, "NetClient"),
            (F::FUNC_NetValidate, "NetValidate"),
            (F::FUNC_HasOutParms, "HasOutParms"),
            (F::FUNC_HasDefaults, "HasDefaults"),
            (F::FUNC_Const, "Const"),
            (F::FUNC_Delegate, "Delegate"),
            (F::FUNC_MulticastDelegate, "MulticastDelegate"),
            (F::FUNC_UbergraphFunction, "UbergraphFunction"),
        ]
        .iter()
        .map(|(bit, name)| (*name, bit.bits() as u64))
        .collect()
    }

    /// The same for `EPropertyFlags`.
    fn property_flag_table() -> Vec<(&'static str, u64)> {
        use ue::EPropertyFlags as P;
        [
            (P::CPF_Edit, "Edit"),
            (P::CPF_ConstParm, "ConstParm"),
            (P::CPF_BlueprintVisible, "BlueprintVisible"),
            (P::CPF_ExportObject, "ExportObject"),
            (P::CPF_BlueprintReadOnly, "BlueprintReadOnly"),
            (P::CPF_Net, "Net"),
            (P::CPF_EditFixedSize, "EditFixedSize"),
            (P::CPF_Parm, "Parm"),
            (P::CPF_OutParm, "OutParm"),
            (P::CPF_ZeroConstructor, "ZeroConstructor"),
            (P::CPF_ReturnParm, "ReturnParm"),
            (P::CPF_DisableEditOnTemplate, "DisableEditOnTemplate"),
            (P::CPF_NonNullable, "NonNullable"),
            (P::CPF_Transient, "Transient"),
            (P::CPF_Config, "Config"),
            (P::CPF_RequiredParm, "RequiredParm"),
            (P::CPF_DisableEditOnInstance, "DisableEditOnInstance"),
            (P::CPF_EditConst, "EditConst"),
            (P::CPF_GlobalConfig, "GlobalConfig"),
            (P::CPF_InstancedReference, "InstancedReference"),
            (P::CPF_DuplicateTransient, "DuplicateTransient"),
            (P::CPF_SaveGame, "SaveGame"),
            (P::CPF_NoClear, "NoClear"),
            (P::CPF_ReferenceParm, "ReferenceParm"),
            (P::CPF_BlueprintAssignable, "BlueprintAssignable"),
            (P::CPF_Deprecated, "Deprecated"),
            (P::CPF_IsPlainOldData, "IsPlainOldData"),
            (P::CPF_RepSkip, "RepSkip"),
            (P::CPF_RepNotify, "RepNotify"),
            (P::CPF_Interp, "Interp"),
            (P::CPF_NonTransactional, "NonTransactional"),
            (P::CPF_EditorOnly, "EditorOnly"),
            (P::CPF_NoDestructor, "NoDestructor"),
            (P::CPF_AutoWeak, "AutoWeak"),
            (
                P::CPF_ContainsInstancedReference,
                "ContainsInstancedReference",
            ),
            (P::CPF_AssetRegistrySearchable, "AssetRegistrySearchable"),
            (P::CPF_SimpleDisplay, "SimpleDisplay"),
            (P::CPF_AdvancedDisplay, "AdvancedDisplay"),
            (P::CPF_Protected, "Protected"),
            (P::CPF_BlueprintCallable, "BlueprintCallable"),
            (P::CPF_BlueprintAuthorityOnly, "BlueprintAuthorityOnly"),
            (P::CPF_TextExportTransient, "TextExportTransient"),
            (P::CPF_NonPIEDuplicateTransient, "NonPIEDuplicateTransient"),
            (P::CPF_ExposeOnSpawn, "ExposeOnSpawn"),
            (P::CPF_PersistentInstance, "PersistentInstance"),
            (P::CPF_UObjectWrapper, "UObjectWrapper"),
            (P::CPF_HasGetValueTypeHash, "HasGetValueTypeHash"),
            (
                P::CPF_NativeAccessSpecifierPublic,
                "NativeAccessSpecifierPublic",
            ),
            (
                P::CPF_NativeAccessSpecifierProtected,
                "NativeAccessSpecifierProtected",
            ),
            (
                P::CPF_NativeAccessSpecifierPrivate,
                "NativeAccessSpecifierPrivate",
            ),
            (P::CPF_SkipSerialization, "SkipSerialization"),
        ]
        .iter()
        .map(|(bit, name)| (*name, bit.bits()))
        .collect()
    }

    /// Names of the set bits in `bits`, in table order.
    fn set_flag_names(table: &[(&'static str, u64)], bits: u64) -> Vec<String> {
        table
            .iter()
            .filter(|(_, bit)| bits & bit == *bit)
            .map(|(name, _)| (*name).to_owned())
            .collect()
    }

    fn function_flags_str(flags: ue::EFunctionFlags) -> String {
        set_flag_names(&function_flag_table(), flags.bits() as u64).join(" ")
    }

    /// Render a UFunction's full signature as a readable string, e.g.
    /// `Foo(in int32 A, out float B) -> bool [Final Native]`. This is the single
    /// string renderer behind `Function`'s `repr` / `signature`.
    fn render_signature(func: &ue::UFunction) -> String {
        let name = func.name_private().to_string();
        let parms: Vec<String> = func
            .params()
            .map(|p| format!("{} {} {}", parm_role(p), type_name(p), p.name()))
            .collect();
        let mut sig = format!("{name}({})", parms.join(", "));
        if let Some(rp) = func.return_property() {
            sig.push_str(&format!(" -> {}", type_name(rp)));
        }
        let flags = function_flags_str(func.function_flags());
        if !flags.is_empty() {
            sig.push_str(&format!(" [{flags}]"));
        }
        sig
    }

    // ──────────────────────────────────────────────────────────────────────
    // FProperty hierarchy — a 1:1 mirror of UE's C++ FProperty tree.
    //
    // `FProperty` carries what every property has (kind/flags/size) and each
    // subclass adds exactly the accessors its C++ counterpart has members for:
    // `target_class` lives on `FObjectPropertyBase`, `inner` on `FArrayProperty`,
    // `struct_type` on `FStructProperty`, and so on. So `dir(pt)` tells you what
    // this property actually is, and asking a property something it can't answer
    // is an AttributeError rather than a `None` you have to distinguish from a
    // genuine null.
    //
    // Every class shares `FProperty`'s payload — RustPython inherits
    // `PAYLOAD_TYPE_ID` down a `base = ...` chain — so the whole tree downcasts
    // to `&Py<PyFProperty>` on the Rust side, and `Deref` walks the newtype
    // nesting back to the fields. Python idents are the C++ names (`FIntProperty`);
    // the Rust ones are `Py`-prefixed to stay clear of libue's `ue::FIntProperty`.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "FProperty", module = "ue")]
    #[derive(PyPayload)]
    pub struct PyFProperty {
        prop: *const ue::FProperty,
        anchor: ue::FWeakObjectPtr,
    }

    impl PyFProperty {
        fn wrap(prop: *const ue::FProperty, anchor: ue::FWeakObjectPtr) -> Self {
            Self { prop, anchor }
        }

        /// The base payload. `Deref` reaches it for field/method access, but a
        /// generic `Deref<Target = PyFProperty>` bound only matches the classes
        /// one level down, so deep subclasses forward through this instead.
        fn base(&self) -> &PyFProperty {
            self
        }

        /// FFields aren't UObjects and have no weak-pointer machinery of their
        /// own, so liveness is borrowed from the UStruct that owns the chain:
        /// while that's alive the property can't have been freed.
        fn resolve(&self, vm: &VirtualMachine) -> PyResult<&ue::FProperty> {
            if self.anchor.get_even_if_pending_kill().is_none() {
                return Err(attr_err(vm, "owning UStruct is stale"));
            }
            Ok(unsafe { &*self.prop })
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl PyFProperty {
        /// The FField class name, e.g. `"StructProperty"` — the Python class
        /// name without the `F`.
        #[pygetset]
        fn kind(&self, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self.resolve(vm)?.class().name.to_string())
        }
        #[pygetset]
        fn cast_flags(&self, vm: &VirtualMachine) -> PyResult<u64> {
            Ok(self.resolve(vm)?.class().cast_flags().bits())
        }
        /// Raw `EPropertyFlags` bitmask — compare against `ue.EPropertyFlags`.
        #[pygetset]
        fn property_flags(&self, vm: &VirtualMachine) -> PyResult<u64> {
            Ok(self.resolve(vm)?.property_flags().bits())
        }

        /// The set flags by name, e.g. `["Parm", "OutParm", "ReferenceParm"]` —
        /// the property-side counterpart of `Function.flag_names()`.
        #[pymethod]
        fn flag_names(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let bits = self.resolve(vm)?.property_flags().bits();
            Ok(to_list(vm, set_flag_names(&property_flag_table(), bits)))
        }
        #[pygetset]
        fn element_size(&self, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(self.resolve(vm)?.element_size())
        }
        #[pygetset]
        fn offset(&self, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(self.resolve(vm)?.offset())
        }
    }

    impl Representable for PyFProperty {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(prop_repr(zelf, vm))
        }
    }

    /// Shared `repr` for the whole tree: the rendered type, e.g.
    /// `<StructProperty<RChatLogItem>>` or `<ArrayProperty<ObjectProperty<Actor>>>`.
    fn prop_repr(zelf: &PyFProperty, vm: &VirtualMachine) -> String {
        match zelf.resolve(vm) {
            Ok(p) => format!("<{}>", type_name(p)),
            Err(_) => "<FProperty stale>".to_string(),
        }
    }

    /// Per-subclass boilerplate: the payload-forwarding constructor, the `Deref`
    /// that makes `self.resolve(..)`/`self.anchor` reach the base through the
    /// newtype nesting, and the inherited `repr`/`Debug`.
    macro_rules! prop_class {
        ($rust:ident, $base:ident, $py:literal) => {
            impl $rust {
                fn wrap(prop: *const ue::FProperty, anchor: ue::FWeakObjectPtr) -> Self {
                    $rust($base::wrap(prop, anchor))
                }
                fn base(&self) -> &PyFProperty {
                    self.0.base()
                }
            }
            impl std::ops::Deref for $rust {
                type Target = $base;
                fn deref(&self) -> &$base {
                    &self.0
                }
            }
            impl Representable for $rust {
                fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
                    Ok(prop_repr(zelf.base(), vm))
                }
            }
            impl std::fmt::Debug for $rust {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str($py)
                }
            }
        };
    }

    #[pyattr]
    #[pyclass(name = "FNumericProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFNumericProperty(PyFProperty);
    prop_class!(PyFNumericProperty, PyFProperty, "FNumericProperty");
    #[pyclass(with(Representable))]
    impl PyFNumericProperty {}
    #[pyattr]
    #[pyclass(name = "FByteProperty", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFByteProperty(PyFNumericProperty);
    prop_class!(PyFByteProperty, PyFNumericProperty, "FByteProperty");
    #[pyclass(with(Representable))]
    impl PyFByteProperty {
        /// `Enum` — set for the legacy `TEnumAsByte<E>` spelling, `None` for a
        /// plain `uint8`. (A modern `enum class` is an `FEnumProperty` instead.)
        #[pymethod]
        fn enum_obj(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FByteProperty>()
                .and_then(|p| p.enum_ptr())
                .map(|e| wrap_obj(vm, e)))
        }
    }
    #[pyattr]
    #[pyclass(name = "FInt8Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFInt8Property(PyFNumericProperty);
    prop_class!(PyFInt8Property, PyFNumericProperty, "FInt8Property");
    #[pyclass(with(Representable))]
    impl PyFInt8Property {}
    #[pyattr]
    #[pyclass(name = "FInt16Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFInt16Property(PyFNumericProperty);
    prop_class!(PyFInt16Property, PyFNumericProperty, "FInt16Property");
    #[pyclass(with(Representable))]
    impl PyFInt16Property {}
    #[pyattr]
    #[pyclass(name = "FIntProperty", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFIntProperty(PyFNumericProperty);
    prop_class!(PyFIntProperty, PyFNumericProperty, "FIntProperty");
    #[pyclass(with(Representable))]
    impl PyFIntProperty {}
    #[pyattr]
    #[pyclass(name = "FInt64Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFInt64Property(PyFNumericProperty);
    prop_class!(PyFInt64Property, PyFNumericProperty, "FInt64Property");
    #[pyclass(with(Representable))]
    impl PyFInt64Property {}
    #[pyattr]
    #[pyclass(name = "FUInt16Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFUInt16Property(PyFNumericProperty);
    prop_class!(PyFUInt16Property, PyFNumericProperty, "FUInt16Property");
    #[pyclass(with(Representable))]
    impl PyFUInt16Property {}
    #[pyattr]
    #[pyclass(name = "FUInt32Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFUInt32Property(PyFNumericProperty);
    prop_class!(PyFUInt32Property, PyFNumericProperty, "FUInt32Property");
    #[pyclass(with(Representable))]
    impl PyFUInt32Property {}
    #[pyattr]
    #[pyclass(name = "FUInt64Property", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFUInt64Property(PyFNumericProperty);
    prop_class!(PyFUInt64Property, PyFNumericProperty, "FUInt64Property");
    #[pyclass(with(Representable))]
    impl PyFUInt64Property {}
    #[pyattr]
    #[pyclass(name = "FFloatProperty", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFFloatProperty(PyFNumericProperty);
    prop_class!(PyFFloatProperty, PyFNumericProperty, "FFloatProperty");
    #[pyclass(with(Representable))]
    impl PyFFloatProperty {}
    #[pyattr]
    #[pyclass(name = "FDoubleProperty", module = "ue", base = PyFNumericProperty)]
    #[repr(transparent)]
    pub struct PyFDoubleProperty(PyFNumericProperty);
    prop_class!(PyFDoubleProperty, PyFNumericProperty, "FDoubleProperty");
    #[pyclass(with(Representable))]
    impl PyFDoubleProperty {}
    #[pyattr]
    #[pyclass(name = "FBoolProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFBoolProperty(PyFProperty);
    prop_class!(PyFBoolProperty, PyFProperty, "FBoolProperty");
    #[pyclass(with(Representable))]
    impl PyFBoolProperty {
        /// `FBoolProperty`'s bitfield descriptor. A `bool` UPROPERTY is a whole
        /// byte with `byte_mask == field_mask == 0xFF`; a `uint8 x : 1` bitfield
        /// packs several properties into one byte and the masks pick out this
        /// one.
        #[pygetset]
        fn field_size(&self, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self.as_bool(vm)?.field_size())
        }
        #[pygetset]
        fn byte_offset(&self, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self.as_bool(vm)?.byte_offset())
        }
        #[pygetset]
        fn byte_mask(&self, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self.as_bool(vm)?.byte_mask())
        }
        #[pygetset]
        fn field_mask(&self, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self.as_bool(vm)?.field_mask())
        }
    }
    impl PyFBoolProperty {
        fn as_bool(&self, vm: &VirtualMachine) -> PyResult<&ue::FBoolProperty> {
            self.resolve(vm)?
                .cast_field::<ue::FBoolProperty>()
                .ok_or_else(|| type_err(vm, "not an FBoolProperty"))
        }
    }
    #[pyattr]
    #[pyclass(name = "FObjectPropertyBase", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFObjectPropertyBase(PyFProperty);
    prop_class!(PyFObjectPropertyBase, PyFProperty, "FObjectPropertyBase");
    #[pyclass(with(Representable))]
    impl PyFObjectPropertyBase {
        /// `PropertyClass` — the UClass a value of this property must be an
        /// instance of (or, for a class property, a subclass of). `None` only if
        /// the engine left it unset.
        #[pymethod]
        fn target_class(&self, vm: &VirtualMachine) -> PyResult<Option<UClass>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FObjectPropertyBase>()
                .and_then(|p| p.property_class())
                .map(|c| UClass::from_weak(ue::FWeakObjectPtr::new(c))))
        }
    }
    #[pyattr]
    #[pyclass(name = "FObjectProperty", module = "ue", base = PyFObjectPropertyBase)]
    #[repr(transparent)]
    pub struct PyFObjectProperty(PyFObjectPropertyBase);
    prop_class!(PyFObjectProperty, PyFObjectPropertyBase, "FObjectProperty");
    #[pyclass(with(Representable))]
    impl PyFObjectProperty {}
    #[pyattr]
    #[pyclass(name = "FClassProperty", module = "ue", base = PyFObjectProperty)]
    #[repr(transparent)]
    pub struct PyFClassProperty(PyFObjectProperty);
    prop_class!(PyFClassProperty, PyFObjectProperty, "FClassProperty");
    #[pyclass(with(Representable))]
    impl PyFClassProperty {
        /// `MetaClass` — for `TSubclassOf<T>`, the `T` the stored UClass must
        /// derive from. `target_class()` is `UClass` itself here.
        #[pymethod]
        fn meta_class(&self, vm: &VirtualMachine) -> PyResult<Option<UClass>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FClassProperty>()
                .and_then(|p| p.meta_class())
                .map(|c| UClass::from_weak(ue::FWeakObjectPtr::new(c))))
        }
    }
    #[pyattr]
    #[pyclass(name = "FWeakObjectProperty", module = "ue", base = PyFObjectPropertyBase)]
    #[repr(transparent)]
    pub struct PyFWeakObjectProperty(PyFObjectPropertyBase);
    prop_class!(
        PyFWeakObjectProperty,
        PyFObjectPropertyBase,
        "FWeakObjectProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFWeakObjectProperty {}
    #[pyattr]
    #[pyclass(name = "FLazyObjectProperty", module = "ue", base = PyFObjectPropertyBase)]
    #[repr(transparent)]
    pub struct PyFLazyObjectProperty(PyFObjectPropertyBase);
    prop_class!(
        PyFLazyObjectProperty,
        PyFObjectPropertyBase,
        "FLazyObjectProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFLazyObjectProperty {}
    #[pyattr]
    #[pyclass(name = "FSoftObjectProperty", module = "ue", base = PyFObjectPropertyBase)]
    #[repr(transparent)]
    pub struct PyFSoftObjectProperty(PyFObjectPropertyBase);
    prop_class!(
        PyFSoftObjectProperty,
        PyFObjectPropertyBase,
        "FSoftObjectProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFSoftObjectProperty {}
    #[pyattr]
    #[pyclass(name = "FSoftClassProperty", module = "ue", base = PyFSoftObjectProperty)]
    #[repr(transparent)]
    pub struct PyFSoftClassProperty(PyFSoftObjectProperty);
    prop_class!(
        PyFSoftClassProperty,
        PyFSoftObjectProperty,
        "FSoftClassProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFSoftClassProperty {
        /// `MetaClass` — the class a `TSoftClassPtr<T>` must resolve to a
        /// subclass of.
        #[pymethod]
        fn meta_class(&self, vm: &VirtualMachine) -> PyResult<Option<UClass>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FSoftClassProperty>()
                .and_then(|p| p.meta_class())
                .map(|c| UClass::from_weak(ue::FWeakObjectPtr::new(c))))
        }
    }
    #[pyattr]
    #[pyclass(name = "FInterfaceProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFInterfaceProperty(PyFProperty);
    prop_class!(PyFInterfaceProperty, PyFProperty, "FInterfaceProperty");
    #[pyclass(with(Representable))]
    impl PyFInterfaceProperty {
        /// `InterfaceClass` — the interface UClass a `TScriptInterface` slot
        /// requires.
        #[pymethod]
        fn interface_class(&self, vm: &VirtualMachine) -> PyResult<Option<UClass>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FInterfaceProperty>()
                .and_then(|p| p.interface_class())
                .map(|c| UClass::from_weak(ue::FWeakObjectPtr::new(c))))
        }
    }
    #[pyattr]
    #[pyclass(name = "FNameProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFNameProperty(PyFProperty);
    prop_class!(PyFNameProperty, PyFProperty, "FNameProperty");
    #[pyclass(with(Representable))]
    impl PyFNameProperty {}
    #[pyattr]
    #[pyclass(name = "FStrProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFStrProperty(PyFProperty);
    prop_class!(PyFStrProperty, PyFProperty, "FStrProperty");
    #[pyclass(with(Representable))]
    impl PyFStrProperty {}
    #[pyattr]
    #[pyclass(name = "FTextProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFTextProperty(PyFProperty);
    prop_class!(PyFTextProperty, PyFProperty, "FTextProperty");
    #[pyclass(with(Representable))]
    impl PyFTextProperty {}
    #[pyattr]
    #[pyclass(name = "FArrayProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFArrayProperty(PyFProperty);
    prop_class!(PyFArrayProperty, PyFProperty, "FArrayProperty");
    #[pyclass(with(Representable))]
    impl PyFArrayProperty {
        /// `Inner` — the element property.
        #[pymethod]
        fn inner(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let ap = prop
                .cast_field::<ue::FArrayProperty>()
                .ok_or_else(|| type_err(vm, "not an FArrayProperty"))?;
            Ok(wrap_prop(vm, ap.inner() as *const _, self.anchor))
        }
    }
    #[pyattr]
    #[pyclass(name = "FSetProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFSetProperty(PyFProperty);
    prop_class!(PyFSetProperty, PyFProperty, "FSetProperty");
    #[pyclass(with(Representable))]
    impl PyFSetProperty {
        /// `ElementProp` — the element property. Named `inner` to match
        /// `FArrayProperty`, since both answer "what's in it".
        #[pymethod]
        fn inner(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let sp = prop
                .cast_field::<ue::FSetProperty>()
                .ok_or_else(|| type_err(vm, "not an FSetProperty"))?;
            Ok(wrap_prop(
                vm,
                sp.element_property() as *const _,
                self.anchor,
            ))
        }
    }
    #[pyattr]
    #[pyclass(name = "FMapProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFMapProperty(PyFProperty);
    prop_class!(PyFMapProperty, PyFProperty, "FMapProperty");
    #[pyclass(with(Representable))]
    impl PyFMapProperty {
        /// `KeyProp` — the key property.
        #[pymethod]
        fn key(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let mp = prop
                .cast_field::<ue::FMapProperty>()
                .ok_or_else(|| type_err(vm, "not an FMapProperty"))?;
            Ok(wrap_prop(vm, mp.key_property() as *const _, self.anchor))
        }

        /// `ValueProp` — the value property.
        #[pymethod]
        fn value(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let mp = prop
                .cast_field::<ue::FMapProperty>()
                .ok_or_else(|| type_err(vm, "not an FMapProperty"))?;
            Ok(wrap_prop(vm, mp.value_property() as *const _, self.anchor))
        }
    }
    #[pyattr]
    #[pyclass(name = "FStructProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFStructProperty(PyFProperty);
    prop_class!(PyFStructProperty, PyFProperty, "FStructProperty");
    #[pyclass(with(Representable))]
    impl PyFStructProperty {
        /// `Struct` — the UScriptStruct describing the value. Comes back as a
        /// `UStruct` proxy, so `pt.struct_type().props()` lists its fields.
        #[pymethod]
        fn struct_type(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let sp = prop
                .cast_field::<ue::FStructProperty>()
                .ok_or_else(|| type_err(vm, "not an FStructProperty"))?;
            Ok(wrap_obj(vm, sp.get_struct()))
        }
    }
    #[pyattr]
    #[pyclass(name = "FDelegateProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFDelegateProperty(PyFProperty);
    prop_class!(PyFDelegateProperty, PyFProperty, "FDelegateProperty");
    #[pyclass(with(Representable))]
    impl PyFDelegateProperty {
        /// `SignatureFunction` — the UFunction whose params define what the
        /// delegate is called with.
        #[pymethod]
        fn signature_function(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FDelegateProperty>()
                .and_then(|p| p.signature_function())
                .map(|f| wrap_obj(vm, f)))
        }
    }
    #[pyattr]
    #[pyclass(name = "FMulticastDelegateProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFMulticastDelegateProperty(PyFProperty);
    prop_class!(
        PyFMulticastDelegateProperty,
        PyFProperty,
        "FMulticastDelegateProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFMulticastDelegateProperty {
        /// `SignatureFunction` — the UFunction whose params define what each
        /// binding is called with.
        #[pymethod]
        fn signature_function(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FMulticastDelegateProperty>()
                .and_then(|p| p.signature_function())
                .map(|f| wrap_obj(vm, f)))
        }
    }
    #[pyattr]
    #[pyclass(name = "FMulticastInlineDelegateProperty", module = "ue", base = PyFMulticastDelegateProperty)]
    #[repr(transparent)]
    pub struct PyFMulticastInlineDelegateProperty(PyFMulticastDelegateProperty);
    prop_class!(
        PyFMulticastInlineDelegateProperty,
        PyFMulticastDelegateProperty,
        "FMulticastInlineDelegateProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFMulticastInlineDelegateProperty {}
    #[pyattr]
    #[pyclass(name = "FMulticastSparseDelegateProperty", module = "ue", base = PyFMulticastDelegateProperty)]
    #[repr(transparent)]
    pub struct PyFMulticastSparseDelegateProperty(PyFMulticastDelegateProperty);
    prop_class!(
        PyFMulticastSparseDelegateProperty,
        PyFMulticastDelegateProperty,
        "FMulticastSparseDelegateProperty"
    );
    #[pyclass(with(Representable))]
    impl PyFMulticastSparseDelegateProperty {}
    #[pyattr]
    #[pyclass(name = "FEnumProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFEnumProperty(PyFProperty);
    prop_class!(PyFEnumProperty, PyFProperty, "FEnumProperty");
    #[pyclass(with(Representable))]
    impl PyFEnumProperty {
        /// `Enum` — the UEnum this property's values name.
        #[pymethod]
        fn enum_obj(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let prop = self.resolve(vm)?;
            Ok(prop
                .cast_field::<ue::FEnumProperty>()
                .and_then(|p| p.enum_ptr())
                .map(|e| wrap_obj(vm, e)))
        }

        /// `UnderlyingProp` — the numeric property the enum is actually stored
        /// in, which is what fixes its width and signedness.
        #[pymethod]
        fn underlying(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let prop = self.resolve(vm)?;
            let ep = prop
                .cast_field::<ue::FEnumProperty>()
                .ok_or_else(|| type_err(vm, "not an FEnumProperty"))?;
            Ok(wrap_prop(vm, ep.underlying_prop() as *const _, self.anchor))
        }
    }
    #[pyattr]
    #[pyclass(name = "FFieldPathProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFFieldPathProperty(PyFProperty);
    prop_class!(PyFFieldPathProperty, PyFProperty, "FFieldPathProperty");
    #[pyclass(with(Representable))]
    impl PyFFieldPathProperty {}
    #[pyattr]
    #[pyclass(name = "FOptionalProperty", module = "ue", base = PyFProperty)]
    #[repr(transparent)]
    pub struct PyFOptionalProperty(PyFProperty);
    prop_class!(PyFOptionalProperty, PyFProperty, "FOptionalProperty");
    #[pyclass(with(Representable))]
    impl PyFOptionalProperty {}

    /// Wrap an `FProperty` as the most-derived Python proxy its cast flags
    /// allow. UE sets every ancestor's bit in `ClassCastFlags`, so the test has
    /// to run most-derived-first — the same shape as `wrap_obj` for UObjects.
    pub(super) fn wrap_prop(
        vm: &VirtualMachine,
        prop: *const ue::FProperty,
        anchor: ue::FWeakObjectPtr,
    ) -> PyObjectRef {
        use ue::EClassCastFlags as F;
        match unsafe { &*prop }.class().cast_flags() {
            f if f.contains(F::CASTCLASS_FClassProperty) => {
                PyFClassProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FSoftClassProperty) => {
                PyFSoftClassProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FByteProperty) => {
                PyFByteProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FInt8Property) => {
                PyFInt8Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FInt16Property) => {
                PyFInt16Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FIntProperty) => {
                PyFIntProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FInt64Property) => {
                PyFInt64Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FUInt16Property) => {
                PyFUInt16Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FUInt32Property) => {
                PyFUInt32Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FUInt64Property) => {
                PyFUInt64Property::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FFloatProperty) => {
                PyFFloatProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FDoubleProperty) => {
                PyFDoubleProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FObjectProperty) => {
                PyFObjectProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FWeakObjectProperty) => {
                PyFWeakObjectProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FLazyObjectProperty) => {
                PyFLazyObjectProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FSoftObjectProperty) => {
                PyFSoftObjectProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FMulticastInlineDelegateProperty) => {
                PyFMulticastInlineDelegateProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FMulticastSparseDelegateProperty) => {
                PyFMulticastSparseDelegateProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FNumericProperty) => {
                PyFNumericProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FBoolProperty) => {
                PyFBoolProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FObjectPropertyBase) => {
                PyFObjectPropertyBase::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FInterfaceProperty) => {
                PyFInterfaceProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FNameProperty) => {
                PyFNameProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FStrProperty) => {
                PyFStrProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FTextProperty) => {
                PyFTextProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FArrayProperty) => {
                PyFArrayProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FSetProperty) => {
                PyFSetProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FMapProperty) => {
                PyFMapProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FStructProperty) => {
                PyFStructProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FDelegateProperty) => {
                PyFDelegateProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FMulticastDelegateProperty) => {
                PyFMulticastDelegateProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FEnumProperty) => {
                PyFEnumProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FFieldPathProperty) => {
                PyFFieldPathProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            f if f.contains(F::CASTCLASS_FOptionalProperty) => {
                PyFOptionalProperty::wrap(prop, anchor).into_pyobject(vm)
            }
            _ => PyFProperty::wrap(prop, anchor).into_pyobject(vm),
        }
    }

    #[pyattr]
    #[pyclass(name = "Param", module = "ue")]
    #[derive(PyPayload)]
    pub struct Param {
        name: String,
        role: String,
        flags: u64,
        type_: PyObjectRef,
    }

    #[pyclass(with(Constructor, Representable))]
    impl Param {
        #[pygetset]
        fn name(&self) -> String {
            self.name.clone()
        }
        #[pygetset]
        fn role(&self) -> String {
            self.role.clone()
        }
        #[pygetset]
        fn flags(&self) -> u64 {
            self.flags
        }
        #[pygetset(name = "type_")]
        fn type_(&self) -> PyObjectRef {
            self.type_.clone()
        }
    }
    impl Representable for Param {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            // `type_` is one of the FProperty subclasses; they all share
            // `PyFProperty`'s payload, so the downcast covers every one.
            let ty = zelf
                .type_
                .downcast_ref::<PyFProperty>()
                .ok_or(())
                .and_then(|p| p.resolve(vm).map(type_name).map_err(|_| ()))
                .unwrap_or_else(|_| "stale".into());
            Ok(format!("<Param {}: {} [{}]>", zelf.name, ty, zelf.role))
        }
    }

    #[pyattr]
    #[pyclass(name = "Function", module = "ue")]
    #[derive(PyPayload)]
    pub struct Function {
        function: *const ue::UFunction,
        anchor: ue::FWeakObjectPtr,
    }

    impl Function {
        /// Check-then-deref is sound only because GC shares this thread: the
        /// weak check and every use of the returned ref happen inside one
        /// Python call with no engine re-entry. Do NOT hold the returned
        /// `&UFunction` across anything that can run the engine (ProcessEvent,
        /// object construction, …) — that can collect the function under it.
        fn resolve(&self, vm: &VirtualMachine) -> PyResult<&ue::UFunction> {
            if self.anchor.get_even_if_pending_kill().is_none() {
                return Err(attr_err(vm, "owning UStruct is stale"));
            }
            Ok(unsafe { &*self.function })
        }
    }

    /// Wrap a UFunction as a structured `Function`, anchored to the function's
    /// own object (a UFunction is a UObject, so it's a valid weak-ptr target and
    /// goes stale exactly when the function itself is GC'd).
    fn function_proxy(vm: &VirtualMachine, func: &ue::UFunction) -> PyRef<Function> {
        let anchor = ue::FWeakObjectPtr::new(func);
        Function {
            function: func as *const _,
            anchor,
        }
        .into_ref(&vm.ctx)
    }

    #[pyclass(with(Constructor, Representable))]
    impl Function {
        #[pygetset]
        fn name(&self, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self.resolve(vm)?.name_private().to_string())
        }
        #[pygetset]
        fn function_flags(&self, vm: &VirtualMachine) -> PyResult<u32> {
            Ok(self.resolve(vm)?.function_flags().bits())
        }
        /// Full human-readable signature string (params, return, flags).
        #[pygetset]
        fn signature(&self, vm: &VirtualMachine) -> PyResult<String> {
            Ok(render_signature(self.resolve(vm)?))
        }
        #[pymethod]
        fn flag_names(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let bits = self.resolve(vm)?.function_flags().bits() as u64;
            Ok(to_list(vm, set_flag_names(&function_flag_table(), bits)))
        }
        #[pymethod]
        fn params(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let func = self.resolve(vm)?;
            let mut out: Vec<PyRef<Param>> = Vec::new();
            for field in func.iter_props() {
                let Some(prop) = field.cast::<ue::FProperty>() else {
                    continue;
                };
                if !prop.property_flags().contains(ue::EPropertyFlags::CPF_Parm) {
                    continue;
                }
                let pt = wrap_prop(vm, prop as *const _, self.anchor);
                out.push(
                    Param {
                        name: prop.name().to_string(),
                        role: parm_role(prop).to_string(),
                        flags: prop.property_flags().bits(),
                        type_: pt,
                    }
                    .into_ref(&vm.ctx),
                );
            }
            Ok(to_list(vm, out))
        }
    }
    impl Representable for Function {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.resolve(vm) {
                Ok(f) => format!("<Function {}>", render_signature(f)),
                Err(_) => "<Function stale>".into(),
            })
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Path — re-resolves a slot in a UObject's memory on every access.
    // ──────────────────────────────────────────────────────────────────────

    /// `PyObjectRef` is `Rc`-based, so `MapKey` clones trivially — no GIL dance.
    #[derive(Clone)]
    pub(super) enum PathStep {
        Field(ue::FName),
        Index(i32),
        MapKey(PyObjectRef),
    }

    impl std::fmt::Debug for PathStep {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                PathStep::Field(n) => write!(f, "Field({})", n),
                PathStep::Index(i) => write!(f, "Index({})", i),
                PathStep::MapKey(_) => write!(f, "MapKey(...)"),
            }
        }
    }

    #[derive(Clone)]
    pub(super) struct Path {
        root: PathRoot,
        steps: Vec<PathStep>,
    }

    #[derive(Clone)]
    enum PathRoot {
        Object(ue::FWeakObjectPtr),
        Frame {
            base: usize,
            prop: *const ue::FProperty,
            guard: Rc<Cell<bool>>,
        },
        /// A Python-owned struct buffer (function-returned / copied-out). The
        /// `Rc` keeps the buffer alive for as long as any proxy references it, so
        /// — unlike `Object`/`Frame` — this root can never go stale.
        Owned {
            root: Rc<OwnedStructRoot>,
            offset: usize,
        },
    }

    #[derive(Clone, Copy)]
    enum WalkContext {
        Struct(*const ue::UStruct),
        Array(*const ue::FArrayProperty),
        Map(*const ue::FMapProperty),
        Terminal,
    }

    fn context_for_property(prop: &ue::FProperty) -> WalkContext {
        if let Some(sp) = prop.cast_field::<ue::FStructProperty>() {
            WalkContext::Struct(sp.get_struct() as *const _)
        } else if let Some(ap) = prop.cast_field::<ue::FArrayProperty>() {
            WalkContext::Array(ap as *const _)
        } else if let Some(mp) = prop.cast_field::<ue::FMapProperty>() {
            WalkContext::Map(mp as *const _)
        } else {
            WalkContext::Terminal
        }
    }

    /// Find an entry in an FScriptMap by Python key.
    fn find_map_key_index(
        vm: &VirtualMachine,
        map: &ue::FScriptMap,
        key_prop: &ue::FProperty,
        layout: &ue::FScriptMapLayout,
        py_key: PyObjectRef,
    ) -> PyResult<Option<i32>> {
        let key_size = key_prop.element_size() as usize;
        let mut key_buf: Vec<u8> = vec![0u8; key_size];
        let key_buf_ptr = key_buf.as_mut_ptr() as *mut c_void;
        let result = unsafe {
            key_prop.initialize_value(key_buf_ptr);
            match write_property(vm, key_prop, key_buf_ptr, py_key) {
                Err(e) => Err(e),
                Ok(()) => {
                    let _hash = key_prop.get_value_type_hash(key_buf_ptr as *const c_void);
                    let mut found: Option<i32> = None;
                    for idx in 0..map.get_max_index() {
                        if !map.is_valid_index(idx) {
                            continue;
                        }
                        let pair_ptr = map.get_data(idx, layout) as *const c_void;
                        if key_prop.identical(pair_ptr, key_buf_ptr as *const c_void) {
                            found = Some(idx);
                            break;
                        }
                    }
                    Ok(found)
                }
            }
        };
        unsafe { key_prop.destroy_value(key_buf_ptr) };
        result
    }

    fn walk_step(
        data: *mut c_void,
        ctx: WalkContext,
        step: &PathStep,
        vm: &VirtualMachine,
    ) -> PyResult<(*mut c_void, WalkContext, &'static ue::FProperty)> {
        match step {
            PathStep::Field(name) => {
                let WalkContext::Struct(s) = ctx else {
                    return Err(type_err(
                        vm,
                        "can't .Field-access here (current slot is not a struct)",
                    ));
                };
                let s = unsafe { &*s };
                let field = s
                    .iter_props()
                    .find(|f| f.name() == *name)
                    .ok_or_else(|| attr_err(vm, name.to_string()))?;
                let prop: &ue::FProperty = field
                    .cast::<ue::FProperty>()
                    .ok_or_else(|| type_err(vm, format!("{name}: not an FProperty")))?;
                let prop_static: &'static ue::FProperty = unsafe { std::mem::transmute(prop) };
                let new_data = unsafe { data.byte_add(prop.offset() as usize) };
                Ok((new_data, context_for_property(prop), prop_static))
            }
            PathStep::Index(i) => {
                let WalkContext::Array(ap_ptr) = ctx else {
                    return Err(type_err(
                        vm,
                        "can't .[i]-index here (current slot is not an array)",
                    ));
                };
                let ap = unsafe { &*ap_ptr };
                let arr = unsafe { &*(data as *const ue::FScriptArray) };
                if !arr.is_valid_index(*i) {
                    return Err(index_err(
                        vm,
                        format!("index {i} out of bounds (len {})", arr.num()),
                    ));
                }
                let inner: &ue::FProperty = ap.inner();
                let inner_static: &'static ue::FProperty = unsafe { std::mem::transmute(inner) };
                let elem_size = inner.element_size() as usize;
                let base = arr.get_data() as *mut u8;
                let elem = unsafe { base.add(*i as usize * elem_size) } as *mut c_void;
                Ok((elem, context_for_property(inner), inner_static))
            }
            PathStep::MapKey(py_key) => {
                let WalkContext::Map(mp_ptr) = ctx else {
                    return Err(type_err(
                        vm,
                        "can't .[key] here (current slot is not a map)",
                    ));
                };
                let mp = unsafe { &*mp_ptr };
                let map = unsafe { &*(data as *const ue::FScriptMap) };
                let layout = mp.map_layout();
                let key_prop = mp.key_property();
                let value_prop = mp.value_property();

                let idx = find_map_key_index(vm, map, key_prop, layout, py_key.clone())?
                    .ok_or_else(|| key_err(vm, "key not found"))?;

                let pair_ptr = map.get_data(idx, layout);
                let value_ptr = unsafe { (pair_ptr as *const u8).add(layout.value_offset as usize) }
                    as *mut c_void;
                let value_static: &'static ue::FProperty =
                    unsafe { std::mem::transmute(value_prop) };
                Ok((value_ptr, context_for_property(value_prop), value_static))
            }
        }
    }

    impl Path {
        fn root(owner: ue::FWeakObjectPtr) -> Self {
            Self {
                root: PathRoot::Object(owner),
                steps: Vec::new(),
            }
        }
        fn frame(base: usize, prop: *const ue::FProperty, guard: Rc<Cell<bool>>) -> Self {
            Self {
                root: PathRoot::Frame { base, prop, guard },
                steps: Vec::new(),
            }
        }
        fn owned(root: Rc<OwnedStructRoot>, offset: usize) -> Self {
            Self {
                root: PathRoot::Owned { root, offset },
                steps: Vec::new(),
            }
        }
        fn extend_field(&self, name: ue::FName) -> Self {
            let mut p = self.clone();
            p.steps.push(PathStep::Field(name));
            p
        }
        fn extend_index(&self, i: i32) -> Self {
            let mut p = self.clone();
            p.steps.push(PathStep::Index(i));
            p
        }
        fn extend_map_key(&self, key: PyObjectRef) -> Self {
            let mut p = self.clone();
            p.steps.push(PathStep::MapKey(key));
            p
        }

        fn owner_object(&self, vm: &VirtualMachine) -> PyResult<*mut ue::UObjectBase> {
            match &self.root {
                PathRoot::Object(weak) => {
                    let o = weak
                        .get_even_if_pending_kill()
                        .ok_or_else(|| attr_err(vm, "stale owner"))?;
                    Ok(std::ptr::from_ref(o).cast_mut())
                }
                PathRoot::Frame { .. } => Err(runtime_err(
                    vm,
                    "sparse delegate parameters can't be mutated (no owning object)",
                )),
                PathRoot::Owned { .. } => Err(runtime_err(
                    vm,
                    "delegates inside a copied-out struct can't be mutated (no owning object)",
                )),
            }
        }

        /// Resolve the root into `(data, walk-context, terminal-prop)`. The
        /// terminal prop is `None` for roots that aren't themselves a property
        /// (a UObject's fields, an owned struct's bytes); `walk_step` fills it in
        /// as steps are taken.
        fn root_start(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<(*mut c_void, WalkContext, Option<&'static ue::FProperty>)> {
            match &self.root {
                PathRoot::Object(weak) => {
                    let obj = weak
                        .get_even_if_pending_kill()
                        .ok_or_else(|| attr_err(vm, "stale owner"))?;
                    Ok((
                        obj as *const ue::UObjectBase as *mut c_void,
                        WalkContext::Struct(obj.class() as *const _ as *const ue::UStruct),
                        None,
                    ))
                }
                PathRoot::Frame { base, prop, guard } => {
                    if !guard.get() {
                        return Err(runtime_err(
                            vm,
                            "hook parameters are no longer valid (callback already returned)",
                        ));
                    }
                    let p: &'static ue::FProperty = unsafe { &**prop };
                    Ok((*base as *mut c_void, context_for_property(p), Some(p)))
                }
                PathRoot::Owned { root, offset } => {
                    let buf_ptr = unsafe { (*root.buf.get()).as_mut_ptr() };
                    Ok((
                        unsafe { buf_ptr.add(*offset) } as *mut c_void,
                        WalkContext::Struct(root.struct_ty),
                        None,
                    ))
                }
            }
        }

        fn resolve(&self, vm: &VirtualMachine) -> PyResult<(*mut c_void, &'static ue::FProperty)> {
            let (mut data, mut ctx, mut last_prop) = self.root_start(vm)?;
            for step in &self.steps {
                let (d, c, p) = walk_step(data, ctx, step, vm)?;
                data = d;
                ctx = c;
                last_prop = Some(p);
            }
            last_prop
                .map(|p| (data, p))
                .ok_or_else(|| value_err(vm, "empty path has no terminal property"))
        }

        /// Resolve to just the data pointer, tolerating an empty step list (the
        /// root struct of an owned/borrowed `Struct` has no terminal property).
        fn resolve_data(&self, vm: &VirtualMachine) -> PyResult<*mut c_void> {
            let (mut data, mut ctx, _) = self.root_start(vm)?;
            for step in &self.steps {
                let (d, c, _p) = walk_step(data, ctx, step, vm)?;
                data = d;
                ctx = c;
            }
            Ok(data)
        }

        /// [`resolve`] plus a check that the terminal property still has the
        /// expected cast flag. `what` names the container for the error message
        /// (e.g. `"an array"`). Used by the container proxies, whose path could
        /// in principle re-resolve to a different property type after mutation.
        fn resolve_as(
            &self,
            vm: &VirtualMachine,
            flag: ue::EClassCastFlags,
            what: &str,
        ) -> PyResult<(*mut c_void, &'static ue::FProperty)> {
            let (data, prop) = self.resolve(vm)?;
            if !prop.class().cast_flags().contains(flag) {
                return Err(type_err(vm, format!("path no longer points at {what}")));
            }
            Ok((data, prop))
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Struct — proxy for an FStructProperty value. Whether the bytes live in a
    // foreign object (borrowed) or a Python-owned buffer (function-returned) is
    // entirely captured by the `Path` root, so field access goes through the same
    // walker either way — owned structs get array/map/set fields for free.
    // ──────────────────────────────────────────────────────────────────────

    pub(super) struct OwnedStructRoot {
        buf: std::cell::UnsafeCell<Box<[u8]>>,
        struct_ty: *const ue::UStruct,
    }

    impl Drop for OwnedStructRoot {
        fn drop(&mut self) {
            unsafe {
                let buf = &mut *self.buf.get();
                let destroy: FnStructOp = std::mem::transmute(vtable_fn(
                    self.struct_ty as *const c_void,
                    vtable_destroy_struct(),
                ));
                destroy(self.struct_ty, buf.as_mut_ptr() as *mut c_void, 1);
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "Struct", module = "ue")]
    #[derive(PyPayload)]
    pub struct Struct {
        path: Path,
        struct_ty: *const ue::UStruct,
    }

    impl Struct {
        fn data_and_struct(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<(*mut c_void, *const ue::UStruct)> {
            Ok((self.path.resolve_data(vm)?, self.struct_ty))
        }
    }

    #[pyclass(with(Constructor, GetAttr, SetAttr, Representable))]
    impl Struct {
        #[pymethod]
        fn __dir__(&self, vm: &VirtualMachine) -> PyObjectRef {
            let ty = unsafe { &*self.struct_ty };
            to_list(vm, ty.iter_props().map(|f| f.name().to_string()))
        }

        /// `s._type` — the UScriptStruct describing this value, the same object
        /// `FStructProperty.struct_type()` returns. Underscore-prefixed like
        /// `UObject._class`, to stay clear of FProperty names.
        #[pygetset]
        fn _type(&self, vm: &VirtualMachine) -> PyObjectRef {
            wrap_obj(vm, unsafe { &*self.struct_ty })
        }

        /// Shallow `{field: value}`. The inverse of the dict a struct field or
        /// param accepts, so a struct round-trips: nested structs stay `Struct`
        /// proxies, which is exactly what the writer takes back.
        #[pymethod]
        fn to_dict(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let ty = unsafe { &*self.struct_ty };
            let dict = vm.ctx.new_dict();
            for field in ty.iter_props() {
                let name = field.name();
                let value = read_at_path(vm, self.path.extend_field(name))?;
                dict.set_item(&*vm.ctx.new_str(name.to_string()), value, vm)?;
            }
            Ok(dict.into())
        }
    }

    impl Representable for Struct {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let ty = unsafe { &*zelf.struct_ty };
            let name = ty.name_private().to_string();
            let stale = zelf.path.resolve_data(vm).is_err();
            Ok(format!(
                "<Struct {name}{}>",
                if stale { " stale" } else { "" }
            ))
        }
    }

    impl Struct {
        /// Resolve an attribute name to an FName, failing with the *name the
        /// caller wrote*. `FName::find` returns index 0 for anything not in the
        /// engine's name table, and index 0 renders as "None" — so without this
        /// a typo'd field came back as `AttributeError: None`.
        fn field_name(name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<ue::FName> {
            let s = name.to_string();
            let fname = ue::FName::find(&s);
            if fname.comparison_index.value == 0 {
                return Err(attr_err(vm, s));
            }
            Ok(fname)
        }
    }

    impl GetAttr for Struct {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            if let Some(attr) = zelf.as_object().generic_getattr_opt(name, None, vm)? {
                return Ok(attr);
            }
            let new_path = zelf.path.extend_field(Struct::field_name(name, vm)?);
            read_at_path(vm, new_path)
        }
    }

    impl SetAttr for Struct {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let value = match value {
                PySetterValue::Assign(v) => v,
                PySetterValue::Delete => return Err(attr_err(vm, "cannot delete struct fields")),
            };
            let new_path = zelf.path.extend_field(Struct::field_name(name, vm)?);
            write_at_path(vm, new_path, value)
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Array — path-based proxy for a TArray inside the owner.
    // ──────────────────────────────────────────────────────────────────────

    /// Normalize a Python sequence index against `len`: negatives count from the
    /// end, and the result is bounds-checked into `0..len` (IndexError if not).
    fn wrap_index(vm: &VirtualMachine, index: i32, len: i32) -> PyResult<i32> {
        let i = if index < 0 { index + len } else { index };
        if i < 0 || i >= len {
            return Err(index_err(
                vm,
                format!("array index {index} out of range (len {len})"),
            ));
        }
        Ok(i)
    }

    #[pyattr]
    #[pyclass(name = "Array", module = "ue")]
    #[derive(PyPayload)]
    pub struct Array {
        path: Path,
        inner: *const ue::FProperty,
    }

    impl Array {
        fn resolve(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<(*mut ue::FScriptArray, &'static ue::FArrayProperty)> {
            let (data, prop) = self.path.resolve_as(
                vm,
                ue::EClassCastFlags::CASTCLASS_FArrayProperty,
                "an array",
            )?;
            // resolve_as guaranteed the cast flag, so this can't fail.
            let ap = prop.cast_field::<ue::FArrayProperty>().unwrap();
            Ok((data as *mut ue::FScriptArray, ap))
        }

        fn len_impl(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let (arr, _) = self.resolve(vm)?;
            Ok(unsafe { (*arr).num() } as usize)
        }
        fn getitem_impl(&self, index: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let (arr, _) = self.resolve(vm)?;
            let i = wrap_index(vm, index, unsafe { (*arr).num() })?;
            read_at_path(vm, self.path.extend_index(i))
        }
        fn setitem_impl(
            &self,
            index: i32,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let (arr, _) = self.resolve(vm)?;
            let i = wrap_index(vm, index, unsafe { (*arr).num() })?;
            write_at_path(vm, self.path.extend_index(i), value)
        }
        fn contains_impl(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            Ok(self.find_index(&value, vm)?.is_some())
        }
        /// Index of the first element equal to `value`, by Python equality —
        /// shared by `in`, `index`, and `remove`.
        fn find_index(&self, value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<i32>> {
            let (arr, _) = self.resolve(vm)?;
            let n = unsafe { (*arr).num() };
            for i in 0..n {
                let item = read_at_path(vm, self.path.extend_index(i))?;
                if vm.bool_eq(&item, value)? {
                    return Ok(Some(i));
                }
            }
            Ok(None)
        }
        fn delitem_impl(&self, index: i32, vm: &VirtualMachine) -> PyResult<()> {
            let (arr_ptr, _ap) = self.resolve(vm)?;
            let arr = unsafe { &mut *arr_ptr };
            let inner = unsafe { &*self.inner };
            let num = arr.num();
            let index = wrap_index(vm, index, num)?;
            let elem_size = inner.element_size();
            let base = arr.get_data_mut() as *mut u8;
            let slot = unsafe { base.add(index as usize * elem_size as usize) } as *mut c_void;
            unsafe {
                inner.destroy_value(slot);
                arr.remove(index, 1, elem_size);
            }
            Ok(())
        }
    }

    #[pyclass(with(Constructor, AsMapping, AsSequence, Iterable, Representable))]
    impl Array {
        #[pymethod]
        fn append(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let (arr_ptr, _ap) = self.resolve(vm)?;
            let arr = unsafe { &mut *arr_ptr };
            let inner = unsafe { &*self.inner };
            let elem_size = inner.element_size();
            let new_idx = arr.add(1, elem_size);
            let base = arr.get_data_mut() as *mut u8;
            let slot = unsafe { base.add(new_idx as usize * elem_size as usize) } as *mut c_void;
            unsafe { inner.initialize_value(slot) };
            if let Err(e) = write_property(vm, inner, slot, value) {
                unsafe {
                    inner.destroy_value(slot);
                    arr.remove(new_idx, 1, elem_size);
                }
                return Err(e);
            }
            Ok(())
        }

        #[pymethod]
        fn insert(&self, index: i32, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let (arr_ptr, _ap) = self.resolve(vm)?;
            let arr = unsafe { &mut *arr_ptr };
            let inner = unsafe { &*self.inner };
            let elem_size = inner.element_size();
            let num = arr.num();
            // Match `list.insert`: negatives count from the end, and both ends
            // clamp rather than raise.
            let index = if index < 0 {
                (num + index).max(0)
            } else {
                index.min(num)
            };
            arr.insert(index, 1, elem_size);
            let base = arr.get_data_mut() as *mut u8;
            let slot = unsafe { base.add(index as usize * elem_size as usize) } as *mut c_void;
            unsafe { inner.initialize_value(slot) };
            if let Err(e) = write_property(vm, inner, slot, value) {
                unsafe {
                    inner.destroy_value(slot);
                    arr.remove(index, 1, elem_size);
                }
                return Err(e);
            }
            Ok(())
        }

        /// `a.pop()` / `a.pop(i)` — like `list.pop`, defaulting to the last
        /// element and accepting negative indices.
        #[pymethod]
        fn pop(&self, index: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let (arr_ptr, _ap) = self.resolve(vm)?;
            let arr = unsafe { &mut *arr_ptr };
            let inner = unsafe { &*self.inner };
            let num = arr.num();
            if num == 0 {
                return Err(index_err(vm, "pop from empty array"));
            }
            let idx = match index.into_option() {
                Some(i) => wrap_index(vm, i, num)?,
                None => num - 1,
            };
            let elem_size = inner.element_size();
            let base = arr.get_data_mut() as *mut u8;
            let slot = unsafe { base.add(idx as usize * elem_size as usize) } as *mut c_void;
            let value = read_disjoint(vm, inner, slot)?;
            unsafe {
                inner.destroy_value(slot);
                arr.remove(idx, 1, elem_size);
            }
            Ok(value)
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            let (arr_ptr, _ap) = self.resolve(vm)?;
            let arr = unsafe { &mut *arr_ptr };
            let inner = unsafe { &*self.inner };
            let elem_size = inner.element_size();
            let num = arr.num();
            let base = arr.get_data_mut() as *mut u8;
            for i in 0..num {
                let slot = unsafe { base.add(i as usize * elem_size as usize) } as *mut c_void;
                unsafe { inner.destroy_value(slot) };
            }
            arr.empty(0, elem_size);
            Ok(())
        }

        #[pymethod]
        fn extend(&self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            for item in py_iter_collect(vm, &iterable)? {
                self.append(item, vm)?;
            }
            Ok(())
        }

        /// `a.index(v)` — first index equal to `v`, ValueError if absent.
        #[pymethod]
        fn index(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
            self.find_index(&value, vm)?
                .ok_or_else(|| value_err(vm, "value not in array"))
        }

        /// `a.count(v)` — how many elements equal `v`.
        #[pymethod]
        fn count(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            let (arr, _) = self.resolve(vm)?;
            let n = unsafe { (*arr).num() };
            let mut hits = 0;
            for i in 0..n {
                let item = read_at_path(vm, self.path.extend_index(i))?;
                if vm.bool_eq(&item, &value)? {
                    hits += 1;
                }
            }
            Ok(hits)
        }

        /// `a.remove(v)` — delete the first element equal to `v`, ValueError if
        /// absent. (`del a[i]` removes by index.)
        #[pymethod]
        fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let idx = self
                .find_index(&value, vm)?
                .ok_or_else(|| value_err(vm, "value not in array"))?;
            self.delitem_impl(idx, vm)
        }
    }

    impl Representable for Array {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let inner = unsafe { &*zelf.inner };
            let inner_name = inner.class().name.to_string();
            let len = match zelf.resolve(vm) {
                Ok((arr, _)) => unsafe { (*arr).num() },
                Err(_) => -1,
            };
            Ok(format!("<Array<{inner_name}> len={len}>"))
        }
    }

    impl AsMapping for Array {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, vm| Array::mapping_downcast(mapping).len_impl(vm)),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let zelf = Array::mapping_downcast(mapping);
                    let index: i32 = needle.try_to_value(vm)?;
                    zelf.getitem_impl(index, vm)
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = Array::mapping_downcast(mapping);
                    let index: i32 = needle.try_to_value(vm)?;
                    match value {
                        Some(value) => zelf.setitem_impl(index, value, vm),
                        None => zelf.delitem_impl(index, vm),
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for Array {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                contains: atomic_func!(|seq, needle, vm| {
                    Array::sequence_downcast(seq).contains_impl(needle.to_owned(), vm)
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for Array {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let (arr, _) = zelf.resolve(vm)?;
            let n = unsafe { (*arr).num() };
            let mut items: Vec<PyObjectRef> = Vec::with_capacity(n.max(0) as usize);
            for i in 0..n {
                items.push(read_at_path(vm, zelf.path.extend_index(i))?);
            }
            Ok(vm.ctx.new_list(items).to_pyobject(vm).get_iter(vm)?.into())
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Map — path-based proxy for a TMap.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "Map", module = "ue")]
    #[derive(PyPayload)]
    pub struct Map {
        path: Path,
        key_prop: *const ue::FProperty,
        value_prop: *const ue::FProperty,
        layout: ue::FScriptMapLayout,
    }

    impl Map {
        fn resolve(&self, vm: &VirtualMachine) -> PyResult<*mut ue::FScriptMap> {
            let (data, _prop) =
                self.path
                    .resolve_as(vm, ue::EClassCastFlags::CASTCLASS_FMapProperty, "a map")?;
            Ok(data as *mut ue::FScriptMap)
        }
        fn len_impl(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let m = self.resolve(vm)?;
            Ok(unsafe { (*m).num() } as usize)
        }
        fn contains_impl(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let m = self.resolve(vm)?;
            let map = unsafe { &*m };
            let kp = unsafe { &*self.key_prop };
            Ok(find_map_key_index(vm, map, kp, &self.layout, key)?.is_some())
        }
        fn getitem_impl(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let new_path = self.path.extend_map_key(key);
            read_at_path(vm, new_path)
        }
        fn setitem_impl(
            &self,
            key: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let m = self.resolve(vm)?;
            let map = unsafe { &mut *m };
            let kp = unsafe { &*self.key_prop };
            let vp = unsafe { &*self.value_prop };
            if let Some(idx) = find_map_key_index(vm, map, kp, &self.layout, key.clone())? {
                let pair_ptr = map.get_data_mut(idx, &self.layout);
                let value_ptr =
                    unsafe { (pair_ptr as *mut u8).add(self.layout.value_offset as usize) }
                        as *mut c_void;
                return write_property(vm, vp, value_ptr, value);
            }
            let new_idx = map.add_uninitialized(&self.layout);
            let pair_ptr = map.get_data_mut(new_idx, &self.layout);
            let key_ptr = pair_ptr as *mut c_void;
            let value_ptr = unsafe { (pair_ptr as *mut u8).add(self.layout.value_offset as usize) }
                as *mut c_void;
            unsafe {
                kp.initialize_value(key_ptr);
                vp.initialize_value(value_ptr);
            }
            let write_result = write_property(vm, kp, key_ptr, key)
                .and_then(|_| write_property(vm, vp, value_ptr, value));
            if let Err(e) = write_result {
                unsafe {
                    kp.destroy_value(key_ptr);
                    vp.destroy_value(value_ptr);
                }
                map.remove_at(new_idx, &self.layout);
                return Err(e);
            }
            map.rehash(&self.layout, |k| unsafe { kp.get_value_type_hash(k) });
            Ok(())
        }
        fn delitem_impl(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let m = self.resolve(vm)?;
            let map = unsafe { &mut *m };
            let kp = unsafe { &*self.key_prop };
            let vp = unsafe { &*self.value_prop };
            let idx = find_map_key_index(vm, map, kp, &self.layout, key)?
                .ok_or_else(|| key_err(vm, "key not found"))?;
            let pair_ptr = map.get_data_mut(idx, &self.layout);
            let key_ptr = pair_ptr as *mut c_void;
            let value_ptr = unsafe { (pair_ptr as *mut u8).add(self.layout.value_offset as usize) }
                as *mut c_void;
            unsafe {
                kp.destroy_value(key_ptr);
                vp.destroy_value(value_ptr);
            }
            map.remove_at(idx, &self.layout);
            Ok(())
        }
        fn keys_vec(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let m = self.resolve(vm)?;
            let map = unsafe { &*m };
            let kp = unsafe { &*self.key_prop };
            let mut out = Vec::with_capacity(map.num() as usize);
            for idx in 0..map.get_max_index() {
                if !map.is_valid_index(idx) {
                    continue;
                }
                let pair_ptr = map.get_data(idx, &self.layout) as *mut c_void;
                out.push(read_disjoint(vm, kp, pair_ptr)?);
            }
            Ok(out)
        }
    }

    #[pyclass(with(Constructor, AsMapping, AsSequence, Iterable, Representable))]
    impl Map {
        #[pymethod]
        fn keys(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            self.keys_vec(vm)
        }
        #[pymethod]
        fn values(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let key_list = self.keys_vec(vm)?;
            let mut out = Vec::with_capacity(key_list.len());
            for k in key_list {
                out.push(read_at_path(vm, self.path.extend_map_key(k))?);
            }
            Ok(out)
        }
        #[pymethod]
        fn items(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let key_list = self.keys_vec(vm)?;
            let mut out: Vec<PyObjectRef> = Vec::with_capacity(key_list.len());
            for k in key_list {
                let v = read_at_path(vm, self.path.extend_map_key(k.clone()))?;
                out.push(vm.new_pyobj((k, v)));
            }
            Ok(vm.ctx.new_list(out).into())
        }
        /// `m.get(key, default=None)` — like `dict.get`, no KeyError.
        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            if self.contains_impl(key.clone(), vm)? {
                return self.getitem_impl(key, vm);
            }
            Ok(default.into_option().unwrap_or_else(|| vm.ctx.none()))
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            let m = self.resolve(vm)?;
            let map = unsafe { &mut *m };
            let kp = unsafe { &*self.key_prop };
            let vp = unsafe { &*self.value_prop };
            for idx in 0..map.get_max_index() {
                if !map.is_valid_index(idx) {
                    continue;
                }
                let pair_ptr = map.get_data_mut(idx, &self.layout);
                let key_ptr = pair_ptr as *mut c_void;
                let value_ptr =
                    unsafe { (pair_ptr as *mut u8).add(self.layout.value_offset as usize) }
                        as *mut c_void;
                unsafe {
                    kp.destroy_value(key_ptr);
                    vp.destroy_value(value_ptr);
                }
            }
            map.empty(0, &self.layout);
            Ok(())
        }
    }

    impl Representable for Map {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let kn = unsafe { (*zelf.key_prop).class().name.to_string() };
            let vn = unsafe { (*zelf.value_prop).class().name.to_string() };
            let len = zelf
                .resolve(vm)
                .ok()
                .map(|m| unsafe { (*m).num() })
                .unwrap_or(-1);
            Ok(format!("<Map<{kn}, {vn}> len={len}>"))
        }
    }
    impl AsMapping for Map {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, vm| Map::mapping_downcast(mapping).len_impl(vm)),
                subscript: atomic_func!(|mapping, needle, vm| {
                    Map::mapping_downcast(mapping).getitem_impl(needle.to_owned(), vm)
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = Map::mapping_downcast(mapping);
                    match value {
                        Some(value) => zelf.setitem_impl(needle.to_owned(), value, vm),
                        None => zelf.delitem_impl(needle.to_owned(), vm),
                    }
                }),
            };
            &AS_MAPPING
        }
    }
    impl AsSequence for Map {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                contains: atomic_func!(|seq, needle, vm| {
                    Map::sequence_downcast(seq).contains_impl(needle.to_owned(), vm)
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }
    impl Iterable for Map {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let keys = zelf.keys_vec(vm)?;
            Ok(vm.ctx.new_list(keys).to_pyobject(vm).get_iter(vm)?.into())
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Set — same shape as Map, elements only.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "Set", module = "ue")]
    #[derive(PyPayload)]
    pub struct Set {
        path: Path,
        element_prop: *const ue::FProperty,
        layout: ue::FScriptSetLayout,
    }

    impl Set {
        fn resolve(&self, vm: &VirtualMachine) -> PyResult<*mut ue::FScriptSet> {
            let (data, _prop) =
                self.path
                    .resolve_as(vm, ue::EClassCastFlags::CASTCLASS_FSetProperty, "a set")?;
            Ok(data as *mut ue::FScriptSet)
        }
        fn len_impl(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let s = self.resolve(vm)?;
            Ok(unsafe { (*s).num() } as usize)
        }
        fn contains_impl(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let s = self.resolve(vm)?;
            let set = unsafe { &*s };
            let ep = unsafe { &*self.element_prop };
            Ok(find_set_element_index(vm, set, ep, &self.layout, value)?.is_some())
        }
        fn items_vec(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let s = self.resolve(vm)?;
            let set = unsafe { &*s };
            let ep = unsafe { &*self.element_prop };
            let mut items: Vec<PyObjectRef> = Vec::with_capacity(set.num() as usize);
            for idx in 0..set.get_max_index() {
                if !set.is_valid_index(idx) {
                    continue;
                }
                let elem_ptr = set.get_data(idx, &self.layout) as *mut c_void;
                items.push(read_disjoint(vm, ep, elem_ptr)?);
            }
            Ok(items)
        }
    }

    #[pyclass(with(Constructor, AsSequence, Iterable, Representable))]
    impl Set {
        #[pymethod]
        fn add(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let s = self.resolve(vm)?;
            let set = unsafe { &mut *s };
            let ep = unsafe { &*self.element_prop };
            if find_set_element_index(vm, set, ep, &self.layout, value.clone())?.is_some() {
                return Ok(false);
            }
            let new_idx = set.add_uninitialized(&self.layout);
            let slot = set.get_data_mut(new_idx, &self.layout) as *mut c_void;
            unsafe { ep.initialize_value(slot) };
            if let Err(e) = write_property(vm, ep, slot, value) {
                unsafe { ep.destroy_value(slot) };
                set.remove_at(new_idx, &self.layout);
                return Err(e);
            }
            set.rehash(&self.layout, |e| unsafe { ep.get_value_type_hash(e) });
            Ok(true)
        }
        #[pymethod]
        fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let s = self.resolve(vm)?;
            let set = unsafe { &mut *s };
            let ep = unsafe { &*self.element_prop };
            let idx = find_set_element_index(vm, set, ep, &self.layout, value)?
                .ok_or_else(|| key_err(vm, "element not in set"))?;
            let slot = set.get_data_mut(idx, &self.layout) as *mut c_void;
            unsafe { ep.destroy_value(slot) };
            set.remove_at(idx, &self.layout);
            Ok(())
        }
        #[pymethod]
        fn discard(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let s = self.resolve(vm)?;
            let set = unsafe { &mut *s };
            let ep = unsafe { &*self.element_prop };
            if let Some(idx) = find_set_element_index(vm, set, ep, &self.layout, value)? {
                let slot = set.get_data_mut(idx, &self.layout) as *mut c_void;
                unsafe { ep.destroy_value(slot) };
                set.remove_at(idx, &self.layout);
                Ok(true)
            } else {
                Ok(false)
            }
        }
        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            let s = self.resolve(vm)?;
            let set = unsafe { &mut *s };
            let ep = unsafe { &*self.element_prop };
            for idx in 0..set.get_max_index() {
                if !set.is_valid_index(idx) {
                    continue;
                }
                let slot = set.get_data_mut(idx, &self.layout) as *mut c_void;
                unsafe { ep.destroy_value(slot) };
            }
            set.empty(0, &self.layout);
            Ok(())
        }
    }

    impl Representable for Set {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let en = unsafe { (*zelf.element_prop).class().name.to_string() };
            let len = zelf
                .resolve(vm)
                .ok()
                .map(|s| unsafe { (*s).num() })
                .unwrap_or(-1);
            Ok(format!("<Set<{en}> len={len}>"))
        }
    }
    impl AsSequence for Set {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, vm| Set::sequence_downcast(seq).len_impl(vm)),
                contains: atomic_func!(|seq, needle, vm| {
                    Set::sequence_downcast(seq).contains_impl(needle.to_owned(), vm)
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }
    impl Iterable for Set {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let items = zelf.items_vec(vm)?;
            Ok(vm.ctx.new_list(items).to_pyobject(vm).get_iter(vm)?.into())
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Text — an opaque, owning handle to an `FText` value.
    //
    // `FText` is a threadsafe refcounted `TSharedRef<ITextData>`, so its
    // lifetime can't be modeled by hand. A handle owns an `FText`-sized heap
    // slot and drives every transition through the property's *engine* vtable
    // ops — `copy_complete_value` (= `FText::operator=`) and `destroy_value`
    // (= `~FText`) — the same ops the container code already uses. This adds no
    // new resolved function addresses; the only new knowledge is `FText`'s size
    // (from the property's `element_size`, cross-checked against
    // `layout().ftext.size`).
    //
    // The handle is opaque by design: it can be read out of a property/return,
    // held, and written back into any `FText` property or UFunction param (e.g.
    // `widget.SetText(t)`), but it is not converted to/from `str` here — that
    // goes through reflected Kismet conversions (`Conv_StringToText` /
    // `Conv_TextToString`) if desired.
    // ──────────────────────────────────────────────────────────────────────
    #[pyattr]
    #[pyclass(name = "Text", module = "ue")]
    #[derive(Debug, PyPayload)]
    pub struct Text {
        /// Owning, 16-byte-aligned storage holding a live `FText` (or a fixed
        /// `FText[N]` when `array_dim > 1`).
        buf: std::ptr::NonNull<u8>,
        /// Byte length of `buf` (`array_dim * element_size`).
        size: usize,
        /// Any live `FTextProperty` — supplies the type-uniform engine vtable
        /// ops that copy/destroy the `FText` in `buf`. Process-lifetime.
        prop: *const ue::FProperty,
    }

    impl Text {
        /// `FText` holds pointers + an atomic controller (8-byte align); 16 is a
        /// safe over-alignment matching the engine allocator.
        fn buf_layout(size: usize) -> std::alloc::Layout {
            std::alloc::Layout::from_size_align(size, 16).expect("FText buffer layout")
        }

        /// Capture an owning copy of the `FText`(s) at `src`.
        ///
        /// The buffer is zeroed rather than engine-initialized: `FText::operator=`
        /// (invoked by `copy_complete_value`) null-checks the destination's shared
        /// controller before releasing it, so copying into zeroed memory correctly
        /// skips a pointless empty-`FText` construction and just adds a reference.
        ///
        /// # Safety
        /// `prop` must be an `FTextProperty` and `src` must point at its value.
        unsafe fn capture(prop: &ue::FProperty, src: *const c_void) -> Self {
            let elem = prop.element_size() as usize;
            debug_assert_eq!(
                elem,
                ue::globals().layout.ftext.size,
                "FText element_size disagrees with layout"
            );
            let size = prop.array_dim().max(1) as usize * elem;
            let raw = unsafe { std::alloc::alloc_zeroed(Self::buf_layout(size)) };
            let buf = std::ptr::NonNull::new(raw).expect("alloc_zeroed FText");
            unsafe { prop.copy_complete_value(buf.as_ptr().cast(), src) };
            Text {
                buf,
                size,
                prop: prop as *const _,
            }
        }

        /// Pointer to the owned `FText` value.
        fn as_ptr(&self) -> *const c_void {
            self.buf.as_ptr() as *const c_void
        }
    }

    impl Drop for Text {
        fn drop(&mut self) {
            // `~FText` — decrements the shared ref (freeing the `ITextData` when
            // the last reference goes), then release the backing storage.
            unsafe {
                (*self.prop).destroy_value(self.buf.as_ptr().cast());
                std::alloc::dealloc(self.buf.as_ptr(), Self::buf_layout(self.size));
            }
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl Text {}

    impl Representable for Text {
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            // Opaque — no ToString exposed. Identify the type only.
            Ok("<ue.Text>".to_owned())
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // FProperty dispatch — read/write the typed value at a data pointer.
    // ──────────────────────────────────────────────────────────────────────

    fn read_value(
        vm: &VirtualMachine,
        prop: &ue::FProperty,
        data: *mut c_void,
    ) -> PyResult<PyObjectRef> {
        use ue::EClassCastFlags as F;
        let flags = prop.class().cast_flags();

        macro_rules! int_at {
            ($T:ty) => {{
                return Ok(vm.new_pyobj(unsafe { *(data as *const $T) }));
            }};
        }

        if let Some(bp) = prop.cast_field::<ue::FBoolProperty>() {
            let v = unsafe { bp.value(data as *const ()).get() };
            return Ok(vm.new_pyobj(v));
        }
        if let Some(ep) = prop.cast_field::<ue::FEnumProperty>() {
            let value = unsafe { ep.read_value(data as *const ()) };
            if let Some(en) = ep.enum_ptr() {
                if let Some(name) = en.name_by_value(value) {
                    return Ok(vm.new_pyobj(name.to_string()));
                }
            }
            return Ok(vm.new_pyobj(value));
        }
        if let Some(bp) = prop.cast_field::<ue::FByteProperty>() {
            if let Some(en) = bp.enum_ptr() {
                let value = unsafe { (data as *const u8).read() } as i64;
                if let Some(name) = en.name_by_value(value) {
                    return Ok(vm.new_pyobj(name.to_string()));
                }
                return Ok(vm.new_pyobj(value));
            }
            return Ok(vm.new_pyobj(unsafe { *(data as *const u8) }));
        }
        if flags.contains(F::CASTCLASS_FIntProperty) {
            int_at!(i32)
        }
        if flags.contains(F::CASTCLASS_FInt64Property) {
            int_at!(i64)
        }
        if flags.contains(F::CASTCLASS_FInt16Property) {
            int_at!(i16)
        }
        if flags.contains(F::CASTCLASS_FInt8Property) {
            int_at!(i8)
        }
        if flags.contains(F::CASTCLASS_FUInt64Property) {
            int_at!(u64)
        }
        if flags.contains(F::CASTCLASS_FUInt32Property) {
            int_at!(u32)
        }
        if flags.contains(F::CASTCLASS_FUInt16Property) {
            int_at!(u16)
        }
        if flags.contains(F::CASTCLASS_FFloatProperty) {
            return Ok(vm
                .ctx
                .new_float(unsafe { *(data as *const f32) } as f64)
                .into());
        }
        if flags.contains(F::CASTCLASS_FDoubleProperty) {
            return Ok(vm.ctx.new_float(unsafe { *(data as *const f64) }).into());
        }

        if flags.contains(F::CASTCLASS_FStrProperty) {
            let s = unsafe { &*(data as *const ue::FString) };
            return Ok(vm.new_pyobj(s.to_string()));
        }
        if flags.contains(F::CASTCLASS_FNameProperty) {
            let n = unsafe { *(data as *const ue::FName) };
            return Ok(vm.new_pyobj(n.to_string()));
        }
        if flags.contains(F::CASTCLASS_FTextProperty) {
            // Owning, refcount-bumped copy — safe to outlive the source object.
            let t = unsafe { Text::capture(prop, data as *const c_void) };
            return Ok(t.into_pyobject(vm));
        }
        if flags.contains(F::CASTCLASS_FObjectProperty) {
            let p: *const ue::UObjectBase = unsafe { *(data as *const *const ue::UObjectBase) };
            return match unsafe { p.as_ref() } {
                Some(o) => Ok(wrap_obj(vm, o)),
                None => Ok(vm.ctx.none()),
            };
        }
        if flags.contains(F::CASTCLASS_FWeakObjectProperty) {
            let weak = unsafe { *(data as *const ue::FWeakObjectPtr) };
            return Ok(match weak.get_even_if_pending_kill() {
                Some(o) => wrap_obj(vm, o),
                None => vm.ctx.none(),
            });
        }
        if flags.contains(F::CASTCLASS_FInterfaceProperty) {
            // An interface slot holds an `FScriptInterface { UObject* ObjectPointer;
            // void* InterfacePointer; }`. The leading object pointer is what's useful
            // from Python — hand back the implementing UObject so its UFUNCTIONs (e.g.
            // the asset-registry query methods on the IAssetRegistry interface) are
            // callable. The raw interface vtable pointer is dropped.
            let p: *const ue::UObjectBase = unsafe { *(data as *const *const ue::UObjectBase) };
            return match unsafe { p.as_ref() } {
                Some(o) => Ok(wrap_obj(vm, o)),
                None => Ok(vm.ctx.none()),
            };
        }
        if flags.contains(F::CASTCLASS_FSoftObjectProperty) {
            let p = unsafe { &*(data as *const ue::FSoftObjectPtr) };
            let s = p.to_string();
            return Ok(if s.is_empty() {
                vm.ctx.none()
            } else {
                vm.new_pyobj(s)
            });
        }
        if flags.contains(F::CASTCLASS_FLazyObjectProperty) {
            let p = unsafe { &*(data as *const ue::FLazyObjectPtr) };
            return Ok(if p.object_id.guid.is_valid() {
                vm.new_pyobj(p.to_string())
            } else {
                vm.ctx.none()
            });
        }

        Err(type_err(
            vm,
            format!(
                "read: unsupported property type {} (no path context)",
                prop.class().name
            ),
        ))
    }

    fn read_disjoint(
        vm: &VirtualMachine,
        prop: &ue::FProperty,
        data: *mut c_void,
    ) -> PyResult<PyObjectRef> {
        if prop.cast_field::<ue::FStructProperty>().is_some() {
            return Ok(copy_struct_out(prop, data).into_pyobject(vm));
        }
        read_value(vm, prop, data)
    }

    fn deep_read(
        vm: &VirtualMachine,
        prop: &ue::FProperty,
        data: *mut c_void,
    ) -> PyResult<PyObjectRef> {
        if let Some(ap) = prop.cast_field::<ue::FArrayProperty>() {
            let inner = ap.inner();
            let arr = unsafe { &*(data as *const ue::FScriptArray) };
            let elem_size = inner.element_size() as usize;
            let n = arr.num();
            let mut items: Vec<PyObjectRef> = Vec::with_capacity(n.max(0) as usize);
            if n > 0 {
                let base = arr.get_data() as *const u8;
                for i in 0..n {
                    let e = unsafe { base.add(i as usize * elem_size) } as *mut c_void;
                    items.push(deep_read(vm, inner, e)?);
                }
            }
            return Ok(vm.ctx.new_list(items).into());
        }
        if let Some(mp) = prop.cast_field::<ue::FMapProperty>() {
            let (kp, vp, layout) = (mp.key_property(), mp.value_property(), mp.map_layout());
            let map = unsafe { &*(data as *const ue::FScriptMap) };
            let dict = vm.ctx.new_dict();
            for i in 0..map.get_max_index() {
                if !map.is_valid_index(i) {
                    continue;
                }
                let pair = map.get_data(i, layout) as *const u8;
                let key = pair as *mut c_void;
                let val = unsafe { pair.add(layout.value_offset as usize) } as *mut c_void;
                dict.set_item(&*deep_read(vm, kp, key)?, deep_read(vm, vp, val)?, vm)?;
            }
            return Ok(dict.into());
        }
        if let Some(sp) = prop.cast_field::<ue::FSetProperty>() {
            let ep = sp.element_property();
            let layout = sp.set_layout();
            let set = unsafe { &*(data as *const ue::FScriptSet) };
            let mut items: Vec<PyObjectRef> = Vec::new();
            for i in 0..set.get_max_index() {
                if !set.is_valid_index(i) {
                    continue;
                }
                let e = set.get_data(i, layout) as *mut c_void;
                items.push(deep_read(vm, ep, e)?);
            }
            return Ok(vm.ctx.new_list(items).into());
        }
        if prop.cast_field::<ue::FStructProperty>().is_some() {
            return Ok(copy_struct_out(prop, data).into_pyobject(vm));
        }
        read_value(vm, prop, data)
    }

    fn find_set_element_index(
        vm: &VirtualMachine,
        set: &ue::FScriptSet,
        elem_prop: &ue::FProperty,
        layout: &ue::FScriptSetLayout,
        py_elem: PyObjectRef,
    ) -> PyResult<Option<i32>> {
        let elem_size = elem_prop.element_size() as usize;
        let mut buf: Vec<u8> = vec![0u8; elem_size];
        let buf_ptr = buf.as_mut_ptr() as *mut c_void;
        let result = unsafe {
            elem_prop.initialize_value(buf_ptr);
            match write_property(vm, elem_prop, buf_ptr, py_elem) {
                Err(e) => Err(e),
                Ok(()) => {
                    let _hash = elem_prop.get_value_type_hash(buf_ptr as *const c_void);
                    let mut found: Option<i32> = None;
                    for idx in 0..set.get_max_index() {
                        if !set.is_valid_index(idx) {
                            continue;
                        }
                        let elem_ptr = set.get_data(idx, layout) as *const c_void;
                        if elem_prop.identical(elem_ptr, buf_ptr as *const c_void) {
                            found = Some(idx);
                            break;
                        }
                    }
                    Ok(found)
                }
            }
        };
        unsafe { elem_prop.destroy_value(buf_ptr) };
        result
    }

    /// Deep-copy a struct value out of foreign memory into a Python-owned buffer.
    fn copy_struct_out(prop: &ue::FProperty, src: *const c_void) -> Struct {
        let sp = prop
            .cast_field::<ue::FStructProperty>()
            .expect("copy_struct_out called on a non-struct property");
        let struct_ty = sp.get_struct() as *const ue::UStruct;
        let size = prop.element_size() as usize;
        let mut buf: Box<[u8]> = vec![0u8; size].into_boxed_slice();
        let dst = buf.as_mut_ptr() as *mut c_void;
        unsafe {
            let init: FnStructOp = std::mem::transmute(vtable_fn(
                struct_ty as *const c_void,
                vtable_initialize_struct(),
            ));
            init(struct_ty, dst, 1);
            prop.copy_complete_value(dst, src);
        }
        let root = Rc::new(OwnedStructRoot {
            buf: std::cell::UnsafeCell::new(buf),
            struct_ty,
        });
        Struct {
            path: Path::owned(root, 0),
            struct_ty,
        }
    }

    fn read_at_path(vm: &VirtualMachine, path: Path) -> PyResult<PyObjectRef> {
        let (data, prop) = path.resolve(vm)?;
        if let Some(sp) = prop.cast_field::<ue::FStructProperty>() {
            let struct_ty = sp.get_struct() as *const ue::UStruct;
            return Ok(Struct { path, struct_ty }.into_pyobject(vm));
        }
        if let Some(ap) = prop.cast_field::<ue::FArrayProperty>() {
            let inner = ap.inner() as *const ue::FProperty;
            return Ok(Array { path, inner }.into_pyobject(vm));
        }
        if let Some(mp) = prop.cast_field::<ue::FMapProperty>() {
            return Ok(Map {
                path,
                key_prop: mp.key_property() as *const _,
                value_prop: mp.value_property() as *const _,
                layout: *mp.map_layout(),
            }
            .into_pyobject(vm));
        }
        if let Some(sp) = prop.cast_field::<ue::FSetProperty>() {
            return Ok(Set {
                path,
                element_prop: sp.element_property() as *const _,
                layout: *sp.set_layout(),
            }
            .into_pyobject(vm));
        }
        if prop.cast_field::<ue::FDelegateProperty>().is_some() {
            return Ok(Delegate { path }.into_pyobject(vm));
        }
        if prop
            .cast_field::<ue::FMulticastDelegateProperty>()
            .is_some()
        {
            return Ok(MulticastDelegate { path }.into_pyobject(vm));
        }
        read_value(vm, prop, data)
    }

    fn write_at_path(vm: &VirtualMachine, path: Path, value: PyObjectRef) -> PyResult<()> {
        let (data, prop) = path.resolve(vm)?;
        write_property(vm, prop, data, value)
    }

    fn write_array(
        vm: &VirtualMachine,
        ap: &ue::FArrayProperty,
        arr: &mut ue::FScriptArray,
        value: PyObjectRef,
    ) -> PyResult<()> {
        let inner = ap.inner();
        let elem_size = inner.element_size();
        let items = py_iter_collect(vm, &value)?;

        let num = arr.num();
        let base = arr.get_data_mut() as *mut u8;
        for i in 0..num {
            let slot = unsafe { base.add(i as usize * elem_size as usize) } as *mut c_void;
            unsafe { inner.destroy_value(slot) };
        }
        arr.empty(items.len() as i32, elem_size);

        for item in items {
            let idx = arr.add(1, elem_size);
            let base = arr.get_data_mut() as *mut u8;
            let slot = unsafe { base.add(idx as usize * elem_size as usize) } as *mut c_void;
            unsafe { inner.initialize_value(slot) };
            if let Err(e) = write_property(vm, inner, slot, item) {
                unsafe {
                    inner.destroy_value(slot);
                    arr.remove(idx, 1, elem_size);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    fn write_map(
        vm: &VirtualMachine,
        mp: &ue::FMapProperty,
        map: &mut ue::FScriptMap,
        value: PyObjectRef,
    ) -> PyResult<()> {
        let kp = mp.key_property();
        let vp = mp.value_property();
        let layout = mp.map_layout();
        let dict = value
            .downcast_ref::<PyDict>()
            .ok_or_else(|| type_err(vm, "expected a dict for map property"))?;

        for idx in 0..map.get_max_index() {
            if !map.is_valid_index(idx) {
                continue;
            }
            let pair = map.get_data_mut(idx, layout);
            let kptr = pair as *mut c_void;
            let vptr =
                unsafe { (pair as *mut u8).add(layout.value_offset as usize) } as *mut c_void;
            unsafe {
                kp.destroy_value(kptr);
                vp.destroy_value(vptr);
            }
        }
        map.empty(0, layout);

        for (k, v) in dict {
            let idx = map.add_uninitialized(layout);
            let pair = map.get_data_mut(idx, layout);
            let kptr = pair as *mut c_void;
            let vptr =
                unsafe { (pair as *mut u8).add(layout.value_offset as usize) } as *mut c_void;
            unsafe {
                kp.initialize_value(kptr);
                vp.initialize_value(vptr);
            }
            let r = write_property(vm, kp, kptr, k).and_then(|_| write_property(vm, vp, vptr, v));
            if let Err(e) = r {
                unsafe {
                    kp.destroy_value(kptr);
                    vp.destroy_value(vptr);
                }
                map.remove_at(idx, layout);
                return Err(e);
            }
        }
        map.rehash(layout, |k| unsafe { kp.get_value_type_hash(k) });
        Ok(())
    }

    fn write_set(
        vm: &VirtualMachine,
        sp: &ue::FSetProperty,
        set: &mut ue::FScriptSet,
        value: PyObjectRef,
    ) -> PyResult<()> {
        let ep = sp.element_property();
        let layout = sp.set_layout();
        let items = py_iter_collect(vm, &value)?;

        for idx in 0..set.get_max_index() {
            if !set.is_valid_index(idx) {
                continue;
            }
            let slot = set.get_data_mut(idx, layout) as *mut c_void;
            unsafe { ep.destroy_value(slot) };
        }
        set.empty(0, layout);

        for item in items {
            if find_set_element_index(vm, set, ep, layout, item.clone())?.is_some() {
                continue;
            }
            let idx = set.add_uninitialized(layout);
            let slot = set.get_data_mut(idx, layout) as *mut c_void;
            unsafe { ep.initialize_value(slot) };
            if let Err(e) = write_property(vm, ep, slot, item) {
                unsafe { ep.destroy_value(slot) };
                set.remove_at(idx, layout);
                return Err(e);
            }
            set.rehash(layout, |e| unsafe { ep.get_value_type_hash(e) });
        }
        Ok(())
    }

    fn write_property(
        vm: &VirtualMachine,
        prop: &ue::FProperty,
        data: *mut c_void,
        value: PyObjectRef,
    ) -> PyResult<()> {
        use ue::EClassCastFlags as F;
        let flags = prop.class().cast_flags();

        // Numeric properties coerce between Python int and float rather than
        // demanding an exact type match: a struct field can flip between int and
        // float across engine versions (e.g. FSlateFontInfo.Size became a float
        // in UE5), and call sites shouldn't have to care. A float written to an
        // integer property truncates toward zero — the same lossy narrowing
        // Unreal's own float→int Blueprint cast performs, and consistent with the
        // f64→f32 narrowing the float branch below already does silently. The
        // target width still range-checks after truncation.
        fn coerce_int<T>(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<T>
        where
            T: rustpython_vm::convert::TryFromObject,
        {
            if let Some(f) = value.downcast_ref::<PyFloat>() {
                let x = f.to_f64();
                if !x.is_finite() {
                    return Err(type_err(
                        vm,
                        "cannot assign a non-finite float to an integer property",
                    ));
                }
                let truncated: PyObjectRef = vm.ctx.new_int(x as i64).into();
                return truncated.try_into_value(vm);
            }
            value.clone().try_into_value(vm)
        }

        macro_rules! int_prim {
            ($T:ty) => {{
                let v: $T = coerce_int(&value, vm)?;
                unsafe { *(data as *mut $T) = v };
                return Ok(());
            }};
        }

        if let Some(bp) = prop.cast_field::<ue::FBoolProperty>() {
            let v: bool = value.try_to_value(vm)?;
            unsafe { bp.value_mut(data as *mut ()).set(v) };
            return Ok(());
        }
        if let Some(ep) = prop.cast_field::<ue::FEnumProperty>() {
            let underlying = if let Ok(s) = value.try_to_value::<String>(vm) {
                let en = ep
                    .enum_ptr()
                    .ok_or_else(|| type_err(vm, "enum property has no associated UEnum"))?;
                en.value_by_name_str(&s)
                    .ok_or_else(|| value_err(vm, format!("'{s}' is not a member of this enum")))?
            } else {
                value
                    .try_to_value::<i64>(vm)
                    .map_err(|_| type_err(vm, "expected enum entry name (str) or integer"))?
            };
            unsafe { ep.write_value(data as *mut (), underlying) };
            return Ok(());
        }
        if let Some(bp) = prop.cast_field::<ue::FByteProperty>() {
            let v: u8 = if let Some(en) = bp.enum_ptr() {
                if let Ok(s) = value.try_to_value::<String>(vm) {
                    en.value_by_name_str(&s).ok_or_else(|| {
                        value_err(vm, format!("'{s}' is not a member of this enum"))
                    })? as u8
                } else {
                    value
                        .try_to_value::<u8>(vm)
                        .map_err(|_| type_err(vm, "expected enum entry name (str) or u8"))?
                }
            } else {
                value.try_to_value::<u8>(vm)?
            };
            unsafe { *(data as *mut u8) = v };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FIntProperty) {
            int_prim!(i32)
        }
        if flags.contains(F::CASTCLASS_FInt64Property) {
            int_prim!(i64)
        }
        if flags.contains(F::CASTCLASS_FInt16Property) {
            int_prim!(i16)
        }
        if flags.contains(F::CASTCLASS_FInt8Property) {
            int_prim!(i8)
        }
        if flags.contains(F::CASTCLASS_FUInt64Property) {
            int_prim!(u64)
        }
        if flags.contains(F::CASTCLASS_FUInt32Property) {
            int_prim!(u32)
        }
        if flags.contains(F::CASTCLASS_FUInt16Property) {
            int_prim!(u16)
        }
        if flags.contains(F::CASTCLASS_FFloatProperty) {
            // `try_float` accepts a Python int as well as a float (via __float__),
            // so an int literal assigned to a float property just works.
            let v = value.try_float(vm)?.to_f64();
            unsafe { *(data as *mut f32) = v as f32 };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FDoubleProperty) {
            let v = value.try_float(vm)?.to_f64();
            unsafe { *(data as *mut f64) = v };
            return Ok(());
        }

        if flags.contains(F::CASTCLASS_FStrProperty) {
            let s: String = value.try_to_value(vm)?;
            let new = ue::FString::from(s.as_str());
            let slot = unsafe { &mut *(data as *mut ue::FString) };
            *slot = new;
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FNameProperty) {
            let s: String = value.try_to_value(vm)?;
            let name = ue::FName::from_string(&s, 1);
            unsafe { *(data as *mut ue::FName) = name };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FTextProperty) {
            let Some(t) = value.downcast_ref::<Text>() else {
                return Err(type_err(
                    vm,
                    "expected a ue.Text handle for FText property (str is not auto-converted)",
                ));
            };
            // `FText::operator=`: releases `data`'s current value if any — the
            // controller null-check makes this safe on a zeroed param slot too —
            // and adds a shared reference to ours (deep-copies the handle, not
            // the string).
            unsafe { prop.copy_complete_value(data, t.as_ptr()) };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FObjectProperty) {
            if vm.is_none(&value) {
                unsafe { *(data as *mut *const ue::UObjectBase) = std::ptr::null() };
                return Ok(());
            }
            // `UClass` shares `UObject`'s payload, so one downcast accepts both.
            let obj_ref: *const ue::UObjectBase = if let Some(u) = value.downcast_ref::<UObject>() {
                u.resolve(vm)? as *const _
            } else {
                return Err(type_err(
                    vm,
                    "expected UObject or UClass for object property",
                ));
            };
            unsafe { *(data as *mut *const ue::UObjectBase) = obj_ref };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FInterfaceProperty) {
            // An interface slot is an `FScriptInterface { UObject* ObjectPointer;
            // void* InterfacePointer; }`. Mirror UE's own assignment: store the object
            // and the address of its interface subobject (`GetInterfaceAddress`).
            let slot = data as *mut *const c_void;
            if vm.is_none(&value) {
                unsafe {
                    *slot = std::ptr::null();
                    *slot.add(1) = std::ptr::null();
                }
                return Ok(());
            }
            let Some(u) = value.downcast_ref::<UObject>() else {
                return Err(type_err(
                    vm,
                    "expected UObject/UClass or None for interface property",
                ));
            };
            let obj = u.resolve(vm)?;
            let iface = prop
                .cast_field::<ue::FInterfaceProperty>()
                .and_then(|ip| ip.interface_class())
                .ok_or_else(|| type_err(vm, "interface property has no interface class"))?;
            let iptr = obj
                .get_interface_address(iface as *const ue::UClass)
                .ok_or_else(|| {
                    type_err(
                        vm,
                        format!(
                            "{} does not implement interface {}",
                            obj.name_private(),
                            iface.name_private()
                        ),
                    )
                })?;
            unsafe {
                *slot = obj as *const ue::UObjectBase as *const c_void;
                *slot.add(1) = iptr;
            }
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FWeakObjectProperty) {
            let weak = if vm.is_none(&value) {
                ue::FWeakObjectPtr::default()
            } else if let Some(u) = value.downcast_ref::<UObject>() {
                ue::FWeakObjectPtr::new(u.resolve(vm)?)
            } else {
                return Err(type_err(
                    vm,
                    "expected UObject or UClass for weak object property",
                ));
            };
            unsafe { *(data as *mut ue::FWeakObjectPtr) = weak };
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FSoftObjectProperty) {
            let new_ptr = if vm.is_none(&value) {
                ue::FSoftObjectPtr::default()
            } else {
                let s: String = value.try_to_value(vm).map_err(|_| {
                    type_err(
                        vm,
                        "expected None or a path string for soft object property",
                    )
                })?;
                ue::FSoftObjectPtr::from_string(&s)
            };
            let slot = unsafe { &mut *(data as *mut ue::FSoftObjectPtr) };
            *slot = new_ptr;
            return Ok(());
        }
        if flags.contains(F::CASTCLASS_FLazyObjectProperty) {
            if vm.is_none(&value) {
                unsafe { *(data as *mut ue::FLazyObjectPtr) = ue::FLazyObjectPtr::default() };
                return Ok(());
            }
            return Err(type_err(
                vm,
                "write: FLazyObjectProperty only supports None (guid assignment not implemented)",
            ));
        }

        if let Some(sp) = prop.cast_field::<ue::FStructProperty>() {
            return write_struct(vm, prop, sp, data, value);
        }
        if let Some(ap) = prop.cast_field::<ue::FArrayProperty>() {
            let arr = unsafe { &mut *(data as *mut ue::FScriptArray) };
            return write_array(vm, ap, arr, value);
        }
        if let Some(mp) = prop.cast_field::<ue::FMapProperty>() {
            let map = unsafe { &mut *(data as *mut ue::FScriptMap) };
            return write_map(vm, mp, map, value);
        }
        if let Some(sp) = prop.cast_field::<ue::FSetProperty>() {
            let set = unsafe { &mut *(data as *mut ue::FScriptSet) };
            return write_set(vm, sp, set, value);
        }

        Err(type_err(
            vm,
            format!("write: unsupported property type {}", prop.class().name),
        ))
    }

    /// Write a struct value: from another `Struct` proxy (type-checked memcpy) or
    /// from a Python `dict` of field assignments.
    fn write_struct(
        vm: &VirtualMachine,
        prop: &ue::FProperty,
        sp: &ue::FStructProperty,
        data: *mut c_void,
        value: PyObjectRef,
    ) -> PyResult<()> {
        let ustruct = sp.get_struct();
        if let Some(other) = value.downcast_ref::<Struct>() {
            if other.struct_ty != ustruct as *const _ {
                return Err(type_err(
                    vm,
                    format!(
                        "struct type mismatch: target {}, got {}",
                        unsafe { &*(ustruct as *const ue::UStruct) }.name_private(),
                        unsafe { &*other.struct_ty }.name_private(),
                    ),
                ));
            }
            let (src_data, _) = other.data_and_struct(vm)?;
            // Copy through the struct's own copy semantics (`CopyScriptStruct`),
            // NOT a raw memcpy. A struct with a refcounted / heap-owning member
            // (an FText, FString, TArray, or a shared handle like
            // `FAssetDataTagMapSharedView`) must run that member's copy — which
            // AddRefs / clones — so the matching `destroy_value` the caller does
            // when tearing down the param frame (or overwriting the field) is
            // balanced. A shallow memcpy duplicates the handle without AddRef,
            // so the later destroy under-refs the shared owner: e.g. passing an
            // `FAssetData` to `GetTagValue` releases the AssetRegistry fixed-tag
            // `FStore` once per call, eventually freeing it and crashing at
            // shutdown in `~FAssetRegistryState`. Like the FText arm above, this
            // is `operator=` semantics: safe into a zeroed param slot (releases a
            // null current) and into a live field (releases the old value).
            unsafe { prop.copy_complete_value(data, src_data) };
            return Ok(());
        }
        let dict = value.downcast_ref::<PyDict>().ok_or_else(|| {
            type_err(
                vm,
                format!(
                    "expected dict for struct {} (got {})",
                    unsafe { &*(ustruct as *const ue::UStruct) }.name_private(),
                    value.class().name(),
                ),
            )
        })?;
        for (k, v) in dict {
            let key: String = k.try_to_value(vm)?;
            let target = ue::FName::find(&key);
            let inner_field = ustruct
                .iter_props()
                .find(|f| f.name() == target)
                .ok_or_else(|| {
                    attr_err(
                        vm,
                        format!(
                            "no field {key} on struct {}",
                            unsafe { &*(ustruct as *const ue::UStruct) }.name_private()
                        ),
                    )
                })?;
            let inner_prop = inner_field
                .cast::<ue::FProperty>()
                .ok_or_else(|| type_err(vm, format!("{key}: not an FProperty")))?;
            let inner_slot = unsafe { data.byte_add(inner_prop.offset() as usize) };
            write_property(vm, inner_prop, inner_slot, v)?;
        }
        Ok(())
    }

    // ── vtable slot offsets (engine-build specific; from `layout().vtable`) ──
    fn vtable_process_event() -> usize {
        ue::globals().layout.vtable.process_event
    }
    fn vtable_initialize_struct() -> usize {
        ue::globals().layout.vtable.initialize_struct
    }
    fn vtable_destroy_struct() -> usize {
        ue::globals().layout.vtable.destroy_struct
    }
    fn vtable_set_super_struct() -> usize {
        ue::globals().layout.vtable.set_super_struct
    }
    type FnSetSuperStruct = unsafe extern "system" fn(*const c_void, *const ue::UStruct);

    type FnProcessEvent =
        unsafe extern "system" fn(*mut ue::UObjectBase, *mut ue::UFunction, *mut c_void);
    type FnStructOp = unsafe extern "system" fn(*const ue::UStruct, *mut c_void, i32);

    unsafe fn vtable_fn(this: *const c_void, offset: usize) -> usize {
        let vtable = unsafe { *(this as *const *const u8) };
        unsafe { *(vtable.byte_add(offset) as *const usize) }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Delegates — dynamic (script) delegate bindings.
    // ──────────────────────────────────────────────────────────────────────

    fn resolve_delegate_target(
        vm: &VirtualMachine,
        target: &PyObject,
    ) -> PyResult<*const ue::UObjectBase> {
        // `UClass` shares `UObject`'s payload, so one downcast accepts both.
        if let Some(u) = target.downcast_ref::<UObject>() {
            Ok(u.resolve(vm)? as *const _)
        } else {
            Err(type_err(vm, "delegate target must be a UObject or UClass"))
        }
    }

    fn build_script_delegate(
        vm: &VirtualMachine,
        target: PyObjectRef,
        function_name: &str,
    ) -> PyResult<ue::FScriptDelegate> {
        let obj_ptr = resolve_delegate_target(vm, &target)?;
        let obj = unsafe { &*obj_ptr };
        let fname = ue::FName::find(function_name);
        if fname.comparison_index.value == 0 {
            return Err(value_err(
                vm,
                format!("no function named '{function_name}' (name not in the name table)"),
            ));
        }
        if obj.class().find_function(fname).is_none() {
            return Err(value_err(
                vm,
                format!(
                    "'{function_name}' is not a UFunction on {}",
                    obj.class().name_private()
                ),
            ));
        }
        Ok(ue::FScriptDelegate::new(obj, fname))
    }

    fn binding_to_py(vm: &VirtualMachine, d: &ue::FScriptDelegate) -> PyObjectRef {
        let obj = match d.object() {
            Some(o) => wrap_obj(vm, o),
            None => vm.ctx.none(),
        };
        (obj, d.function_name.to_string()).to_pyobject(vm)
    }

    #[pyattr]
    #[pyclass(name = "Delegate", module = "ue")]
    #[derive(PyPayload)]
    pub struct Delegate {
        path: Path,
    }

    impl Delegate {
        fn resolve(&self, vm: &VirtualMachine) -> PyResult<*mut ue::FScriptDelegate> {
            let (data, _prop) = self.path.resolve_as(
                vm,
                ue::EClassCastFlags::CASTCLASS_FDelegateProperty,
                "a delegate",
            )?;
            Ok(data as *mut ue::FScriptDelegate)
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl Delegate {
        #[pygetset]
        fn is_bound(&self, vm: &VirtualMachine) -> PyResult<bool> {
            Ok(unsafe { &*self.resolve(vm)? }.is_bound())
        }

        #[pygetset]
        fn target(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let d = unsafe { &*self.resolve(vm)? };
            if d.is_bound() {
                Ok(binding_to_py(vm, d))
            } else {
                Ok(vm.ctx.none())
            }
        }

        #[pymethod]
        fn bind(
            &self,
            target: PyObjectRef,
            function_name: String,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let new = build_script_delegate(vm, target, &function_name)?;
            unsafe { *self.resolve(vm)? = new };
            Ok(())
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            unsafe { *self.resolve(vm)? = ue::FScriptDelegate::unbound() };
            Ok(())
        }

        /// Bind this single-cast delegate to a Python callable. Returns a
        /// `DelegateBinding`; call `.unbind()` (or use it as a context manager)
        /// to detach and free the manufactured sink function.
        #[pymethod]
        fn bind_callable(
            &self,
            callback: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<DelegateBinding> {
            let (data, prop) = self.path.resolve_as(
                vm,
                ue::EClassCastFlags::CASTCLASS_FDelegateProperty,
                "a delegate",
            )?;
            let sig = prop
                .cast_field::<ue::FDelegateProperty>()
                .and_then(|dp| dp.signature_function());
            let (sink_obj, name, cookie, host) = register_callable(callback, sig, vm)?;
            unsafe {
                *(data as *mut ue::FScriptDelegate) = ue::FScriptDelegate::new(&*sink_obj, name)
            };
            Ok(DelegateBinding {
                cookie,
                sink_obj: sink_obj as usize,
                name,
                host,
                path: self.path.clone(),
                multicast: false,
            })
        }
    }
    impl Representable for Delegate {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.resolve(vm) {
                Ok(p) => {
                    let d = unsafe { &*p };
                    if d.is_bound() {
                        format!(
                            "<Delegate {}.{}>",
                            match d.object() {
                                Some(o) => o.name_private().to_string(),
                                None => "<stale>".into(),
                            },
                            d.function_name
                        )
                    } else {
                        "<Delegate unbound>".into()
                    }
                }
                Err(_) => "<Delegate stale>".into(),
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "MulticastDelegate", module = "ue")]
    #[derive(PyPayload)]
    pub struct MulticastDelegate {
        path: Path,
    }

    impl MulticastDelegate {
        fn resolve(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<(
            *mut c_void,
            &'static ue::FMulticastDelegateProperty,
            *mut ue::UObjectBase,
        )> {
            let (data, prop) = self.path.resolve_as(
                vm,
                ue::EClassCastFlags::CASTCLASS_FMulticastDelegateProperty,
                "a multicast delegate",
            )?;
            // resolve_as guaranteed the cast flag, so this can't fail.
            let mdp = prop.cast_field::<ue::FMulticastDelegateProperty>().unwrap();
            let parent = self.path.owner_object(vm)?;
            Ok((data, mdp, parent))
        }

        fn with_list<R>(
            &self,
            vm: &VirtualMachine,
            f: impl FnOnce(&[ue::FScriptDelegate]) -> R,
        ) -> PyResult<R> {
            let (data, mdp, _) = self.resolve(vm)?;
            let list = unsafe { mdp.get_multicast_delegate(data).as_ref() };
            Ok(match list {
                Some(l) => f(l.as_slice()),
                None => f(&[]),
            })
        }
    }

    #[pyclass(with(Constructor, AsSequence, Iterable, Representable))]
    impl MulticastDelegate {
        #[pygetset]
        fn bindings(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            self.with_list(vm, |l| l.iter().map(|b| binding_to_py(vm, b)).collect())
        }

        #[pymethod]
        fn add(
            &self,
            target: PyObjectRef,
            function_name: String,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let binding = build_script_delegate(vm, target, &function_name)?;
            let (data, mdp, parent) = self.resolve(vm)?;
            unsafe { mdp.add_delegate(binding, parent, data) };
            Ok(())
        }

        #[pymethod]
        fn remove(
            &self,
            target: PyObjectRef,
            function_name: String,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let binding = build_script_delegate(vm, target, &function_name)?;
            let (data, mdp, parent) = self.resolve(vm)?;
            unsafe { mdp.remove_delegate(&binding, parent, data) };
            Ok(())
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            let (data, mdp, parent) = self.resolve(vm)?;
            unsafe { mdp.clear_delegate(parent, data) };
            Ok(())
        }

        /// Add a Python callable to this multicast delegate's invocation list
        /// (e.g. `button.OnClicked.add_callable(fn)`). Returns a
        /// `DelegateBinding`; call `.unbind()` to remove it and free the sink.
        #[pymethod]
        fn add_callable(
            &self,
            callback: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<DelegateBinding> {
            let (data, mdp, parent) = self.resolve(vm)?;
            let sig = mdp.signature_function();
            let (sink_obj, name, cookie, host) = register_callable(callback, sig, vm)?;
            let binding = ue::FScriptDelegate::new(unsafe { &*sink_obj }, name);
            unsafe { mdp.add_delegate(binding, parent, data) };
            Ok(DelegateBinding {
                cookie,
                sink_obj: sink_obj as usize,
                name,
                host,
                path: self.path.clone(),
                multicast: true,
            })
        }
    }
    impl Representable for MulticastDelegate {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.with_list(vm, |l| l.len()) {
                Ok(n) => format!("<MulticastDelegate n={n}>"),
                Err(_) => "<MulticastDelegate stale>".into(),
            })
        }
    }
    impl Iterable for MulticastDelegate {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let bindings = zelf.bindings(vm)?;
            Ok(vm
                .ctx
                .new_list(bindings)
                .to_pyobject(vm)
                .get_iter(vm)?
                .into())
        }
    }

    impl AsSequence for MulticastDelegate {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, vm| {
                    MulticastDelegate::sequence_downcast(seq).with_list(vm, |l| l.len())
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Native sink table — one mechanism behind BOTH UFunction hooks and delegate
    // → Python bindings.
    //
    // Both features work by pointing a `UFunction::Func` pointer at native code
    // of ours and recovering, at call time, which Python callback it belongs to.
    // A single shared trampoline can't carry that identity (the call frame
    // doesn't reliably name our function — see `crate::trampoline`), so per
    // binding we emit a tiny runtime stub that tail-jumps to `sink_dispatch`
    // carrying a *cookie*: a stable `*const SinkHeader`. That removes the old
    // fixed-slot cap on both.
    //
    //  * A **hook** swaps an existing function's `Func` to our stub, remembering
    //    the original (in the header, so a worker-thread call can fall through).
    //  * A **delegate binding** manufactures a fresh empty `FUNC_Native`
    //    UFunction whose `Func` is our stub, registers it in a shared rooted sink
    //    object's class `FuncMap`, and binds the delegate to (sink_obj, name).
    //    On broadcast: FindFunctionChecked → our UFunction → ProcessEvent → stub.
    //
    // The registry is thread-local (the interpreter/game thread), like the tick
    // and begin-play registries; `SinkHeader`s are the only cross-thread part.
    // ──────────────────────────────────────────────────────────────────────

    use crate::trampoline::StubPool;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Copy, PartialEq)]
    enum SinkKind {
        Hook,
        Delegate,
    }

    /// The cross-thread-visible part of a binding, addressed by the cookie the
    /// stub carries. Heap-boxed so its address is stable while installed, and —
    /// like the old `ORIGINALS` array — **never freed**: a worker-thread
    /// invocation may still be reading it after the binding is torn down. Leaking
    /// it (tens of bytes) is the price of not needing a teardown barrier.
    #[repr(C)]
    struct SinkHeader {
        // All fields below `original` are written once at construction, before
        // the stub is ever installed (the install is the publication point), and
        // only read thereafter — so a worker-thread call that sees the swapped
        // `Func` also sees a fully-populated header.
        kind: SinkKind,
        // Hooks: the original `Func` to fall through to (read off-thread too).
        // Stays published after unhook — it's the engine's own ProcessInternal /
        // execXxx thunk, valid for the function's whole lifetime — so a worker
        // thread that loaded the stub's address just before the restore still
        // falls through transparently instead of no-opping the call. Delegates:
        // always 0 (a manufactured sink has no original — off-thread it's a
        // harmless no-op).
        original: AtomicUsize,
        // Hooks only (0 / false for delegates). The off-thread fallthrough runs
        // without the game-thread-local registry, so the identity it needs to
        // rebuild a script frame for `ProcessInternal` lives here:
        //   func_ptr   — the hooked UFunction.
        //   was_native — whether it was already `FUNC_Native` before we hooked it
        //                (if so `original` is an `execXxx` thunk that takes the
        //                caller's frame as-is; if not, it's `ProcessInternal` and
        //                needs a reconstructed script frame).
        func_ptr: usize,
        was_native: bool,
    }

    /// One binding's game-thread state, keyed in the table by cookie
    /// (`&*header as usize`).
    struct Sink {
        header: Box<SinkHeader>,
        callback: PyObjectRef,
        detail: Detail,
    }

    enum Detail {
        Hook {
            target: ue::FWeakObjectPtr, // the hooked UFunction (identity / liveness)
            func_addr: usize,           // same, raw, for the frame.node() compare
            orig_flags: u32,            // FunctionFlags to restore (undo forced FUNC_Native)
        },
        Delegate {
            sink_func: ue::FWeakObjectPtr, // manufactured UFunction, rooted; unrooted on unbind
            sig: ue::FWeakObjectPtr,       // SignatureFunction (arg layout); null if 0-param
        },
    }

    struct SinkTable {
        sinks: HashMap<usize, Sink>,
        stubs: StubPool,
    }

    /// All Python-facing registry state, in one game-thread thread-local.
    ///
    /// Layout: each domain gets its **own** cell because they have independent
    /// reentrancy patterns — a hook callback may register a tick, a begin-play
    /// callback may install a delegate binding, `ensure_delegate_host` runs
    /// while a `sinks` borrow is held, and the trace handler fires between any
    /// two of these. One shared `RefCell` would turn those legal nestings into
    /// borrow panics; separate cells keep each borrow scoped to its own domain.
    ///
    /// Drop policy: the whole struct is `ManuallyDrop`, *never* torn down. This
    /// is a DLL injected into the game, so the game thread's TLS destructors
    /// only ever run at process shutdown — where dropping the held
    /// `PyObjectRef`s (Rc) re-enters the interpreter's own thread-local
    /// (`interp::STATE`, itself never dropped for the same reason) after it may
    /// already be destroyed: the classic "cannot access a TLS value during or
    /// after destruction" abort. The OS reclaims the memory anyway. Every
    /// game-thread `thread_local!` in this DLL follows this policy (see also
    /// `server::SERVER`).
    ///
    /// Note the trace *handler* can touch this TLS from worker threads (any
    /// thread running BP bytecode); they lazily get their own empty instance
    /// and find `trace` unset, which is exactly the "don't record off-thread"
    /// behavior wanted. Nothing else reads registry state off the game thread —
    /// the cross-thread halves of hooking live in `SinkHeader`/`SAVED_NATIVES`.
    struct Registries {
        /// Unified hook + delegate-binding table (keyed by stub cookie).
        sinks: RefCell<SinkTable>,
        /// Shared delegate sink object + its class, created lazily. Held as
        /// `FWeakObjectPtr`s (index+serial), NOT raw pointers, so a cache hit can
        /// detect a host that was garbage-collected out from under us (e.g. on a
        /// UE5 level transition, where a manufactured class is collected if
        /// engine rooting didn't take) and rebuild instead of dereferencing a
        /// dangling pointer. `(sink, class)`. A `Cell` (no borrow) so
        /// `ensure_delegate_host` can run while a `sinks` borrow is held.
        delegate_host: Cell<Option<(ue::FWeakObjectPtr, ue::FWeakObjectPtr)>>,
        begin_play: RefCell<Vec<BeginPlayReg>>,
        tick: RefCell<Vec<TickReg>>,
        trace: RefCell<Option<TraceState>>,
        /// One counter for every registration id / generated sink name. Ids only
        /// need to be unique, not dense or per-domain.
        next_id: Cell<u64>,
    }

    thread_local! {
        static REGISTRIES: std::mem::ManuallyDrop<Registries> =
            std::mem::ManuallyDrop::new(Registries {
                sinks: RefCell::new(SinkTable {
                    sinks: HashMap::new(),
                    stubs: StubPool::new(),
                }),
                delegate_host: Cell::new(None),
                begin_play: RefCell::new(Vec::new()),
                tick: RefCell::new(Vec::new()),
                trace: RefCell::new(None),
                next_id: Cell::new(1),
            });
    }

    fn with_sinks<R>(f: impl FnOnce(&mut SinkTable) -> R) -> R {
        REGISTRIES.with(|r| f(&mut r.sinks.borrow_mut()))
    }

    fn with_begin_play<R>(f: impl FnOnce(&RefCell<Vec<BeginPlayReg>>) -> R) -> R {
        REGISTRIES.with(|r| f(&r.begin_play))
    }

    fn with_tick<R>(f: impl FnOnce(&RefCell<Vec<TickReg>>) -> R) -> R {
        REGISTRIES.with(|r| f(&r.tick))
    }

    fn with_trace<R>(f: impl FnOnce(&RefCell<Option<TraceState>>) -> R) -> R {
        REGISTRIES.with(|r| f(&r.trace))
    }

    fn next_reg_id() -> u64 {
        REGISTRIES.with(|r| {
            let id = r.next_id.get();
            r.next_id.set(id + 1);
            id
        })
    }

    /// Allocate a leaked `SinkHeader` + a runtime stub for a new binding. Returns
    /// the boxed header (to move into the `Sink`), its cookie, and the stub addr
    /// to install as a `Func`.
    fn new_stub(
        t: &mut SinkTable,
        kind: SinkKind,
        original: usize,
    ) -> (Box<SinkHeader>, usize, *mut u8) {
        let header = Box::new(SinkHeader {
            kind,
            original: AtomicUsize::new(original),
            func_ptr: 0,
            was_native: false,
        });
        let cookie = &*header as *const SinkHeader as usize;
        let stub = t.stubs.alloc(cookie, sink_dispatch as *const () as usize);
        (header, cookie, stub)
    }

    /// Drop a binding's game-thread state. The header *and* the stub are leaked
    /// (see `SinkHeader` / [`StubPool`]): a worker thread that read the target's
    /// `Func` just before teardown can still enter the stub and read the header
    /// arbitrarily late. For delegate sinks, unroots the manufactured UFunction.
    /// The caller must have *already* made the stub unreachable for new calls
    /// (restored the original `Func` / removed the manufactured function from the
    /// `FuncMap`).
    fn free_sink(t: &mut SinkTable, cookie: usize) {
        if let Some(sink) = t.sinks.remove(&cookie) {
            if let Detail::Delegate { sink_func, .. } = &sink.detail {
                if let Some(f) = sink_func.get_even_if_pending_kill() {
                    f.remove_from_root();
                }
            }
            let _ = Box::leak(sink.header);
        }
    }

    /// The single native entry point every stub tail-jumps to. `cookie` is a
    /// `*const SinkHeader` identifying the binding; `context`/`stack`/`result`
    /// are the native `Func` arguments.
    unsafe extern "system" fn sink_dispatch(
        context: *mut ue::UObject,
        stack: *mut ue::kismet::FFrame,
        result: *mut c_void,
        cookie: usize,
    ) {
        let hdr = unsafe { &*(cookie as *const SinkHeader) };
        match hdr.kind {
            SinkKind::Delegate => {
                // Broadcasts always arrive via ProcessEvent, so `stack.node()` is
                // our manufactured function. There's no original to fall through
                // to; off the interpreter thread this is simply a no-op.
                delegate_dispatch(cookie, stack);
            }
            SinkKind::Hook => {
                let original = hdr.original.load(Ordering::Acquire);
                let ran = hook_dispatch(cookie, context, stack, result, original);
                // Off the interpreter thread (BP VM on a worker — e.g. a
                // multithreaded AnimBP evaluating its graph): we can't touch
                // RustPython, so behave transparently and run the original. Doing
                // nothing would leave the VM frame un-stepped and the interpreter
                // would run the parameter bytecode as garbage opcodes.
                if !ran && original != 0 {
                    hook_fallthrough(hdr, context, stack, result, original);
                }
            }
        }
    }

    fn delegate_dispatch(cookie: usize, stack: *mut ue::kismet::FFrame) {
        crate::interp::with_vm(|vm| {
            let Some((cb, sig)) = with_sinks(|t| {
                t.sinks.get(&cookie).and_then(|s| match &s.detail {
                    Detail::Delegate { sig, .. } => Some((s.callback.clone(), *sig)),
                    _ => None,
                })
            }) else {
                return;
            };
            // Marshal the broadcast's arguments out of the call frame. The `guard`
            // must outlive the callback: struct/array args are returned as proxies
            // that read the frame lazily, and are invalidated once it's dropped.
            let guard = Rc::new(Cell::new(true));
            let args = match build_delegate_args(vm, sig, stack, &guard) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("delegate arg marshalling failed");
                    vm.print_exception(e);
                    guard.set(false);
                    return;
                }
            };
            let res = cb.call(args, vm);
            guard.set(false);
            if let Err(e) = res {
                tracing::error!("delegate callback failed");
                vm.print_exception(e);
            }
        });
    }

    /// Read a broadcast's in-args out of the call frame as Python objects, in
    /// signature order, using the delegate's SignatureFunction property layout
    /// (`ProcessEvent` memcpy'd the arg block into the frame at those offsets).
    /// Empty for a 0-param delegate. Mirrors `HookParams::args`.
    fn build_delegate_args(
        vm: &VirtualMachine,
        sig: ue::FWeakObjectPtr,
        stack: *mut ue::kismet::FFrame,
        guard: &Rc<Cell<bool>>,
    ) -> PyResult<Vec<PyObjectRef>> {
        let Some(obj) = sig.get_even_if_pending_kill() else {
            return Ok(Vec::new());
        };
        let Some(ufunc) = obj.cast::<ue::UFunction>() else {
            return Ok(Vec::new());
        };
        if stack.is_null() {
            return Ok(Vec::new());
        }
        let locals = unsafe { &*stack }.locals() as usize;
        if locals == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for p in ufunc
            .params()
            .filter(|p| !p.property_flags().contains(ue::EPropertyFlags::CPF_OutParm))
        {
            let addr = locals + p.offset() as usize;
            let prop: &'static ue::FProperty =
                unsafe { std::mem::transmute::<&ue::FProperty, &'static _>(p) };
            out.push(read_at_path(
                vm,
                Path::frame(addr, prop as *const _, guard.clone()),
            )?);
        }
        Ok(out)
    }

    fn resolve_class_by_path(vm: &VirtualMachine, path: &str) -> PyResult<*const ue::UClass> {
        ue::hash_tables()
            .find_object_by_path(path)
            .and_then(|o| o.cast::<ue::UClass>())
            .map(|c| c as *const ue::UClass)
            .ok_or_else(|| runtime_err(vm, format!("could not resolve class {path}")))
    }

    /// The `/Engine/Transient` package — the conventional outer for runtime-only
    /// objects. The manufactured host class (and its instances/CDO) must have a
    /// real package outer, not null: a null-outer `UClass` is its own outermost
    /// "package", which DRG's asset-registry / UGC tag machinery walks and
    /// mis-registers, double-freeing an interned tag handle when the registry
    /// resets at shutdown. Parenting into the transient package (excluded from
    /// asset enumeration by its package flags) keeps our class out of that path.
    fn transient_package(vm: &VirtualMachine) -> PyResult<*const ue::UObjectBase> {
        ue::hash_tables()
            .find_object_by_path("/Engine/Transient")
            .map(|o| o as *const ue::UObjectBase)
            .ok_or_else(|| runtime_err(vm, "could not resolve /Engine/Transient package"))
    }

    /// The shared, rooted sink object and its **private** host class (created
    /// once).
    ///
    /// The host is a UClass we manufacture ourselves — *not* the shared
    /// `UObject::StaticClass()`. All manufactured sink UFunctions land in this
    /// private class's FuncMap, so injecting them never mutates a live engine
    /// class and their names are unreachable from any other object's
    /// `FindFunction`. The single sink object is constructed *directly* as an
    /// instance of the host class, so `sink->GetClass()` (delegate dispatch's
    /// resolution root) is the isolated class with no re-parenting. The host
    /// derives from `UObject` and adds no properties, so instances share
    /// `UObject`'s exact layout — which is what keeps the copied GC reference
    /// data (see step 1) valid for them.
    fn ensure_delegate_host(
        vm: &VirtualMachine,
    ) -> PyResult<(*const ue::UObjectBase, &'static ue::UClass)> {
        if let Some((sink_w, host_w)) = REGISTRIES.with(|r| r.delegate_host.get()) {
            // Only reuse the cache if BOTH objects are still live. `FWeakObjectPtr::get`
            // returns None once the slot's serial number moves on (i.e. the object
            // was destroyed), so a stale host from a prior GC is detected here
            // rather than handed back as a dangling pointer.
            if let (Some(sink), Some(host)) = (sink_w.get(), host_w.get()) {
                if host.is::<ue::UClass>() {
                    let cls = host as *const _ as *const ue::UClass;
                    return Ok((sink as *const ue::UObjectBase, unsafe { &*cls }));
                }
            }
            // Stale (collected) — drop it and fall through to rebuild a fresh host.
            REGISTRIES.with(|r| r.delegate_host.set(None));
        }
        // Template class both the host and the sink derive from: plain UObject.
        let obj_class = resolve_class_by_path(vm, "/Script/CoreUObject.Object")?;
        let flags = (ue::EObjectFlags::RF_Public | ue::EObjectFlags::RF_Transient).bits();
        // Real package outer (not null) — see `transient_package`.
        let outer = transient_package(vm)?;

        // 1. Fabricate the dedicated host UClass (an instance of the `Class`
        //    meta-class) and make it a *real, instantiable* class rather than a
        //    frankenclass we mutate after the fact. Two things drove this design:
        //    GC traverses the class through its reference-token stream, and the
        //    object-hash tables assert add/remove symmetry at teardown. So we
        //    copy from a fully-formed template (`UObject`'s class) everything the
        //    engine needs to construct instances and trace them:
        //      * ClassCastFlags        → `Cast<>` classifies instances.
        //      * ClassWithin + ClassAddReferencedObjects → GC's
        //        `UClass::AddReferencedObjects(host)` hits valid pointers.
        //      * ClassConstructor      → the engine can placement-construct
        //        instances (and the CDO) — a bare `UObject` constructor.
        //      * PropertiesSize + MinAlignment → `StaticAllocateObject` sizes and
        //        aligns instances correctly.
        //      * ReferenceTokenStream (deep-copied into host's own gmalloc buffer)
        //        + CLASS_TokenStreamAssembled → GC's parallel collector reads
        //        `Tokens[0]` unconditionally; UObject's stream (ClassPrivate +
        //        OuterPrivate + package + EndOfStream) is valid for host
        //        instances (host adds no properties, so they share UObject's
        //        layout), and the flag stops the engine rebuilding/freeing it.
        //    The super link is set *afterward* through the engine's
        //    `UClass::SetSuperStruct` (not a raw write) so it un-hashes,
        //    rebuilds the fast-`IsChildOf` base chain, and re-hashes — keeping
        //    `ClassToChildListMap` symmetric so teardown's `RemoveFromClassMap`
        //    doesn't fatal. CLASS_Abstract is cleared so instances can be made.
        let meta_class = resolve_class_by_path(vm, "/Script/CoreUObject.Class")?;
        let host_name = ue::FName::from_string("PyDelegateHostClass", 1); // FNAME_Add
        // Root at construction via `InternalSetFlags = RootSet`: on UE5.4+ this
        // routes through `StaticAllocateObject` → `FUObjectItem::SetRootFlags`,
        // which inserts into the `GRoots` `TSet` under its lock. A plain
        // `add_to_root()` after the fact only ORs the flag and would leave the
        // object out of `GRoots` (collected on the next GC). Rooting here is
        // safe despite the class not being fully assembled yet — construction is
        // game-thread-synchronous, so no GC traces it before setup completes.
        let root = ue::EInternalObjectFlags::RootSet.bits() as u32;
        let host =
            unsafe { ue::static_construct_object(meta_class, outer, host_name, flags, root) }
                .ok_or_else(|| runtime_err(vm, "failed to construct delegate host class"))?
                .cast::<ue::UClass>()
                .ok_or_else(|| runtime_err(vm, "constructed host object is not a UClass"))?;
        // SAFETY (handle_mut): game thread; the class is freshly constructed
        // and not yet reachable by any VM lookup or reflection walk, so we are
        // its only accessor until the sink instantiation below makes it live.
        // SAFETY (deref of `obj_class`): a live, permanent engine class, so
        // the fields we read off it (and copy) stay valid.
        let host_mut = unsafe { host.handle_mut() };
        let template = unsafe { &*obj_class };
        host_mut
            .class_cast_flags_mut()
            .set(template.class_cast_flags());
        host_mut.class_within_mut().set(template.class_within());
        host_mut
            .class_add_referenced_objects_mut()
            .set(template.class_add_referenced_objects());
        host_mut
            .class_constructor_mut()
            .set(template.class_constructor());
        host_mut
            .properties_size_mut()
            .set(template.properties_size());
        host_mut.min_alignment_mut().set(template.min_alignment());
        // Give host valid GC reference data, THEN mark assembled (and clear
        // CLASS_Abstract). Order matters: the data must be valid before the flag
        // makes the engine trust it. The representation is engine-generation-
        // specific: UE4 deep-copies UObject's `TArray<uint32>` token stream into
        // host's own buffer (false if UObject's is empty — fail loudly rather
        // than relocate the GC crash); UE5 uses a schema, and host's own empty
        // schema is already correct for a UObject-layout instance, so there's
        // nothing to copy (false only if we're somehow not on a schema build).
        let stream_ok = match ue::globals().layout.gc_reference_format {
            ue::GcReferenceFormat::Ue4TokenStream => {
                host_mut.copy_reference_token_stream_from(template)
            }
            ue::GcReferenceFormat::Ue5Schema => host_mut.reference_schema_already_valid(),
        };
        let class_flags = (host_mut.class_flags() | ue::EClassFlags::CLASS_TokenStreamAssembled)
            .difference(ue::EClassFlags::CLASS_Abstract);
        host_mut.class_flags_mut().set(class_flags);
        if !stream_ok {
            return Err(runtime_err(
                vm,
                "UObject class has no assembled GC reference data (token stream / schema) to copy",
            ));
        }
        // Already rooted at construction (InternalSetFlags = RootSet above).
        // Set the super link through the engine so the class-hash tables stay
        // symmetric (see `set_super_struct` vtable doc). Must precede any
        // instantiation of `host`: its CDO construction reads GetSuperClass().
        // SAFETY: game thread; `host` is a live UClass whose vtable is UClass's,
        // so slot `set_super_struct` resolves to `UClass::SetSuperStruct`.
        unsafe {
            let f: FnSetSuperStruct = std::mem::transmute(vtable_fn(
                host as *const _ as *const c_void,
                vtable_set_super_struct(),
            ));
            f(
                host as *const _ as *const c_void,
                obj_class as *const ue::UStruct,
            );
        }

        // 2. Construct the sink as a genuine `host` instance. The engine hashes
        //    it under `host` (symmetric teardown) and, on first construction,
        //    builds host's own CDO via `CreateDefaultObject`/`StaticLink` — no
        //    re-parenting, no shared CDO.
        let sink_name = ue::FName::from_string("PyDelegateSinkObject", 1); // FNAME_Add
        let obj = unsafe {
            ue::static_construct_object(host as *const ue::UClass, outer, sink_name, flags, root)
        }
        .ok_or_else(|| runtime_err(vm, "failed to construct delegate sink object"))?;
        // Rooted at construction (InternalSetFlags = RootSet) so it lands in GRoots.

        // Cache as weak pointers so a later GC of the host is detected on reuse.
        let host_base: &ue::UObjectBase = host;
        REGISTRIES.with(|r| {
            r.delegate_host.set(Some((
                ue::FWeakObjectPtr::new(obj),
                ue::FWeakObjectPtr::new(host_base),
            )))
        });
        Ok((obj as *const ue::UObjectBase, host))
    }

    /// Manufacture one empty native sink UFunction in `host`'s FuncMap, wired to
    /// `func_addr` (a per-binding stub, or 0 in the self-test — it's never
    /// invoked there). Returns the function and its FuncMap key.
    fn manufacture_sink(
        host: &ue::UClass,
        func_addr: usize,
        sig: Option<&ue::UFunction>,
        vm: &VirtualMachine,
    ) -> PyResult<(*const ue::UFunction, ue::FName)> {
        let func_class = resolve_class_by_path(vm, "/Script/CoreUObject.Function")?;
        let name = ue::FName::from_string(&format!("PyDelegateSink_{}", next_reg_id()), 1);
        let host_base: &ue::UObjectBase = host;
        let flags = (ue::EObjectFlags::RF_Public | ue::EObjectFlags::RF_Transient).bits();
        // Root at construction (InternalSetFlags = RootSet) → inserted into GRoots
        // on UE5.4+. NOTE: `free_sink`/`unbind` only clears the flag via
        // `remove_from_root()`, so on 5.4+ the index is leaked in GRoots (the
        // function is never collected) until `ClearRootFlags` is resolved and
        // `remove_from_root` routed through it. Accepted for now.
        let root = ue::EInternalObjectFlags::RootSet.bits() as u32;
        let obj = unsafe { ue::static_construct_object(func_class, host_base, name, flags, root) }
            .ok_or_else(|| runtime_err(vm, "failed to construct sink UFunction"))?;
        // Checked downcast: the object was constructed with the Function class, so
        // this always succeeds — but it turns a mis-resolved layout into an error
        // instead of a garbage reinterpret.
        let f = obj
            .cast::<ue::UFunction>()
            .ok_or_else(|| runtime_err(vm, "constructed sink object is not a UFunction"))?;
        // The UFunction constructor already left ParmsSize/PropertiesSize = 0,
        // ChildProperties null, Script empty, ReturnValueOffset = MAX_uint16.
        // We assert the calling convention: native + public + final, no Net (so
        // ProcessEvent stays local) and no HasOutParms.
        //
        // `function_flags`/`func` are racy fields, so the atomic place is their
        // one idiom; on this freshly-constructed, not-yet-reachable function
        // the stores race nothing. SAFETY: the values are a valid calling
        // convention and stub.
        use ue::EFunctionFlags as FF;
        unsafe {
            f.function_flags_atomic()
                .store(FF::FUNC_Native | FF::FUNC_Public | FF::FUNC_Final);
            f.native_func_addr_atomic().store(func_addr);
        }
        // Size the frame like the delegate signature so ProcessEvent memcpys
        // the broadcast args into it. We copy only the sizes, never the
        // property chain (see copy_param_sizes) — the trampoline reads the
        // args against `sig`'s own properties. No-op for a 0-param delegate.
        // SAFETY (handle_mut): freshly constructed, not yet in any FuncMap.
        if let Some(sig) = sig {
            unsafe { f.handle_mut() }.copy_param_sizes(sig);
        }
        let func = f as *const ue::UFunction;
        // Already rooted at construction (InternalSetFlags = RootSet above).
        // SAFETY (handle_mut): game thread, no other Rust borrow of the host's
        // FuncMap is live here, and the host is our private sink class — the
        // engine only resolves its functions from game-thread delegate dispatch.
        unsafe { host.handle_mut() }.add_function_to_func_map(name, func);
        Ok((func, name))
    }

    /// Manufacture + register a delegate callback; returns (sinkObj, funcName,
    /// cookie, host). `sig` is the delegate's SignatureFunction, whose param
    /// layout the sink adopts so the callback receives the broadcast's arguments.
    fn register_callable(
        callback: PyObjectRef,
        sig: Option<&ue::UFunction>,
        vm: &VirtualMachine,
    ) -> PyResult<(*const ue::UObjectBase, ue::FName, usize, usize)> {
        if !callback.is_callable() {
            return Err(type_err(vm, "callback must be callable"));
        }
        // `ensure_delegate_host` uses its own cell, so this is safe to call before
        // (and the manufacture below, which never touches the sink table, safe
        // within) the `with_sinks` borrow.
        let (sink_obj, host) = ensure_delegate_host(vm)?;
        let sig_weak = match sig {
            Some(s) => ue::FWeakObjectPtr::new(s),
            None => ue::FWeakObjectPtr::default(),
        };
        let host_addr = host as *const ue::UClass as usize;
        with_sinks(|t| {
            let (header, cookie, stub) = new_stub(t, SinkKind::Delegate, 0);
            let (func, name) = manufacture_sink(host, stub as usize, sig, vm)?;
            let sink_func = ue::FWeakObjectPtr::new(unsafe { &*(func as *const ue::UObjectBase) });
            t.sinks.insert(
                cookie,
                Sink {
                    header,
                    callback,
                    detail: Detail::Delegate {
                        sink_func,
                        sig: sig_weak,
                    },
                },
            );
            Ok((sink_obj, name, cookie, host_addr))
        })
    }

    #[pyattr]
    #[pyclass(name = "DelegateBinding", module = "ue")]
    #[derive(PyPayload)]
    pub struct DelegateBinding {
        cookie: usize,
        sink_obj: usize,
        name: ue::FName,
        host: usize,
        // Where the binding lives, so `unbind` can re-resolve and detach it.
        path: Path,
        multicast: bool,
    }

    #[pyclass(with(Constructor, Representable))]
    impl DelegateBinding {
        /// Detach this callback: remove the binding from the delegate, drop the
        /// manufactured function from the FuncMap and the GC root set, and free
        /// the sink. Idempotent.
        #[pymethod]
        fn unbind(&self, vm: &VirtualMachine) -> PyResult<()> {
            let sink = unsafe { (self.sink_obj as *const ue::UObjectBase).as_ref() };
            if self.multicast {
                let md = MulticastDelegate {
                    path: self.path.clone(),
                };
                if let (Some(sink), Ok((data, mdp, parent))) = (sink, md.resolve(vm)) {
                    let binding = ue::FScriptDelegate::new(sink, self.name);
                    unsafe { mdp.remove_delegate(&binding, parent, data) };
                }
            } else {
                let d = Delegate {
                    path: self.path.clone(),
                };
                if let Ok(p) = d.resolve(vm) {
                    // Only clear if it's still our binding (don't stomp a rebind).
                    let cur = unsafe { &*p };
                    if cur.function_name == self.name {
                        unsafe { *p = ue::FScriptDelegate::unbound() };
                    }
                }
            }
            // Remove the manufactured function so the engine can no longer resolve
            // it, then drop our registry entry (unroots the function, recycles the
            // stub). Order matters: FuncMap-removed first, then free.
            if let Some(host) = unsafe { (self.host as *const ue::UClass).as_ref() } {
                // SAFETY (handle_mut): as in `manufacture_sink` — game thread,
                // no live FuncMap borrow, private sink class.
                unsafe { host.handle_mut() }.remove_function_from_func_map(&self.name);
            }
            with_sinks(|t| free_sink(t, self.cookie));
            Ok(())
        }
        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
            self.unbind(vm)?;
            Ok(false)
        }
    }
    impl Representable for DelegateBinding {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let active = with_sinks(|t| t.sinks.contains_key(&zelf.cookie));
            Ok(format!(
                "<DelegateBinding {:#x} active={}>",
                zelf.cookie, active
            ))
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // UFunction hooking. A hook swaps a function's `Func` to a per-binding stub
    // (registered in the shared sink table above) and — for BP-script functions
    // — forces `FUNC_Native` so the Blueprint VM routes dispatch through `Func`.
    // The saved original lives in the binding's `SinkHeader` (readable from any
    // thread), so a UFunction invoked by the Blueprint VM on a worker thread (e.g.
    // async animation/physics calling `VSize`) still falls through to it — we
    // can't run the Python callback there, but we MUST call the original or the VM
    // frame goes un-stepped and the interpreter runs the parameter bytecode as
    // garbage opcodes.
    // ──────────────────────────────────────────────────────────────────────

    /// The cookie of a live hook on `target` (the hooked UFunction's weak
    /// pointer), if any.
    fn find_live_hook(t: &SinkTable, target: &ue::FWeakObjectPtr) -> Option<usize> {
        t.sinks.iter().find_map(|(&cookie, s)| match &s.detail {
            Detail::Hook { target: tgt, .. }
                if tgt.get_even_if_pending_kill().is_some() && tgt == target =>
            {
                Some(cookie)
            }
            _ => None,
        })
    }

    /// Drop hook entries whose target UFunction has been GC'd. There's nothing to
    /// restore (the function is gone), so just drop the entry.
    fn reclaim_dead_hooks(t: &mut SinkTable) {
        let dead: Vec<usize> = t
            .sinks
            .iter()
            .filter_map(|(&cookie, s)| match &s.detail {
                Detail::Hook { target, .. } if target.get_even_if_pending_kill().is_none() => {
                    Some(cookie)
                }
                _ => None,
            })
            .collect();
        for cookie in dead {
            free_sink(t, cookie);
        }
    }

    /// The hook half of `sink_dispatch`: run the Python callback on the
    /// interpreter thread and report whether it ran (the caller falls through to
    /// `original` otherwise). `original` is the saved `Func`, from the header.
    fn hook_dispatch(
        cookie: usize,
        context: *mut ue::UObject,
        stack: *mut ue::kismet::FFrame,
        result: *mut c_void,
        original: usize,
    ) -> bool {
        let orig: ue::FNativeFuncPtr = unsafe { std::mem::transmute(original) };

        // Run the Python hook only on the interpreter's own thread. `with_vm`
        // returns None off that thread (its `STATE` is thread-local), and the
        // table lookup yields None if this cookie isn't a hook registered here.
        crate::interp::with_vm(|vm| {
            let Some((func_ptr, callback, was_native)) = with_sinks(|t| {
                t.sinks.get(&cookie).and_then(|s| match &s.detail {
                    Detail::Hook {
                        func_addr,
                        orig_flags,
                        ..
                    } => {
                        // Whether the target was *originally* native (before we may
                        // have forced `FUNC_Native` on to reroute dispatch). This
                        // tells `original()` whether the saved `Func` is an
                        // `execXxx` thunk (native) or `ProcessInternal` (BP script).
                        let was_native = ue::EFunctionFlags::from_bits_truncate(*orig_flags)
                            .contains(ue::EFunctionFlags::FUNC_Native);
                        Some((*func_addr, s.callback.clone(), was_native))
                    }
                    _ => None,
                })
            }) else {
                return false;
            };
            let frame = unsafe { &mut *stack };
            if frame.node() as usize == func_ptr {
                run_hook(
                    vm,
                    &callback,
                    context,
                    stack as usize,
                    result,
                    func_ptr,
                    original,
                    orig,
                    was_native,
                    frame.locals() as usize,
                    build_out_addrs(frame.out_parms() as usize),
                );
            } else {
                run_bytecode_hook(
                    vm, &callback, context, frame, result, func_ptr, original, orig, was_native,
                );
            }
            true
        })
        .unwrap_or(false)
    }

    /// Transparent pass-through to a hook's original `Func`, used when the Python
    /// callback couldn't run (off the interpreter thread, or a torn-down cookie).
    /// The tricky case is a BP-script function we force-flipped to `FUNC_Native`,
    /// reached from bytecode on a worker thread: its `original` is
    /// `ProcessInternal`, which needs a proper script frame, so we rebuild one
    /// exactly like the game-thread bytecode path instead of handing it the
    /// caller's frame raw (an access violation — the multithreaded-AnimBP crash).
    ///
    /// A genuinely-native original (`was_native`) is an `execXxx` thunk that reads
    /// its params from the caller's frame, and a frame reached via `ProcessEvent`
    /// (`node == func_ptr`) is already a real callee frame — both take the frame
    /// as-is. One exception: a force-native BP function with an *empty* script
    /// (an unimplemented BlueprintImplementableEvent) reached via `ProcessEvent`.
    /// Unhooked, ProcessEvent early-outs on `Script.Num() == 0` for non-native
    /// functions; the forced flag defeats that, so the call arrives here with a
    /// null `Code` that `ProcessInternal` would dereference. The call is a no-op
    /// anyway (mirrors `HookParams::original`'s empty-script skip), so just
    /// return — a ProcessEvent callee frame leaves no caller bytecode to advance.
    fn hook_fallthrough(
        hdr: &SinkHeader,
        context: *mut ue::UObject,
        stack: *mut ue::kismet::FFrame,
        result: *mut c_void,
        original: usize,
    ) {
        let orig: ue::FNativeFuncPtr = unsafe { std::mem::transmute(original) };
        let node = unsafe { &*stack }.node() as usize;
        if hdr.was_native || hdr.func_ptr == 0 || node == hdr.func_ptr {
            if !hdr.was_native && unsafe { &*stack }.code().is_null() {
                return;
            }
            unsafe { orig(context, stack, result) };
        } else {
            let frame = unsafe { &mut *stack };
            run_bytecode_original(context, frame, result, hdr.func_ptr, orig);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_hook(
        vm: &VirtualMachine,
        callback: &PyObjectRef,
        context: *mut ue::UObject,
        original_stack: usize,
        result: *mut c_void,
        func_ptr: usize,
        original: usize,
        orig: ue::FNativeFuncPtr,
        was_native: bool,
        params_base: usize,
        out_addrs: HashMap<usize, usize>,
    ) {
        let ufunc = unsafe { &*(func_ptr as *const ue::UFunction) };
        let return_prop = ufunc.return_property().map(|p| p as *const ue::FProperty);
        let this = match unsafe { (context as *const ue::UObjectBase).as_ref() } {
            Some(o) => wrap_obj(vm, o),
            None => vm.ctx.none(),
        };
        let guard = Rc::new(Cell::new(true));
        let hp = HookParams {
            func_ptr,
            context: context as usize,
            params_base,
            out_addrs,
            result: result as usize,
            stack: original_stack,
            original,
            was_native,
            func_name: ufunc.name_private(),
            guard: guard.clone(),
            called_original: Cell::new(false),
        }
        .into_ref(&vm.ctx);

        let call_res = callback.call((this, hp.clone()), vm);
        let called_original = hp.called_original.get();

        match call_res {
            Ok(ret) => {
                if let Some(rp) = return_prop {
                    let rp = unsafe { &*rp };
                    if !result.is_null() && !vm.is_none(&ret) {
                        if let Err(e) = write_property(vm, rp, result, ret) {
                            tracing::error!("hook: writing return value failed");
                            vm.print_exception(e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("hook callback failed");
                vm.print_exception(e);
                if !called_original {
                    unsafe { orig(context, original_stack as *mut ue::kismet::FFrame, result) };
                }
            }
        }
        guard.set(false);
    }

    /// Game-thread bytecode-call path: rebuild the callee's frame and run the
    /// Python hook against it (which may, in turn, call the original).
    #[allow(clippy::too_many_arguments)]
    fn run_bytecode_hook(
        vm: &VirtualMachine,
        callback: &PyObjectRef,
        context: *mut ue::UObject,
        frame: &mut ue::kismet::FFrame,
        result: *mut c_void,
        func_ptr: usize,
        original: usize,
        orig: ue::FNativeFuncPtr,
        was_native: bool,
    ) {
        with_reconstructed_frame(
            context,
            frame,
            func_ptr,
            was_native,
            result,
            |synth_ptr, base, out_addrs| {
                run_hook(
                    vm, callback, context, synth_ptr, result, func_ptr, original, orig, was_native,
                    base, out_addrs,
                );
            },
        );
    }

    /// Worker-thread fallthrough for a force-native BP-script function reached
    /// from bytecode: rebuild the same script frame the game-thread path does and
    /// run the *original* body — no VM, no Python. Without this, `ProcessInternal`
    /// runs against the caller's `Code` and access-violates (the crash the
    /// multithreaded AnimBP graph functions hit). `was_native` is false here by
    /// construction (a genuine native takes the caller's frame as-is; see
    /// `hook_fallthrough`).
    fn run_bytecode_original(
        context: *mut ue::UObject,
        frame: &mut ue::kismet::FFrame,
        result: *mut c_void,
        func_ptr: usize,
        orig: ue::FNativeFuncPtr,
    ) {
        // Engine guard (`if (Function->Script.Num() > 0)` in ProcessScriptFunction):
        // an empty body must not reach ProcessInternal — its null `Code` would be
        // dereferenced. The caller's arg expressions still have to be stepped to
        // advance its `Code`, so reconstruct either way and only skip the invoke.
        let script_empty = unsafe { &*(func_ptr as *const ue::UFunction) }
            .script()
            .is_empty();
        with_reconstructed_frame(
            context,
            frame,
            func_ptr,
            false,
            result,
            |synth_ptr, _base, _out_addrs| {
                if !script_empty {
                    unsafe { orig(context, synth_ptr as *mut ue::kismet::FFrame, result) };
                }
            },
        );
    }

    /// The frame surgery the engine's `ProcessScriptFunction` performs, split out
    /// so it runs identically on the game thread (where `invoke` calls the Python
    /// hook) and on a worker thread (where `invoke` just calls the original). It
    /// steps the params out of the caller's `frame`, initializes locals, wires the
    /// out-param records (for a script function, seeded with the return value's
    /// record pointing at `result`) and — for a script function — points `Code`
    /// at the body, runs `invoke(synth_ptr, base, out_addrs)`, then advances the
    /// caller's `Code` and destroys everything it constructed.
    fn with_reconstructed_frame(
        context: *mut ue::UObject,
        frame: &mut ue::kismet::FFrame,
        func_ptr: usize,
        was_native: bool,
        result: *mut c_void,
        invoke: impl FnOnce(usize, usize, HashMap<usize, usize>),
    ) {
        use ue::EPropertyFlags as PF;
        let ufunc = unsafe { &*(func_ptr as *const ue::UFunction) };

        // A genuine native (`was_native`) is re-entered in `original()` as an
        // `execXxx` thunk that only touches the parameters, so a `ParmsSize` frame
        // with a null `Code` is enough. A BP-script function we force-flipped to
        // FUNC_Native is re-entered through `ProcessInternal`, which runs the WHOLE
        // body via `ProcessLocalScriptFunction(Stack)` — it reads `Stack.Code` and
        // uses locals past the parameters. So it needs a real script frame:
        // `PropertiesSize` locals (zeroed, then initialized), `Code` at the
        // function's bytecode, `Object` set to the call's context — exactly what
        // the engine's `ProcessScriptFunction` builds. (A `ParmsSize` buffer or a
        // null `Code` here is the access violation we're fixing.)
        let is_script = !was_native;

        let param_props: Vec<&'static ue::FProperty> = ufunc
            .params()
            .map(|p| unsafe { std::mem::transmute::<&ue::FProperty, &'static _>(p) })
            .collect();

        let step_ctx = frame.object();
        let frame_size = if is_script {
            ufunc.properties_size() as usize
        } else {
            ufunc.parms_size() as usize
        };
        let mut tempbuf = vec![0u8; frame_size];
        let base = tempbuf.as_mut_ptr() as usize;
        let mut out_addrs: HashMap<usize, usize> = HashMap::new();
        for p in &param_props {
            let dest = (base + p.offset() as usize) as *mut c_void;
            let is_out = p.property_flags().contains(PF::CPF_OutParm);
            // Mirror ProcessScriptFunction: clear the address before every step
            // so an expression that doesn't set it (an optional/default out-param)
            // can't leave a *stale* address from the previous parameter behind.
            frame
                .most_recent_property_address_mut()
                .set(std::ptr::null());
            if is_script && is_out {
                // Mirror ProcessScriptFunction: evaluate the out-param expression
                // with a NULL result so the caller's value isn't copied into our
                // local — the OutParm record below points at the caller's storage,
                // which is where the body writes and the caller reads.
                unsafe { ue::kismet::step(frame, step_ctx, std::ptr::null_mut()) };
            } else if is_script {
                // In-params are initialized before the arg expression is copied in.
                unsafe { p.initialize_value(dest) };
                unsafe { ue::kismet::step(frame, step_ctx, dest) };
            } else {
                unsafe { ue::kismet::step(frame, step_ctx, dest) };
            }
            if is_out {
                // Engine fallback: an out-param expression that names no property
                // (optional out-param fed a default) yields a null address — point
                // the record at the param's own frame slot so there's always
                // valid storage to write through.
                let addr = frame.most_recent_property_address() as usize;
                out_addrs.insert(
                    *p as *const _ as usize,
                    if addr != 0 {
                        addr
                    } else {
                        base + p.offset() as usize
                    },
                );
            }
        }

        // Initialize local variables (everything that isn't a parameter) so the
        // body sees constructed values — ProcessScriptFunction's FirstPropertyToInit
        // pass. Own props only: a super's locals don't index into this frame.
        if is_script {
            for p in ufunc.own_props() {
                if !p.property_flags().contains(PF::CPF_Parm) {
                    unsafe { p.initialize_value((base + p.offset() as usize) as *mut c_void) };
                }
            }
        }

        // A script body reads/writes every CPF_OutParm — *including* the return
        // value — through `Stack.OutParms` (execLocalOutVariable walks the chain
        // matching by property, with no null check in shipping). So the chain
        // must carry a record for the return property too, pointing at the
        // caller's result storage — ProcessScriptFunction seeds its chain with
        // exactly this record before wiring the other out-params. Omitting it
        // walks off the end of the chain the first time the body touches
        // ReturnValue. Natives don't need it: their thunk gets RESULT_PARAM
        // directly and never looks ReturnValue up in the chain.
        let ret_rec = (is_script && !result.is_null())
            .then(|| ufunc.return_property())
            .flatten()
            .map(|rp| ue::kismet::FOutParmRec {
                property: rp as *const _,
                prop_addr: result as *mut u8,
                next_out_parm: std::ptr::null(),
            });
        let mut synth_out: Vec<ue::kismet::FOutParmRec> = ret_rec
            .into_iter()
            .chain(
                param_props
                    .iter()
                    .filter(|p| p.property_flags().contains(PF::CPF_OutParm))
                    .filter_map(|p| {
                        out_addrs.get(&(*p as *const _ as usize)).map(|&addr| {
                            ue::kismet::FOutParmRec {
                                property: *p as *const _,
                                prop_addr: addr as *mut u8,
                                next_out_parm: std::ptr::null(),
                            }
                        })
                    }),
            )
            .collect();
        let addrs: Vec<usize> = synth_out.iter().map(|r| r as *const _ as usize).collect();
        for (i, rec) in synth_out.iter_mut().enumerate() {
            rec.next_out_parm = addrs
                .get(i + 1)
                .map(|&a| a as *const ue::kismet::FOutParmRec)
                .unwrap_or(std::ptr::null());
        }
        let synth_out_head = addrs.first().copied().unwrap_or(0);

        // Native frame: null `Code` (the thunk reads params from `Locals`) and
        // advertise the native function. Script frame: `Code` at the bytecode,
        // `Object` = context (the `self` opcodes execute against), no current
        // native function — a plain script call frame.
        let (code, cur_native) = if is_script {
            (ufunc.script().as_ptr() as *const c_void, std::ptr::null())
        } else {
            (std::ptr::null(), func_ptr as *const c_void)
        };
        let mut synth_buf = unsafe { ue::kismet::FFrameBuf::from_frame(frame) };
        let synth = synth_buf.frame_mut();
        synth.code_mut().set(code);
        synth.locals_mut().set(base as *const c_void);
        synth.most_recent_property_mut().set(std::ptr::null());
        synth
            .most_recent_property_address_mut()
            .set(std::ptr::null());
        synth.out_parms_mut().set(synth_out_head as *const c_void);
        synth
            .property_chain_for_compiled_in_mut()
            .set(ufunc.child_properties());
        synth.node_mut().set(func_ptr as *const c_void);
        synth.current_native_function_mut().set(cur_native);
        if is_script {
            synth.object_mut().set(context);
        }
        let synth_ptr = synth as *mut ue::kismet::FFrame as usize;

        invoke(synth_ptr, base, out_addrs);

        let next_code = unsafe { (frame.code() as *const u8).add(1) as *const c_void };
        frame.code_mut().set(next_code);

        // Destroy what we constructed. Script frame: every non-out-param — value
        // in-params and locals (mirrors ProcessScriptFunction's DestructorLink
        // pass); out-params live in the caller's storage, which the caller owns.
        // Native frame: just the stepped parameters.
        if is_script {
            for p in ufunc.own_props() {
                if !p.property_flags().contains(PF::CPF_OutParm) {
                    unsafe { p.destroy_value((base + p.offset() as usize) as *mut c_void) };
                }
            }
        } else {
            for p in &param_props {
                let slot = (base + p.offset() as usize) as *mut c_void;
                unsafe { p.destroy_value(slot) };
            }
        }
    }

    fn build_out_addrs(out_parms_head: usize) -> HashMap<usize, usize> {
        let mut m = HashMap::new();
        let mut rec = out_parms_head as *const ue::kismet::FOutParmRec;
        while let Some(r) = unsafe { rec.as_ref() } {
            m.insert(r.property as usize, r.prop_addr as usize);
            rec = r.next_out_parm;
        }
        m
    }

    // ── HookParams — handed to the callback as `cb(this, params)` ────────────

    #[pyattr]
    #[pyclass(name = "HookParams", module = "ue")]
    #[derive(PyPayload)]
    pub struct HookParams {
        func_ptr: usize,
        context: usize,
        params_base: usize,
        out_addrs: HashMap<usize, usize>,
        result: usize,
        stack: usize,
        original: usize,
        // Was the target originally native? If not, `original` points at
        // `ProcessInternal` (the BP-script dispatcher); if so, it's an `execXxx`
        // thunk. Governs whether calling `original()` on an empty-script function
        // is safe (see `original`).
        was_native: bool,
        // Captured at construction so `repr` works without touching engine
        // memory after the guard drops.
        func_name: ue::FName,
        guard: Rc<Cell<bool>>,
        called_original: Cell<bool>,
    }

    impl HookParams {
        /// The hooked function. Only valid while `guard` is true: during the
        /// callback the function is on the native call stack (and GC shares this
        /// thread), so it's alive; afterwards the hook holds only a weak ref and
        /// `func_ptr` may dangle. Every Python-reachable entry point must
        /// `check_alive` **before** calling this.
        fn ufunc(&self) -> &ue::UFunction {
            debug_assert!(self.guard.get(), "HookParams::ufunc after guard dropped");
            unsafe { &*(self.func_ptr as *const ue::UFunction) }
        }
        fn check_alive(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.guard.get() {
                Ok(())
            } else {
                Err(runtime_err(
                    vm,
                    "hook parameters are no longer valid (callback already returned)",
                ))
            }
        }
        fn find_param(&self, name: &str) -> Option<&'static ue::FProperty> {
            let fname = ue::FName::find(name);
            if fname.comparison_index.value == 0 {
                return None;
            }
            self.ufunc()
                .params()
                .find(|p| p.name() == fname)
                .map(|p| unsafe { std::mem::transmute::<&ue::FProperty, &'static _>(p) })
        }
        fn param_addr(&self, p: &ue::FProperty) -> usize {
            if let Some(&a) = self.out_addrs.get(&(p as *const _ as usize)) {
                a
            } else {
                self.params_base + p.offset() as usize
            }
        }
        fn param_path(&self, p: &ue::FProperty) -> Path {
            Path::frame(self.param_addr(p), p as *const _, self.guard.clone())
        }
        fn dir_names(&self) -> Vec<String> {
            self.ufunc()
                .params()
                .map(|p| p.name().to_string())
                .collect()
        }
    }

    #[pyclass(with(Constructor, GetAttr, SetAttr, Representable))]
    impl HookParams {
        #[pygetset]
        fn this(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.check_alive(vm)?;
            Ok(
                match unsafe { (self.context as *const ue::UObjectBase).as_ref() } {
                    Some(o) => wrap_obj(vm, o),
                    None => vm.ctx.none(),
                },
            )
        }

        #[pygetset]
        fn args(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            self.check_alive(vm)?;
            let mut out = Vec::new();
            for p in self.ufunc().params().filter(|p| {
                let f = p.property_flags();
                // Same test as `parm_role` and `BoundMethod::call`: only a
                // non-const out param is an output. A const-ref param is an
                // input the caller had to supply, so it belongs in `args`.
                !(f.contains(ue::EPropertyFlags::CPF_OutParm)
                    && !f.contains(ue::EPropertyFlags::CPF_ConstParm))
            }) {
                out.push(read_at_path(vm, self.param_path(p))?);
            }
            Ok(out)
        }

        #[pymethod]
        fn __dir__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.check_alive(vm)?;
            Ok(to_list(vm, self.dir_names()))
        }

        #[pymethod]
        fn original(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.check_alive(vm)?;
            if self.called_original.get() {
                return Err(runtime_err(vm, "original() already called"));
            }
            self.called_original.set(true);
            // A BP-script function with an empty `Script` (an unimplemented
            // BlueprintImplementableEvent) has no body: its `Func` is
            // `ProcessInternal`, which hands the frame straight to
            // `ProcessLocalScriptFunction`'s `while (*Stack.Code != EX_Return)`
            // loop — and an empty `Script` gives a null `Code`, so it dereferences
            // 0x0. Such a call is a no-op anyway, so skip it. Genuine natives
            // (`was_native`) run their thunk even with an empty `Script` — that's
            // where their logic lives.
            if !self.was_native && self.ufunc().script().is_empty() {
                return Ok(vm.ctx.none());
            }
            let orig: ue::FNativeFuncPtr = unsafe { std::mem::transmute(self.original) };
            unsafe {
                orig(
                    self.context as *mut ue::UObject,
                    self.stack as *mut ue::kismet::FFrame,
                    self.result as *mut c_void,
                );
            }
            match self.ufunc().return_property() {
                Some(rp) if self.result != 0 => read_at_path(
                    vm,
                    Path::frame(self.result, rp as *const _, self.guard.clone()),
                ),
                _ => Ok(vm.ctx.none()),
            }
        }
    }

    impl Representable for HookParams {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            // The stale form uses only the cached name: after the callback
            // returns the hooked UFunction may have been GC'd.
            if !zelf.guard.get() {
                return Ok(format!("<HookParams {} (stale)>", zelf.func_name));
            }
            let names = zelf.dir_names();
            Ok(format!(
                "<HookParams {}({})>",
                zelf.func_name,
                names.join(", ")
            ))
        }
    }
    impl GetAttr for HookParams {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            if let Some(attr) = zelf.as_object().generic_getattr_opt(name, None, vm)? {
                return Ok(attr);
            }
            // Before `find_param` walks the UFunction's params: the function
            // pointer is only known-good while the guard holds.
            zelf.check_alive(vm)?;
            let name = name.to_string();
            let p = zelf
                .find_param(&name)
                .ok_or_else(|| attr_err(vm, name.clone()))?;
            read_at_path(vm, zelf.param_path(p))
        }
    }
    impl SetAttr for HookParams {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.check_alive(vm)?;
            let value = match value {
                PySetterValue::Assign(v) => v,
                PySetterValue::Delete => return Err(attr_err(vm, "cannot delete hook params")),
            };
            let name = name.to_string();
            let p = zelf
                .find_param(&name)
                .ok_or_else(|| attr_err(vm, name.clone()))?;
            write_at_path(vm, zelf.param_path(p), value)
        }
    }

    /// Resolve `(class, name)` to a UFunction. Returns a raw pointer rather
    /// than a reference: the function's lifetime is governed by the GC, not by
    /// any borrow we could name. Callers deref immediately, on the game thread,
    /// with no engine re-entry (and hence no GC) between resolve and last use.
    fn resolve_hook_function(
        vm: &VirtualMachine,
        cls: &PyObject,
        func_name: &str,
    ) -> PyResult<*const ue::UFunction> {
        // A `UClass` also downcasts to `UObject` (it's a subclass), so only
        // reject plain instances — i.e. a `UObject` that is *not* a `UClass`.
        if cls.downcast_ref::<UObject>().is_some() && cls.downcast_ref::<UClass>().is_none() {
            return Err(type_err(
                vm,
                "hooks are global per function, not per instance — pass the class \
                 (e.g. obj._class) or a class path string, not an object",
            ));
        }
        let ustruct = resolve_class(vm, cls)? as *const ue::UStruct;
        let fname = ue::FName::find(func_name);
        if fname.comparison_index.value == 0 {
            return Err(value_err(
                vm,
                format!("no function named '{func_name}' (name not in the name table)"),
            ));
        }
        let func = unsafe { &*ustruct }
            .find_function(fname)
            .ok_or_else(|| value_err(vm, format!("no UFunction '{func_name}' on this class")))?;
        Ok(func as *const ue::UFunction)
    }

    #[pyfunction]
    fn hook(
        cls: PyObjectRef,
        func_name: String,
        callback: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // SAFETY: just resolved, game thread, and nothing below re-enters the
        // engine, so no GC can free the function before the last use.
        let func = unsafe { &*resolve_hook_function(vm, &cls, &func_name)? };
        let func_addr = func as *const ue::UFunction as usize;
        let weak = ue::FWeakObjectPtr::new(func);

        with_sinks(|t| {
            reclaim_dead_hooks(t);
            if find_live_hook(t, &weak).is_some() {
                return Err(runtime_err(
                    vm,
                    format!("'{func_name}' is already hooked (unhook it first)"),
                ));
            }
            let original = func.native_func_addr();
            let orig_flags = func.function_flags();
            let (mut header, cookie, stub) = new_stub(t, SinkKind::Hook, original);
            // Populate the cross-thread header *before* installing the stub, so a
            // worker-thread call that sees the swapped `Func` finds the identity
            // its fallthrough needs (see `SinkHeader` / `hook_fallthrough`). The
            // Release store of `original` publishes the two plain fields written
            // just above; the worker reads them only after its Acquire load of
            // `original` in `sink_dispatch`, so that load synchronizes with this.
            // The `Func` install below is itself a Release store (the
            // `native_func_addr` place), which additionally orders the stub's
            // code bytes before the swapped pointer.
            header.func_ptr = func_addr;
            header.was_native = orig_flags.contains(ue::EFunctionFlags::FUNC_Native);
            header.original.store(original, Ordering::Release);
            // SAFETY: `stub` is a live, executable trampoline with this
            // function's calling convention, and the saved `original` stays
            // valid across the swap for threads still entering it.
            unsafe { func.native_func_addr_atomic().store(stub as usize) };
            // Force FUNC_Native so the Blueprint VM dispatches this function
            // through `Func` (the stub). Without it, BP-script functions called
            // from other bytecode go straight to ProcessLocalScriptFunction and
            // never hit our swapped `Func`. The stub's bytecode path
            // (run_bytecode_hook) handles the script calling convention.
            // SAFETY: with `Func` already the stub, both flag values are valid
            // in every interleaving (see the field docs).
            if !orig_flags.contains(ue::EFunctionFlags::FUNC_Native) {
                unsafe {
                    func.function_flags_atomic()
                        .store(orig_flags | ue::EFunctionFlags::FUNC_Native);
                }
            }
            tracing::info!(
                "hooked {func_name} (UFunction {func_addr:#x}, cookie {cookie:#x}, original Func {original:#x}, native={})",
                orig_flags.contains(ue::EFunctionFlags::FUNC_Native)
            );
            t.sinks.insert(
                cookie,
                Sink {
                    header,
                    callback,
                    detail: Detail::Hook {
                        target: weak,
                        func_addr,
                        orig_flags: orig_flags.bits(),
                    },
                },
            );
            Ok(())
        })
    }

    #[pyfunction]
    fn unhook(cls: PyObjectRef, func_name: String, vm: &VirtualMachine) -> PyResult<()> {
        // SAFETY: as in `hook` — freshly resolved, game thread, no engine
        // re-entry before the last use.
        let func = unsafe { &*resolve_hook_function(vm, &cls, &func_name)? };
        let weak = ue::FWeakObjectPtr::new(func);

        with_sinks(|t| {
            reclaim_dead_hooks(t);
            let Some(cookie) = find_live_hook(t, &weak) else {
                return Err(runtime_err(vm, format!("'{func_name}' is not hooked")));
            };
            let (original, orig_flags) = match &t.sinks[&cookie].detail {
                Detail::Hook { orig_flags, .. } => (
                    t.sinks[&cookie].header.original.load(Ordering::Acquire),
                    *orig_flags,
                ),
                _ => unreachable!("find_live_hook only returns Hook cookies"),
            };
            // Restore the original FunctionFlags *before* Func. A worker thread
            // may observe these two stores in either order; flags-first
            // keeps every interleaving safe: once FUNC_Native is cleared, bytecode
            // dispatch routes through ProcessScriptFunction again and stops
            // consulting `Func`; a worker that still saw the flag set finds `Func`
            // pointing at the stub, which falls through correctly. The reverse
            // order has a window where FUNC_Native is set but `Func` is already
            // ProcessInternal — ProcessLocalFunction would invoke ProcessInternal
            // with the *caller's* frame and execute the caller's remaining
            // bytecode as the callee body. `header.original` stays published
            // (see `SinkHeader`) so stragglers that entered the stub late remain
            // transparent pass-throughs.
            unsafe {
                func.function_flags_atomic()
                    .store(ue::EFunctionFlags::from_bits_truncate(orig_flags));
            }
            if original != 0 {
                unsafe { func.native_func_addr_atomic().store(original) };
            }
            free_sink(t, cookie);
            Ok(())
        })
    }

    /// Unhook everything, returning how many were removed. Hooks are global
    /// per UFunction and survive a failed experiment, so a REPL session needs a
    /// way to get back to a clean slate without tracking what it installed.
    #[pyfunction]
    fn unhook_all(vm: &VirtualMachine) -> PyResult<usize> {
        let mut removed = 0;
        loop {
            let next = with_sinks(|t| {
                reclaim_dead_hooks(t);
                t.sinks.values().find_map(|s| match &s.detail {
                    Detail::Hook { target, .. } => target
                        .get_even_if_pending_kill()
                        .map(|o| (o.name_private().to_string(), o.outer_private())),
                    _ => None,
                })
            });
            let Some((name, outer)) = next else { break };
            let Some(owner) = (unsafe { outer.as_ref() }) else {
                break;
            };
            unhook(wrap_obj(vm, owner), name, vm)?;
            removed += 1;
        }
        Ok(removed)
    }

    #[pyfunction(name = "hooks")]
    /// `[(owning_class, "FuncName"), ...]` — the exact argument pair `unhook`
    /// takes, so `for h in ue.hooks(): ue.unhook(*h)` works. (It used to return
    /// `"/Script/X.Class:Func"` paths, which `unhook` rejects.)
    fn hooks_list(vm: &VirtualMachine) -> PyObjectRef {
        let out: Vec<PyObjectRef> = with_sinks(|t| {
            reclaim_dead_hooks(t);
            t.sinks
                .values()
                .filter_map(|s| match &s.detail {
                    Detail::Hook { target, .. } => target.get_even_if_pending_kill(),
                    _ => None,
                })
                .map(|o| {
                    // A UFunction's Outer is the UClass that declares it.
                    let owner = match unsafe { o.outer_private().as_ref() } {
                        Some(c) => wrap_obj(vm, c),
                        None => vm.ctx.none(),
                    };
                    vm.new_pyobj((owner, o.name_private().to_string()))
                })
                .collect()
        });
        vm.ctx.new_list(out).into()
    }

    // ──────────────────────────────────────────────────────────────────────
    // Actor begin-play observation (detour on AActor::DispatchBeginPlay).
    // ──────────────────────────────────────────────────────────────────────

    struct BeginPlayReg {
        id: u64,
        class: Option<ue::FWeakObjectPtr>,
        cb: PyObjectRef,
    }

    /// Called from the `AActor::DispatchBeginPlay` detour for every actor.
    pub fn dispatch_begin_play(this: *const ue::UObjectBase) {
        // Cheap fast path: the overwhelmingly common case is no registrations.
        if with_begin_play(|r| r.borrow().is_empty()) {
            return;
        }
        let Some(obj) = (unsafe { this.as_ref() }) else {
            return;
        };
        let class_ptr = obj.class_private();

        crate::interp::with_vm(|vm| {
            // Snapshot matching callbacks, then drop the borrow before invoking:
            // a callback that spawns an actor re-enters this fn.
            let callbacks: Vec<PyObjectRef> = with_begin_play(|r| {
                r.borrow()
                    .iter()
                    .filter(|r| match &r.class {
                        None => true,
                        Some(w) => match w.get_even_if_pending_kill() {
                            // `class_ptr: *const UClass` — `is_child_of` (a UStruct
                            // method) is reachable through Deref. `c` is a class, so
                            // downcast it to UStruct via the checked cast.
                            Some(c) => unsafe { class_ptr.as_ref() }.is_some_and(|cls| {
                                c.cast::<ue::UStruct>()
                                    .is_some_and(|c_struct| cls.is_child_of(c_struct))
                            }),
                            None => false,
                        },
                    })
                    .map(|r| r.cb.clone())
                    .collect()
            });
            if callbacks.is_empty() {
                return;
            }
            let actor = wrap_obj(vm, obj);
            for cb in callbacks {
                if let Err(e) = cb.call((actor.clone(),), vm) {
                    tracing::error!("on_begin_play callback failed");
                    vm.print_exception(e);
                }
            }
        });
    }

    #[pyfunction]
    fn on_begin_play(
        cls: PyObjectRef,
        callback: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<BeginPlayHook> {
        if !callback.is_callable() {
            return Err(type_err(vm, "callback must be callable"));
        }
        let class = if vm.is_none(&cls) {
            None
        } else {
            let cp = resolve_class(vm, &cls)?;
            let base =
                unsafe { cp.as_ref() }.ok_or_else(|| value_err(vm, "could not resolve class"))?;
            // &UClass coerces to &UObjectBase.
            Some(ue::FWeakObjectPtr::new(base))
        };
        let id = next_reg_id();
        with_begin_play(|r| {
            r.borrow_mut().push(BeginPlayReg {
                id,
                class,
                cb: callback,
            })
        });
        Ok(BeginPlayHook { id })
    }

    #[pyfunction]
    fn begin_play_hooks(vm: &VirtualMachine) -> PyObjectRef {
        let items: Vec<(u64, String)> = with_begin_play(|r| {
            r.borrow()
                .iter()
                .map(|r| {
                    let label = match &r.class {
                        None => "<all actors>".to_string(),
                        Some(w) => w
                            .get_even_if_pending_kill()
                            .map(|o| o.object_path())
                            .unwrap_or_else(|| "<gc'd class>".to_string()),
                    };
                    (r.id, label)
                })
                .collect()
        });
        to_list(vm, items)
    }

    #[pyattr]
    #[pyclass(name = "BeginPlayHook", module = "ue")]
    #[derive(PyPayload)]
    pub struct BeginPlayHook {
        id: u64,
    }

    #[pyclass(with(Constructor, Representable))]
    impl BeginPlayHook {
        #[pymethod]
        fn remove(&self) {
            with_begin_play(|r| r.borrow_mut().retain(|x| x.id != self.id));
        }
        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
        #[pymethod]
        fn __exit__(&self, _args: FuncArgs) -> bool {
            self.remove();
            false
        }
    }
    impl Representable for BeginPlayHook {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let active = with_begin_play(|r| r.borrow().iter().any(|x| x.id == zelf.id));
            Ok(format!("<BeginPlayHook id={} active={}>", zelf.id, active))
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Per-frame world tick (detour on UWorld::Tick). Callbacks fire once per
    // frame *per world* with `(world, delta_seconds)`, where `delta_seconds`
    // is the world's dilated/clamped frame delta (the same value BP
    // `Event Tick` sees), and after the engine's own tick for the frame — see
    // `hooked_uworld_tick`. Registry is thread-local (engine thread) like the
    // begin-play one, since callbacks are RustPython `PyObjectRef` (Rc, !Send).
    // ──────────────────────────────────────────────────────────────────────

    struct TickReg {
        id: u64,
        cb: PyObjectRef,
    }

    /// Called from the `UWorld::Tick` detour every frame, for every world.
    pub fn dispatch_world_tick(this: *const ue::UObjectBase, delta_seconds: f32) {
        // Cheap fast path: the overwhelmingly common case is no registrations.
        if with_tick(|r| r.borrow().is_empty()) {
            return;
        }
        let Some(obj) = (unsafe { this.as_ref() }) else {
            return;
        };

        crate::interp::with_vm(|vm| {
            // Snapshot then drop the borrow before invoking: a callback that
            // (un)registers a tick must not re-enter the borrow.
            let callbacks: Vec<PyObjectRef> =
                with_tick(|r| r.borrow().iter().map(|r| r.cb.clone()).collect());
            let world = wrap_obj(vm, obj);
            let dt: PyObjectRef = vm.ctx.new_float(delta_seconds as f64).into();
            for cb in callbacks {
                if let Err(e) = cb.call((world.clone(), dt.clone()), vm) {
                    tracing::error!("on_tick callback failed");
                    vm.print_exception(e);
                }
            }
        });
    }

    #[pyfunction]
    fn on_tick(callback: PyObjectRef, vm: &VirtualMachine) -> PyResult<TickHook> {
        if !callback.is_callable() {
            return Err(type_err(vm, "callback must be callable"));
        }
        let id = next_reg_id();
        with_tick(|r| r.borrow_mut().push(TickReg { id, cb: callback }));
        Ok(TickHook { id })
    }

    #[pyfunction]
    fn tick_hooks(vm: &VirtualMachine) -> PyObjectRef {
        let ids: Vec<u64> = with_tick(|r| r.borrow().iter().map(|r| r.id).collect());
        to_list(vm, ids)
    }

    #[pyattr]
    #[pyclass(name = "TickHook", module = "ue")]
    #[derive(PyPayload)]
    pub struct TickHook {
        id: u64,
    }

    #[pyclass(with(Constructor, Representable))]
    impl TickHook {
        #[pymethod]
        fn remove(&self) {
            with_tick(|r| r.borrow_mut().retain(|x| x.id != self.id));
        }
        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
        #[pymethod]
        fn __exit__(&self, _args: FuncArgs) -> bool {
            self.remove();
            false
        }
    }
    impl Representable for TickHook {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let active = with_tick(|r| r.borrow().iter().any(|x| x.id == zelf.id));
            Ok(format!("<TickHook id={} active={}>", zelf.id, active))
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Kismet bytecode instruction trace (GNatives dispatch-table swap).
    // ──────────────────────────────────────────────────────────────────────

    const GNATIVES_LEN: usize = 0xff;

    macro_rules! opcodes {
        ($($name:ident = $val:expr),* $(,)?) => {
            fn opcode_name(op: u8) -> &'static str {
                match op { $($val => stringify!($name),)* _ => "EX_Unknown" }
            }
            fn opcode_from_name(s: &str) -> Option<u8> {
                match s { $(stringify!($name) => Some($val),)* _ => None }
            }
        };
    }
    opcodes! {
        EX_LocalVariable=0x00, EX_InstanceVariable=0x01, EX_DefaultVariable=0x02,
        EX_Return=0x04, EX_Jump=0x06, EX_JumpIfNot=0x07, EX_Assert=0x09, EX_Nothing=0x0B,
        EX_Let=0x0F, EX_ClassContext=0x12, EX_MetaCast=0x13, EX_LetBool=0x14,
        EX_EndParmValue=0x15, EX_EndFunctionParms=0x16, EX_Self=0x17, EX_Skip=0x18,
        EX_Context=0x19, EX_Context_FailSilent=0x1A, EX_VirtualFunction=0x1B,
        EX_FinalFunction=0x1C, EX_IntConst=0x1D, EX_FloatConst=0x1E, EX_StringConst=0x1F,
        EX_ObjectConst=0x20, EX_NameConst=0x21, EX_RotationConst=0x22, EX_VectorConst=0x23,
        EX_ByteConst=0x24, EX_IntZero=0x25, EX_IntOne=0x26, EX_True=0x27, EX_False=0x28,
        EX_TextConst=0x29, EX_NoObject=0x2A, EX_TransformConst=0x2B, EX_IntConstByte=0x2C,
        EX_NoInterface=0x2D, EX_DynamicCast=0x2E, EX_StructConst=0x2F, EX_EndStructConst=0x30,
        EX_SetArray=0x31, EX_EndArray=0x32, EX_PropertyConst=0x33, EX_UnicodeStringConst=0x34,
        EX_Int64Const=0x35, EX_UInt64Const=0x36, EX_PrimitiveCast=0x38, EX_SetSet=0x39,
        EX_EndSet=0x3A, EX_SetMap=0x3B, EX_EndMap=0x3C, EX_SetConst=0x3D, EX_EndSetConst=0x3E,
        EX_MapConst=0x3F, EX_EndMapConst=0x40, EX_StructMemberContext=0x42,
        EX_LetMulticastDelegate=0x43, EX_LetDelegate=0x44, EX_LocalVirtualFunction=0x45,
        EX_LocalFinalFunction=0x46, EX_LocalOutVariable=0x48, EX_DeprecatedOp4A=0x4A,
        EX_InstanceDelegate=0x4B, EX_PushExecutionFlow=0x4C, EX_PopExecutionFlow=0x4D,
        EX_ComputedJump=0x4E, EX_PopExecutionFlowIfNot=0x4F, EX_Breakpoint=0x50,
        EX_InterfaceContext=0x51, EX_ObjToInterfaceCast=0x52, EX_EndOfScript=0x53,
        EX_CrossInterfaceCast=0x54, EX_InterfaceToObjCast=0x55, EX_WireTracepoint=0x5A,
        EX_SkipOffsetConst=0x5B, EX_AddMulticastDelegate=0x5C, EX_ClearMulticastDelegate=0x5D,
        EX_Tracepoint=0x5E, EX_LetObj=0x5F, EX_LetWeakObjPtr=0x60, EX_BindDelegate=0x61,
        EX_RemoveMulticastDelegate=0x62, EX_CallMulticastDelegate=0x63,
        EX_LetValueOnPersistentFrame=0x64, EX_ArrayConst=0x65, EX_EndArrayConst=0x66,
        EX_SoftObjectConst=0x67, EX_CallMath=0x68, EX_SwitchValue=0x69,
        EX_InstrumentationEvent=0x6A, EX_ArrayGetByRef=0x6B, EX_ClassSparseDataVariable=0x6C,
        EX_FieldPathConst=0x6D,
    }

    struct TraceState {
        saved: Box<[Option<ue::FNativesFuncPtr>; GNATIVES_LEN]>,
        swapped: Vec<usize>,
        active: bool,
        func: usize,
        class: usize,
        // (label_id, script offset, opcode). We store an interned label id, NOT
        // the raw `node` pointer: the function's path is resolved at RECORD time
        // (game thread, node guaranteed live) and interned. Deferring resolution
        // to drain time is unsound -- by then the owning world may be torn down
        // and the `node` freed, and worse, its address may have been REUSED by a
        // different UFunction, so a pointer-keyed lookup would silently mislabel.
        buf: Vec<(u32, u32, u8)>,
        labels: Vec<String>,
        label_ids: HashMap<String, u32>,
        max_events: usize,
        dropped: u64,
    }

    impl TraceState {
        /// Intern a function path resolved at record time; returns its label id.
        fn intern(&mut self, path: String) -> u32 {
            if let Some(&id) = self.label_ids.get(&path) {
                return id;
            }
            let id = self.labels.len() as u32;
            self.labels.push(path.clone());
            self.label_ids.insert(path, id);
            id
        }
    }

    /// Original `GNatives` opcode handlers for the currently-swapped slots,
    /// reachable from **any** thread. `GNatives` is a process-global table, so a
    /// swapped opcode's `trace_handler` fires on worker threads too (e.g. a
    /// `ParallelAnimationEvaluation` task running AnimBP bytecode). Those threads
    /// have no trace state in their `REGISTRIES`, so they read the original here and
    /// forward to it -- otherwise the opcode is silently skipped, leaving the BP
    /// `FFrame` mid-instruction and corrupting the VM (a Python-reachable native
    /// crash). 0 = no override installed for that slot. Mirrors `ORIGINALS`.
    static SAVED_NATIVES: [std::sync::atomic::AtomicUsize; GNATIVES_LEN] =
        [const { std::sync::atomic::AtomicUsize::new(0) }; GNATIVES_LEN];

    fn trace_offset(node: usize, code: *const u8) -> u32 {
        let func = unsafe { &*(node as *const ue::UFunction) };
        let base = func.script().as_ptr();
        if base.is_null() {
            return 0;
        }
        let off = unsafe { code.offset_from(base) } - 1;
        off.max(0) as u32
    }

    fn node_in_class(node: usize, class: usize) -> bool {
        let outer = unsafe { (*(node as *const ue::UObjectBase)).outer_private() };
        outer as usize == class
    }

    unsafe extern "system" fn trace_handler(
        obj: *mut ue::UObject,
        stack: &mut ue::kismet::FFrame,
        result: *mut c_void,
    ) {
        let code = stack.code() as *const u8;
        let opcode = unsafe { *code.sub(1) };
        let node = stack.node() as usize;

        // The original handler is reachable cross-thread; the recording buffer
        // (`REGISTRIES.trace`) is thread-local to the game thread. On worker threads
        // (e.g. parallel anim eval) we can't record, but we MUST still forward to
        // the original -- skipping it leaves the VM frame mid-instruction and
        // corrupts execution (crash). Read the original before touching the
        // buffer so it's captured regardless of thread.
        let orig_addr = SAVED_NATIVES[opcode as usize].load(std::sync::atomic::Ordering::Acquire);

        // Record only on the interpreter's own (game) thread. Short critical
        // section: the borrow is dropped when the closure returns, before we
        // dispatch to `orig` (which re-enters this handler for nested exprs).
        with_trace(|t| {
            if let Some(ts) = t.borrow_mut().as_mut() {
                let matches = (ts.func == 0 || ts.func == node)
                    && (ts.class == 0 || node_in_class(node, ts.class));
                if matches {
                    if ts.buf.len() < ts.max_events {
                        // `node` is the currently-executing function -- live here,
                        // on the game thread. Resolve + intern its path NOW; never
                        // stash the raw pointer for later (freed/reused by drain).
                        let offset = trace_offset(node, code);
                        let path = unsafe { (node as *const ue::UObjectBase).as_ref() }
                            .map(|o| o.object_path())
                            .unwrap_or_default();
                        let id = ts.intern(path);
                        ts.buf.push((id, offset, opcode));
                    } else {
                        ts.dropped += 1;
                    }
                }
            }
        });

        if orig_addr != 0 {
            let orig: ue::FNativesFuncPtr = unsafe { std::mem::transmute(orig_addr) };
            unsafe { orig(obj, stack, result) };
        }
    }

    #[pyfunction]
    fn trace(
        func: OptionalArg<PyObjectRef>,
        class_: OptionalArg<Option<PyObjectRef>>,
        opcodes: OptionalArg<Option<Vec<String>>>,
        max_events: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Trace> {
        // `func=(cls, "FnName")` — RustPython can't auto-extract a tuple as a
        // single arg, so destructure the Python 2-tuple by hand.
        let func: Option<(PyObjectRef, String)> = match func.into_option() {
            Some(o) if !vm.is_none(&o) => {
                let items: Vec<PyObjectRef> = o
                    .try_to_value(vm)
                    .map_err(|_| type_err(vm, "func must be (class, \"FuncName\")"))?;
                if items.len() != 2 {
                    return Err(type_err(vm, "func must be (class, \"FuncName\")"));
                }
                let name: String = items[1].try_to_value(vm)?;
                Some((items[0].clone(), name))
            }
            _ => None,
        };
        let class_ = class_.flatten();
        let opcodes = opcodes.flatten();
        let max_events = max_events.unwrap_or(200_000);

        let gnatives = ue::globals().gnatives;
        if gnatives.is_null() {
            return Err(runtime_err(vm, "GNatives unavailable"));
        }

        let func_addr = match &func {
            Some((cls, name)) => resolve_hook_function(vm, cls, name)? as usize,
            None => 0,
        };
        let class_addr = match &class_ {
            Some(c) => resolve_class(vm, c)? as usize,
            None => 0,
        };
        let slots: Vec<usize> = match opcodes {
            Some(names) => names
                .iter()
                .map(|n| {
                    opcode_from_name(n)
                        .map(|b| b as usize)
                        .ok_or_else(|| value_err(vm, format!("unknown opcode '{n}'")))
                })
                .collect::<PyResult<Vec<_>>>()?,
            None => (0..GNATIVES_LEN).collect(),
        };

        with_trace(|t| {
            let mut g = t.borrow_mut();
            if g.as_ref().is_some_and(|ts| ts.active) {
                return Err(runtime_err(vm, "a trace is already active; stop it first"));
            }

            let arr = unsafe { &mut *gnatives };
            let mut saved: Box<[Option<ue::FNativesFuncPtr>; GNATIVES_LEN]> =
                Box::new([None; GNATIVES_LEN]);
            saved.copy_from_slice(&arr[..]);
            let mut swapped = Vec::new();
            for &s in &slots {
                if s < GNATIVES_LEN && arr[s].is_some() {
                    // Publish the original cross-thread BEFORE swapping, so a
                    // worker thread that observes the swap can forward to it
                    // (Release pairs with the Acquire load in `trace_handler`).
                    let orig_addr = arr[s].map(|f| f as usize).unwrap_or(0);
                    SAVED_NATIVES[s].store(orig_addr, std::sync::atomic::Ordering::Release);
                    arr[s] = Some(trace_handler);
                    swapped.push(s);
                }
            }

            *g = Some(TraceState {
                saved,
                swapped,
                active: true,
                func: func_addr,
                class: class_addr,
                buf: Vec::new(),
                labels: Vec::new(),
                label_ids: HashMap::new(),
                max_events,
                dropped: 0,
            });
            tracing::info!(
                "trace started: func={func_addr:#x} class={class_addr:#x} slots={}",
                g.as_ref().unwrap().swapped.len()
            );
            Ok(())
        })?;
        Ok(Trace {})
    }

    fn trace_global_stop() {
        with_trace(|t| {
            let mut g = t.borrow_mut();
            if let Some(ts) = g.as_mut() {
                if ts.active {
                    let gnatives = ue::globals().gnatives;
                    if !gnatives.is_null() {
                        let arr = unsafe { &mut *gnatives };
                        for &s in &ts.swapped {
                            arr[s] = ts.saved[s];
                        }
                    }
                    ts.active = false;
                    tracing::info!("trace stopped: {} events", ts.buf.len());
                }
            }
        });
    }

    /// Snapshot the recorded events plus the label table needed to name them.
    /// Labels are cloned (not moved) so they still resolve events recorded after
    /// a non-clearing `events()` read; `drain` clears only the event buffer.
    fn trace_global_take(clear: bool) -> (Vec<(u32, u32, u8)>, Vec<String>) {
        with_trace(|t| {
            let mut g = t.borrow_mut();
            match g.as_mut() {
                Some(ts) if clear => (std::mem::take(&mut ts.buf), ts.labels.clone()),
                Some(ts) => (ts.buf.clone(), ts.labels.clone()),
                None => (Vec::new(), Vec::new()),
            }
        })
    }

    #[pyfunction(name = "trace_stop")]
    fn trace_stop_fn() {
        trace_global_stop();
    }

    #[pyfunction(name = "trace_events")]
    fn trace_events_fn(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        let (raw, labels) = trace_global_take(false);
        Trace::to_events(vm, raw, labels)
    }

    #[pyfunction(name = "trace_drain")]
    fn trace_drain_fn(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        let (raw, labels) = trace_global_take(true);
        Trace::to_events(vm, raw, labels)
    }

    #[pyattr]
    #[pyclass(name = "Trace", module = "ue")]
    #[derive(PyPayload)]
    pub struct Trace {}

    impl Trace {
        fn to_events(
            vm: &VirtualMachine,
            raw: Vec<(u32, u32, u8)>,
            labels: Vec<String>,
        ) -> Vec<PyObjectRef> {
            raw.into_iter()
                .map(|(label_id, offset, opcode)| {
                    // Labels were resolved at record time; just index the table.
                    // No raw pointer is ever dereferenced here.
                    let path = labels.get(label_id as usize).cloned().unwrap_or_default();
                    let d = vm.ctx.new_dict();
                    let _ = d.set_item("op", vm.new_pyobj(opcode_name(opcode)), vm);
                    let _ = d.set_item("raw", vm.new_pyobj(opcode), vm);
                    let _ = d.set_item("offset", vm.new_pyobj(offset), vm);
                    let _ = d.set_item("func", vm.new_pyobj(path), vm);
                    d.into()
                })
                .collect()
        }
    }

    #[pyclass(with(Constructor, AsSequence, Representable))]
    impl Trace {
        #[pygetset]
        fn dropped(&self) -> u64 {
            with_trace(|t| t.borrow().as_ref().map(|ts| ts.dropped).unwrap_or(0))
        }
        #[pygetset]
        fn events(&self, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            let (raw, labels) = trace_global_take(false);
            Self::to_events(vm, raw, labels)
        }
        #[pymethod]
        fn drain(&self, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            let (raw, labels) = trace_global_take(true);
            Self::to_events(vm, raw, labels)
        }
        #[pymethod]
        fn stop(&self) {
            trace_global_stop();
        }
        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
        #[pymethod]
        fn __exit__(&self, _args: FuncArgs) -> bool {
            self.stop();
            false
        }
    }
    impl Representable for Trace {
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(with_trace(|t| match t.borrow().as_ref() {
                Some(ts) => format!(
                    "<Trace active={} events={} dropped={}>",
                    ts.active,
                    ts.buf.len(),
                    ts.dropped
                ),
                None => "<Trace (stopped)>".into(),
            }))
        }
    }
    impl AsSequence for Trace {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|_seq, _vm| {
                    Ok(with_trace(|t| {
                        t.borrow().as_ref().map(|ts| ts.buf.len()).unwrap_or(0)
                    }))
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // BoundMethod — calls a UFunction on the owner via ProcessEvent.
    // ──────────────────────────────────────────────────────────────────────

    #[pyattr]
    #[pyclass(name = "BoundMethod", module = "ue")]
    #[derive(PyPayload)]
    pub struct BoundMethod {
        owner: ue::FWeakObjectPtr,
        // The UFunction, held weakly (a UFunction is a UObject): a BoundMethod
        // can outlive its function (the BP class unloads, hot-reload replaces
        // it), and the owner being alive doesn't strictly pin the function. A
        // weak ptr goes stale instead of dangling.
        function: ue::FWeakObjectPtr,
        // Captured at construction so `repr` never touches engine memory.
        func_name: ue::FName,
    }

    impl BoundMethod {
        /// The function if it's still alive; `None` once GC'd.
        fn resolve_func(&self) -> Option<&ue::UFunction> {
            self.function
                .get_even_if_pending_kill()
                .and_then(|o| o.cast::<ue::UFunction>())
        }
        fn repr_string(&self) -> String {
            match self.owner.get_even_if_pending_kill() {
                Some(o) => format!(
                    "<BoundMethod {}::{} on {}>",
                    o.class().name_private(),
                    self.func_name,
                    o.path()
                ),
                None => format!("<BoundMethod {} on stale owner>", self.func_name),
            }
        }
    }

    #[pyclass(with(Constructor, Callable, Representable))]
    impl BoundMethod {
        #[pymethod]
        fn function_info(&self, vm: &VirtualMachine) -> Option<PyRef<Function>> {
            let func = self.resolve_func()?;
            Some(function_proxy(vm, func))
        }
    }

    impl Representable for BoundMethod {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(zelf.repr_string())
        }
    }

    impl Callable for BoundMethod {
        type Args = FuncArgs;
        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let owner = zelf
                .owner
                .get_even_if_pending_kill()
                .ok_or_else(|| value_err(vm, "BoundMethod owner is stale"))?;
            let function = zelf
                .resolve_func()
                .ok_or_else(|| value_err(vm, "BoundMethod function is stale (GC'd)"))?;
            let args = args.args;

            let parms_size = function.parms_size() as usize;
            let mut buf: Vec<u8> = vec![0u8; parms_size];
            let buf_ptr: *mut c_void = if parms_size == 0 {
                std::ptr::null_mut()
            } else {
                buf.as_mut_ptr() as *mut c_void
            };

            let mut in_parms: Vec<&ue::FProperty> = Vec::new();
            let mut out_parms: Vec<&ue::FProperty> = Vec::new();
            let return_parm: Option<&ue::FProperty> = function.return_property();

            for prop in function.params() {
                let flags = prop.property_flags();
                if flags.contains(ue::EPropertyFlags::CPF_OutParm)
                    && !flags.contains(ue::EPropertyFlags::CPF_ConstParm)
                {
                    out_parms.push(prop);
                } else {
                    in_parms.push(prop);
                }
            }

            if args.len() != in_parms.len() {
                return Err(type_err(
                    vm,
                    format!(
                        "{}: expected {} arg(s), got {}",
                        zelf.repr_string(),
                        in_parms.len(),
                        args.len()
                    ),
                ));
            }
            let mut marshal_err: Option<PyBaseExceptionRef> = None;
            for (i, prop) in in_parms.iter().enumerate() {
                let slot =
                    unsafe { (buf_ptr as *mut u8).add(prop.offset() as usize) } as *mut c_void;
                if let Err(e) = write_property(vm, prop, slot, args[i].clone()) {
                    marshal_err = Some(e);
                    break;
                }
            }
            if let Some(e) = marshal_err {
                unsafe { destroy_parms(buf_ptr, function) };
                return Err(e);
            }

            unsafe {
                let pe: FnProcessEvent = std::mem::transmute(vtable_fn(
                    owner as *const _ as *const c_void,
                    vtable_process_event(),
                ));
                pe(
                    owner as *const _ as *mut _,
                    function as *const _ as *mut _,
                    buf_ptr,
                );
            }

            let mut results: Vec<PyObjectRef> = Vec::new();
            let collect = |prop: &ue::FProperty, results: &mut Vec<PyObjectRef>| -> PyResult<()> {
                let slot =
                    unsafe { (buf_ptr as *mut u8).add(prop.offset() as usize) } as *mut c_void;
                let flags = prop.class().cast_flags();
                if let Some(sp) = prop.cast_field::<ue::FStructProperty>() {
                    let ustruct = sp.get_struct();
                    let size = prop.element_size() as usize;
                    let mut owned: Box<[u8]> = vec![0u8; size].into_boxed_slice();
                    unsafe {
                        std::ptr::copy_nonoverlapping(slot as *const u8, owned.as_mut_ptr(), size);
                    }
                    let struct_ty = ustruct as *const _;
                    let root = Rc::new(OwnedStructRoot {
                        buf: std::cell::UnsafeCell::new(owned),
                        struct_ty,
                    });
                    let s = Struct {
                        path: Path::owned(root, 0),
                        struct_ty,
                    };
                    results.push(s.into_pyobject(vm));
                    Ok(())
                } else if flags.intersects(
                    ue::EClassCastFlags::CASTCLASS_FArrayProperty
                        | ue::EClassCastFlags::CASTCLASS_FMapProperty
                        | ue::EClassCastFlags::CASTCLASS_FSetProperty,
                ) {
                    let v = deep_read(vm, prop, slot)?;
                    unsafe { prop.destroy_value(slot) };
                    results.push(v);
                    Ok(())
                } else {
                    let v = read_value(vm, prop, slot)?;
                    // `read_value` yields a fully-owned Python value — an owning
                    // `ue.Text`, a copied `str`, a boxed scalar, a proxy holding a
                    // weak ptr — never a borrow into `slot`. So release the value
                    // the callee constructed here: for a returned/out `FText`,
                    // `FString` or delegate this drops the ref/heap the engine put
                    // in the buffer (the `ue.Text` copy keeps its own ref); for
                    // POD/pointer props `destroy_value` is a no-op. The struct and
                    // container branches above own their cleanup, so this only runs
                    // for the by-value scalar-ish props.
                    unsafe { prop.destroy_value(slot) };
                    results.push(v);
                    Ok(())
                }
            };
            let mut err: Option<PyBaseExceptionRef> = None;
            if let Some(rp) = return_parm {
                if let Err(e) = collect(rp, &mut results) {
                    err = Some(e);
                }
            }
            if err.is_none() {
                for op in &out_parms {
                    if let Err(e) = collect(op, &mut results) {
                        err = Some(e);
                        break;
                    }
                }
            }

            // Destroy the in-params we constructed for this call. `ProcessEvent`
            // copies whatever it needs and never takes ownership of the caller's
            // params buffer, so on return every in-param is still a live value we
            // own and must drop — otherwise `FText`/`FString`/struct/array args
            // (e.g. the handle passed to `SetText`) leak their ref/heap. `in_parms`
            // is disjoint from the return/out params `collect` reads and destroys
            // above (a param is either out or in; the return parm is excluded from
            // `params()`), so this can't double-free. Runs before the early return
            // below so it also cleans up on the collect-error path.
            unsafe {
                for prop in &in_parms {
                    let slot = (buf_ptr as *mut u8).add(prop.offset() as usize) as *mut c_void;
                    prop.destroy_value(slot);
                }
            }

            if let Some(e) = err {
                return Err(e);
            }

            Ok(match results.len() {
                0 => vm.ctx.none(),
                1 => results.into_iter().next().unwrap(),
                _ => vm.ctx.new_tuple(results).into(),
            })
        }
    }

    /// Destroy the values in a UFunction parm buffer (CPF_Parm props only).
    unsafe fn destroy_parms(buf: *mut c_void, function: &ue::UFunction) {
        if buf.is_null() {
            return;
        }
        for field in function.iter_props() {
            let Some(prop) = field.cast::<ue::FProperty>() else {
                continue;
            };
            if !prop.property_flags().contains(ue::EPropertyFlags::CPF_Parm) {
                continue;
            }
            let slot = unsafe { (buf as *mut u8).add(prop.offset() as usize) } as *mut c_void;
            unsafe { prop.destroy_value(slot) };
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Module init hook — adds `ue.EObjectFlags` (an enum.IntFlag) after the
    // derive registers all the `#[pyattr]` classes and `#[pyfunction]`s.
    // ──────────────────────────────────────────────────────────────────────

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        register_object_flags(vm, module)?;
        register_flag_enum(vm, module, "EPropertyFlags", &property_flag_table())?;
        register_flag_enum(vm, module, "EFunctionFlags", &function_flag_table())?;
        Ok(())
    }

    /// Build an `enum.IntFlag` named `name` from `(entry, bits)` pairs and bind
    /// it on the module. Shared by the property/function flag enums so they come
    /// out the same shape as the hand-rolled `EObjectFlags`.
    fn register_flag_enum(
        vm: &VirtualMachine,
        module: &Py<PyModule>,
        name: &'static str,
        table: &[(&'static str, u64)],
    ) -> PyResult<()> {
        let members = vm.ctx.new_dict();
        for (entry, bits) in table {
            members.set_item(*entry, vm.new_pyobj(*bits), vm)?;
        }
        let int_flag = vm.import("enum", 0)?.get_attr("IntFlag", vm)?;
        let cls = int_flag.call((name, members.to_pyobject(vm)), vm)?;
        module.set_attr(name, cls, vm)?;
        Ok(())
    }

    fn register_object_flags(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        use ue::EObjectFlags as F;
        let members = vm.ctx.new_dict();
        for (name, flag) in [
            ("RF_Public", F::RF_Public),
            ("RF_Standalone", F::RF_Standalone),
            ("RF_MarkAsNative", F::RF_MarkAsNative),
            ("RF_Transactional", F::RF_Transactional),
            ("RF_ClassDefaultObject", F::RF_ClassDefaultObject),
            ("RF_ArchetypeObject", F::RF_ArchetypeObject),
            ("RF_Transient", F::RF_Transient),
            ("RF_MarkAsRootSet", F::RF_MarkAsRootSet),
            ("RF_TagGarbageTemp", F::RF_TagGarbageTemp),
            ("RF_NeedInitialization", F::RF_NeedInitialization),
            ("RF_NeedLoad", F::RF_NeedLoad),
            ("RF_KeepForCooker", F::RF_KeepForCooker),
            ("RF_NeedPostLoad", F::RF_NeedPostLoad),
            ("RF_NeedPostLoadSubobjects", F::RF_NeedPostLoadSubobjects),
            ("RF_NewerVersionExists", F::RF_NewerVersionExists),
            ("RF_BeginDestroyed", F::RF_BeginDestroyed),
            ("RF_FinishDestroyed", F::RF_FinishDestroyed),
            ("RF_BeingRegenerated", F::RF_BeingRegenerated),
            ("RF_DefaultSubObject", F::RF_DefaultSubObject),
            ("RF_WasLoaded", F::RF_WasLoaded),
            ("RF_TextExportTransient", F::RF_TextExportTransient),
            ("RF_LoadCompleted", F::RF_LoadCompleted),
            (
                "RF_InheritableComponentTemplate",
                F::RF_InheritableComponentTemplate,
            ),
            ("RF_DuplicateTransient", F::RF_DuplicateTransient),
            ("RF_StrongRefOnFrame", F::RF_StrongRefOnFrame),
            (
                "RF_NonPIEDuplicateTransient",
                F::RF_NonPIEDuplicateTransient,
            ),
            ("RF_Dynamic", F::RF_Dynamic),
            ("RF_WillBeLoaded", F::RF_WillBeLoaded),
        ] {
            members.set_item(name, vm.new_pyobj(flag.bits()), vm)?;
        }
        let int_flag = vm.import("enum", 0)?.get_attr("IntFlag", vm)?;
        let cls = int_flag.call(("EObjectFlags", members.to_pyobject(vm)), vm)?;
        module.set_attr("EObjectFlags", cls, vm)?;
        Ok(())
    }
}
