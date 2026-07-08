//! `ObjectResolver` implementation backed by live UE object pointers.

use jmap_kismet_test::bytecode::refs::{ObjectRef, PropertyRef, StructRef};
use jmap_kismet_test::bytecode::types::Address;
use jmap_kismet_test::kismet_text::{
    ExprType, FunctionCallKind, LetKind, ObjectResolver, PropertyKind, ResolutionError,
};
use libue as ue;

/// [`ObjectResolver`] backed by live in-process UE pointers.
pub struct LiveResolver {
    class_path: String,
    class_addr: u64,
}

impl LiveResolver {
    pub fn new(obj: &ue::UObjectBase, _func: &ue::UFunction) -> Self {
        let function_path = obj.object_path();
        let class_path = function_path
            .rsplit_once(':')
            .map(|(c, _)| c)
            .unwrap_or(&function_path)
            .to_string();

        let class_addr = if let Some(outer) = unsafe { obj.outer_private().as_ref() } {
            outer as *const _ as u64
        } else {
            0
        };

        Self {
            class_path,
            class_addr,
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────

/// Walk `child_properties` of a UStruct (and its super chain via `iter_props`),
/// looking for a property whose FName matches `name`. Returns its address.
fn find_property_in_struct(ustruct: &ue::UStruct, name: &str) -> Option<Address> {
    let target = ue::FName::from_string(name, 0); // FNAME_Find
    for field in ustruct.iter_props() {
        if field.name() == target {
            return Some(Address(field as *const ue::FField as u64));
        }
    }
    None
}

/// Walk the `children` UField linked list (functions/sub-objects) of a UStruct,
/// including its super chain, looking for one whose FName matches `name`.
fn find_child_in_hierarchy(start: &ue::UStruct, name: &str) -> Option<Address> {
    let target = ue::FName::from_string(name, 0);
    start
        .iter_super()
        .flat_map(|st| st.iter_children())
        .find(|field| field.name_private() == target)
        .map(|field| Address(field as *const ue::UField as u64))
}

/// Resolve a UE object path to a UObjectBase, loading the package from disk
/// on demand if needed. `StaticLoadObject` handles paths to objects that
/// haven't been loaded yet, which is important for text source that references
/// classes/functions outside the currently-resident object graph.
fn resolve_path_to_object(path: &str) -> Option<&'static ue::UObjectBase> {
    ue::static_load_object(path)
}

/// Resolve a UE object path to a UStruct.
fn resolve_path_to_struct(path: &str) -> Option<&'static ue::UStruct> {
    let obj = resolve_path_to_object(path)?;
    obj.cast::<ue::UStruct>()
}

/// Resolve a path that may refer to a UObject OR a property (FField).
///
/// Properties aren't in the object hash table. If the direct lookup fails,
/// progressively split off `:` segments from the right and try to find the
/// owner UObject, then walk its properties to find the remainder.
///
/// e.g. `/Game/Foo.Bar_C:MyFunc:LocalVar` →
///   1. Try hash table for full path → miss
///   2. Split: owner=`/Game/Foo.Bar_C:MyFunc`, prop=`LocalVar`
///   3. Hash table finds `MyFunc` as UFunction (UStruct)
///   4. Walk child_properties for `LocalVar` → found
fn resolve_path_to_address(path: &str) -> Option<Address> {
    // Try as a UObject first
    if let Some(obj) = resolve_path_to_object(path) {
        return Some(Address(obj as *const _ as u64));
    }

    // Try splitting off property segments from the right
    let mut split_pos = path.len();
    while let Some(pos) = path[..split_pos].rfind(':') {
        let owner_path = &path[..pos];
        let prop_chain = &path[pos + 1..];

        if let Some(owner) = resolve_path_to_struct(owner_path) {
            // Walk the property chain (could be nested: "ArrayProp:Inner")
            let segments: Vec<&str> = prop_chain.split(':').collect();
            if let Some(addr) = walk_property_chain(owner, &segments) {
                return Some(addr);
            }
        }
        split_pos = pos;
    }

    None
}

/// Walk a chain of property names within a UStruct.
/// e.g. ["ArrayProp", "Inner"] finds ArrayProp, then follows the inner
/// property pointer (FArrayProperty::inner, FMapProperty::key/value, etc.)
/// to find nested sub-properties by name.
fn walk_property_chain(ustruct: &ue::UStruct, segments: &[&str]) -> Option<Address> {
    if segments.is_empty() {
        return None;
    }

    let first = find_property_in_struct(ustruct, segments[0])?;

    if segments.len() == 1 {
        return Some(first);
    }

    let mut current_addr = first;
    for &seg in &segments[1..] {
        current_addr = find_inner_property(current_addr, seg)?;
    }
    Some(current_addr)
}

/// Find a named inner property within a container property (array/map/set).
/// These are accessed via typed pointers, not the FField::next chain.
fn find_inner_property(parent: Address, name: &str) -> Option<Address> {
    let field = unsafe { as_field(parent)? };
    let target = ue::FName::from_string(name, 0);

    // Try array inner
    if let Some(arr) = field.cast::<ue::FArrayProperty>() {
        let inner = arr.inner();
        if inner.name() == target {
            return Some(Address(inner as *const _ as u64));
        }
    }
    // Try map key/value
    if let Some(map) = field.cast::<ue::FMapProperty>() {
        let key = map.key_property();
        if key.name() == target {
            return Some(Address(key as *const _ as u64));
        }
        let val = map.value_property();
        if val.name() == target {
            return Some(Address(val as *const _ as u64));
        }
    }
    // Try set element
    if let Some(set) = field.cast::<ue::FSetProperty>() {
        let elem = set.element_property();
        if elem.name() == target {
            return Some(Address(elem as *const _ as u64));
        }
    }

    None
}

// ── Pointer helpers ─────────────────────────────────────────────────────

unsafe fn as_field(addr: Address) -> Option<&'static ue::FFieldBase> {
    (addr.0 as *const ue::FFieldBase).as_ref()
}

unsafe fn as_property(addr: Address) -> Option<&'static ue::FProperty> {
    (addr.0 as *const ue::FProperty).as_ref()
}

unsafe fn as_object(addr: Address) -> Option<&'static ue::UObjectBase> {
    (addr.0 as *const ue::UObjectBase).as_ref()
}

unsafe fn as_struct(addr: Address) -> Option<&'static ue::UStruct> {
    (addr.0 as *const ue::UStruct).as_ref()
}

unsafe fn as_function(addr: Address) -> Option<&'static ue::UFunction> {
    (addr.0 as *const ue::UFunction).as_ref()
}

fn property_cast_flags(addr: Address) -> Option<ue::EClassCastFlags> {
    let field = unsafe { as_field(addr)? };
    Some(field.class().cast_flags())
}

// ── ObjectResolver ──────────────────────────────────────────────────────

impl ObjectResolver for LiveResolver {
    // ── Property queries ─────────────────────────────────────────────

    fn property_name(&self, prop: &PropertyRef) -> Option<String> {
        let field = unsafe { as_field(prop.address)? };
        Some(field.name().to_string())
    }

    fn property_owner(&self, prop: &PropertyRef) -> Option<ObjectRef> {
        let field = unsafe { as_field(prop.address)? };
        // Walk owner chain until we reach a UObject (owners can be FFields for nested props)
        let mut current = field.owner();
        loop {
            match current.get()? {
                ue::FFieldOwner::UObject(obj) => {
                    return Some(ObjectRef::new(Address(obj as *const _ as u64)));
                }
                ue::FFieldOwner::FField(f) => {
                    current = f.owner();
                }
            }
        }
    }

    fn property_qualified_path(&self, prop: &PropertyRef) -> Option<String> {
        // Build chain: walk owner FFields until we hit a UObject, collecting names
        let field = unsafe { as_field(prop.address)? };
        let mut chain = vec![field.name().to_string()];
        let mut current = field.owner();
        loop {
            match current.get()? {
                ue::FFieldOwner::UObject(obj) => {
                    let owner_path = obj.object_path();
                    chain.reverse();
                    return Some(format!("{owner_path}:{}", chain.join(":")));
                }
                ue::FFieldOwner::FField(f) => {
                    chain.push(f.name().to_string());
                    current = f.owner();
                }
            }
        }
    }

    fn property_expr_type(&self, prop: &PropertyRef) -> Option<ExprType> {
        use ue::EClassCastFlags as F;
        let flags = property_cast_flags(prop.address)?;

        if flags.contains(F::CASTCLASS_FBoolProperty) {
            return Some(ExprType::Bool);
        }
        if flags.contains(F::CASTCLASS_FFloatProperty)
            || flags.contains(F::CASTCLASS_FDoubleProperty)
        {
            return Some(ExprType::Float);
        }
        if flags.contains(F::CASTCLASS_FIntProperty)
            || flags.contains(F::CASTCLASS_FInt64Property)
            || flags.contains(F::CASTCLASS_FUInt64Property)
            || flags.contains(F::CASTCLASS_FInt8Property)
            || flags.contains(F::CASTCLASS_FInt16Property)
            || flags.contains(F::CASTCLASS_FUInt16Property)
            || flags.contains(F::CASTCLASS_FUInt32Property)
        {
            return Some(ExprType::Int);
        }
        if flags.contains(F::CASTCLASS_FByteProperty) || flags.contains(F::CASTCLASS_FEnumProperty)
        {
            return Some(ExprType::Byte);
        }
        if flags.contains(F::CASTCLASS_FStrProperty) {
            return Some(ExprType::Str);
        }
        if flags.contains(F::CASTCLASS_FNameProperty) {
            return Some(ExprType::Name);
        }
        if flags.contains(F::CASTCLASS_FTextProperty) {
            return Some(ExprType::Text);
        }
        if flags.contains(F::CASTCLASS_FArrayProperty) {
            return Some(ExprType::Array);
        }
        if flags.contains(F::CASTCLASS_FMapProperty) {
            return Some(ExprType::Map);
        }
        if flags.contains(F::CASTCLASS_FSetProperty) {
            return Some(ExprType::Set);
        }
        if flags.contains(F::CASTCLASS_FDelegateProperty)
            || flags.contains(F::CASTCLASS_FMulticastDelegateProperty)
            || flags.contains(F::CASTCLASS_FMulticastInlineDelegateProperty)
            || flags.contains(F::CASTCLASS_FMulticastSparseDelegateProperty)
        {
            return Some(ExprType::Delegate);
        }
        if flags.contains(F::CASTCLASS_FStructProperty) {
            let field = unsafe { as_field(prop.address)? };
            let struct_prop = field.cast::<ue::FStructProperty>()?;
            let path = struct_prop.get_struct().object_path();
            return Some(ExprType::Struct(path));
        }
        // Check more specific types before FObjectPropertyBase (which they inherit from)
        if flags.contains(F::CASTCLASS_FClassProperty) {
            let field = unsafe { as_field(prop.address)? };
            let cls_prop = field.cast::<ue::FClassProperty>()?;
            let path = cls_prop
                .meta_class()
                .map(|c| c.object_path())
                .unwrap_or_default();
            return Some(ExprType::Class(path));
        }
        if flags.contains(F::CASTCLASS_FSoftClassProperty) {
            let field = unsafe { as_field(prop.address)? };
            let cls_prop = field.cast::<ue::FSoftClassProperty>()?;
            let path = cls_prop
                .meta_class()
                .map(|c| c.object_path())
                .unwrap_or_default();
            return Some(ExprType::Class(path));
        }
        if flags.contains(F::CASTCLASS_FInterfaceProperty) {
            let field = unsafe { as_field(prop.address)? };
            let iface_prop = field.cast::<ue::FInterfaceProperty>()?;
            let path = iface_prop
                .interface_class()
                .map(|c| c.object_path())
                .unwrap_or_default();
            return Some(ExprType::Object(path));
        }
        // FObjectPropertyBase — check last since subclasses (Class, SoftClass, Interface) match it too
        if flags.contains(F::CASTCLASS_FObjectPropertyBase) {
            let field = unsafe { as_field(prop.address)? };
            let obj_prop = field.cast::<ue::FObjectPropertyBase>()?;
            let path = obj_prop
                .property_class()
                .map(|c| c.object_path())
                .unwrap_or_default();
            return Some(ExprType::Object(path));
        }

        None
    }

    fn property_let_kind(&self, prop: &PropertyRef) -> LetKind {
        use ue::EClassCastFlags as F;
        let Some(flags) = property_cast_flags(prop.address) else {
            return LetKind::Unknown;
        };
        if flags.contains(F::CASTCLASS_FBoolProperty) {
            return LetKind::Bool;
        }
        if flags.contains(F::CASTCLASS_FWeakObjectProperty) {
            return LetKind::WeakObj;
        }
        if flags.contains(F::CASTCLASS_FDelegateProperty) {
            return LetKind::Delegate;
        }
        if flags.contains(F::CASTCLASS_FMulticastDelegateProperty)
            || flags.contains(F::CASTCLASS_FMulticastInlineDelegateProperty)
            || flags.contains(F::CASTCLASS_FMulticastSparseDelegateProperty)
        {
            return LetKind::MulticastDelegate;
        }
        // Only FObjectProperty and FClassProperty use LetObj.
        // FSoftObjectProperty, FLazyObjectProperty, etc. inherit FObjectPropertyBase
        // but use Let with a property hint in bytecode.
        if flags.contains(F::CASTCLASS_FObjectProperty)
            || flags.contains(F::CASTCLASS_FClassProperty)
        {
            return LetKind::Obj;
        }
        LetKind::Let
    }

    fn property_array_element_type(&self, prop: &PropertyRef) -> Option<ExprType> {
        // Need FArrayProperty struct definition to read inner property
        // TODO: cast to FArrayProperty, read inner, recurse
        let flags = property_cast_flags(prop.address)?;
        if flags.contains(ue::EClassCastFlags::CASTCLASS_FArrayProperty) {
            let array_prop = unsafe { (prop.address.0 as *const ue::FArrayProperty).as_ref()? };
            let inner_addr = Address(array_prop.inner() as *const _ as u64);
            let inner_prop = PropertyRef::new(inner_addr);
            return self.property_expr_type(&inner_prop);
        }
        None
    }

    // ── Object queries ───────────────────────────────────────────────

    fn object_path(&self, obj: &ObjectRef) -> Option<String> {
        let o = unsafe { as_object(obj.address)? };
        Some(o.object_path())
    }

    fn object_class_path(&self, obj: &ObjectRef) -> Option<String> {
        let o = unsafe { as_object(obj.address)? };
        let cls = unsafe { o.class_private().as_ref()? };
        let cls_base: &ue::UObjectBase =
            unsafe { &*(cls as *const ue::UClass as *const ue::UObjectBase) };
        Some(cls_base.object_path())
    }

    fn object_is_class(&self, obj: &ObjectRef) -> bool {
        let Some(o) = (unsafe { as_object(obj.address) }) else {
            return false;
        };
        o.is::<ue::UClass>()
    }

    // ── Struct/Class hierarchy ───────────────────────────────────────

    fn super_struct(&self, s: &StructRef) -> Option<StructRef> {
        let st = unsafe { as_struct(s.address)? };
        let parent = unsafe { st.super_struct().as_ref()? };
        Some(StructRef::new(Address(parent as *const _ as u64)))
    }

    fn struct_path(&self, s: &StructRef) -> Option<String> {
        let o = unsafe { as_object(s.address)? };
        Some(o.object_path())
    }

    // ── Function queries ─────────────────────────────────────────────

    fn function_return_type(&self, func: Address) -> Option<ExprType> {
        let f = unsafe { as_function(func)? };
        // Walk child_properties to find CPF_ReturnParm
        let mut prop_ptr = f.child_properties();
        while let Some(field) = unsafe { prop_ptr.as_ref() } {
            let fprop = unsafe { (field as *const ue::FField as *const ue::FProperty).as_ref()? };
            if fprop
                .property_flags()
                .contains(ue::EPropertyFlags::CPF_ReturnParm)
            {
                let addr = Address(fprop as *const _ as u64);
                return self.property_expr_type(&PropertyRef::new(addr));
            }
            prop_ptr = field.next();
        }
        None
    }

    fn function_call_kind(&self, func: Address) -> FunctionCallKind {
        let Some(f) = (unsafe { as_function(func) }) else {
            return FunctionCallKind::Virtual;
        };
        let flags = f.function_flags();

        let math_required = ue::EFunctionFlags::FUNC_Static
            | ue::EFunctionFlags::FUNC_Final
            | ue::EFunctionFlags::FUNC_Native;
        let net_flags = ue::EFunctionFlags::FUNC_Net
            | ue::EFunctionFlags::FUNC_NetReliable
            | ue::EFunctionFlags::FUNC_NetRequest
            | ue::EFunctionFlags::FUNC_NetResponse
            | ue::EFunctionFlags::FUNC_NetMulticast
            | ue::EFunctionFlags::FUNC_NetServer
            | ue::EFunctionFlags::FUNC_NetClient;

        if flags.contains(math_required) && !flags.intersects(net_flags) {
            return FunctionCallKind::Math;
        }

        let non_local = ue::EFunctionFlags::FUNC_Native
            | net_flags
            | ue::EFunctionFlags::FUNC_BlueprintAuthorityOnly
            | ue::EFunctionFlags::FUNC_BlueprintCosmetic;
        if !flags.intersects(non_local) {
            return FunctionCallKind::LocalFinal;
        }

        if flags.contains(ue::EFunctionFlags::FUNC_Final) {
            return FunctionCallKind::Final;
        }

        FunctionCallKind::Virtual
    }

    fn is_function_unambiguous(&self, func: Address, name: &str, scope: &StructRef) -> bool {
        // Walk the class hierarchy from scope, look for a child function with matching name
        let mut current = Some(*scope);
        while let Some(s) = current {
            let Some(st) = (unsafe { as_struct(s.address) }) else {
                break;
            };
            // Scan children (UField linked list) for a function with this name
            let mut child = st.children();
            while let Some(c) = unsafe { child.as_ref() } {
                let base: &ue::UObjectBase =
                    unsafe { &*(c as *const ue::UField as *const ue::UObjectBase) };
                if base.name_private().to_string() == name {
                    let found_addr = base as *const _ as u64;
                    return found_addr == func.0;
                }
                child = c.next();
            }
            current = self.super_struct(&s);
        }
        // Not found — fall back to qualified path
        false
    }

    // ── Context ──────────────────────────────────────────────────────

    fn current_class(&self) -> &str {
        &self.class_path
    }

    fn current_class_ref(&self) -> StructRef {
        StructRef::new(Address(self.class_addr))
    }

    fn is_ubergraph_function(&self, addr: Address) -> bool {
        let Some(f) = (unsafe { as_function(addr) }) else {
            return false;
        };
        f.function_flags()
            .contains(ue::EFunctionFlags::FUNC_UbergraphFunction)
    }

    // ── Reverse lookups ─────────────────────────────────────────────

    fn resolve_property(
        &self,
        name: &str,
        kind: PropertyKind,
        function_path: &str,
    ) -> Result<PropertyRef, ResolutionError> {
        // Determine which UStruct(s) to search based on property kind
        let result = match kind {
            PropertyKind::Local | PropertyKind::Out => {
                resolve_path_to_struct(function_path).and_then(|s| find_property_in_struct(s, name))
            }
            PropertyKind::Instance | PropertyKind::Default | PropertyKind::Sparse => {
                resolve_path_to_struct(&self.class_path)
                    .and_then(|s| find_property_in_struct(s, name))
            }
            PropertyKind::Other => resolve_path_to_struct(function_path)
                .and_then(|s| find_property_in_struct(s, name))
                .or_else(|| {
                    resolve_path_to_struct(&self.class_path)
                        .and_then(|s| find_property_in_struct(s, name))
                }),
        };

        result.map(PropertyRef::new).ok_or_else(|| ResolutionError {
            message: format!("unknown property '{name}' ({kind:?})"),
        })
    }

    fn resolve_path(&self, path: &str) -> Result<Address, ResolutionError> {
        resolve_path_to_address(path).ok_or_else(|| ResolutionError {
            message: format!("unknown path '{path}'"),
        })
    }

    fn resolve_property_of(&self, name: &str, owner: &str) -> Result<PropertyRef, ResolutionError> {
        let ustruct = resolve_path_to_struct(owner).ok_or_else(|| ResolutionError {
            message: format!("unknown owner '{owner}'"),
        })?;
        find_property_in_struct(ustruct, name)
            .map(PropertyRef::new)
            .ok_or_else(|| ResolutionError {
                message: format!("unknown property '{name}' in '{owner}'"),
            })
    }

    fn resolve_function_in(&self, name: &str, owner: &str) -> Result<Address, ResolutionError> {
        let ustruct = resolve_path_to_struct(owner).ok_or_else(|| ResolutionError {
            message: format!("unknown owner '{owner}'"),
        })?;
        find_child_in_hierarchy(ustruct, name).ok_or_else(|| ResolutionError {
            message: format!("unknown function '{name}' in '{owner}'"),
        })
    }
}

// ── LiveNameMap ──────────────────────────────────────────────────────────

/// FName resolution backed by the live UE name table.
pub struct LiveNameMap;

impl jmap_kismet_test::bytecode::reader::NameMap for LiveNameMap {
    fn resolve(&self, display_index: u32, number: u32) -> String {
        let name = ue::FName {
            comparison_index: ue::FNameEntryId {
                value: display_index,
            },
            number,
        };
        name.to_string()
    }

    fn lookup(&self, name: &str) -> (u32, u32, u32) {
        let fname = ue::FName::find(name);
        let idx = fname.comparison_index.value;
        (idx, idx, fname.number)
    }
}
