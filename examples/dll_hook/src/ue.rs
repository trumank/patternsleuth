use std::{
    cell::UnsafeCell,
    convert::Infallible,
    ffi::c_void,
    fmt::Display,
    ops::{Deref, DerefMut},
};

use windows::Win32::System::Threading::{
    EnterCriticalSection, LeaveCriticalSection, CRITICAL_SECTION,
};

use crate::globals;

macro_rules! impl_deref {
    ($class:ty, $member:ident: $parent:ty) => {
        impl std::ops::Deref for $class {
            type Target = $parent;
            fn deref(&self) -> &Self::Target {
                &self.$member
            }
        }
        impl std::ops::DerefMut for $class {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$member
            }
        }
    };
}

pub type FnFFrameStep =
    unsafe extern "system" fn(stack: &mut kismet::FFrame, *mut UObject, result: *mut c_void);
pub type FnFFrameStepExplicitProperty = unsafe extern "system" fn(
    stack: &mut kismet::FFrame,
    result: *mut c_void,
    property: *const FProperty,
);

pub type FnFNameToString = unsafe extern "system" fn(&FName, &mut FString);
impl Display for FName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut string = FString::new();
        unsafe {
            (globals().fname_to_string())(self, &mut string);
        };
        write!(f, "{string}")
    }
}

pub type FnUObjectBaseUtilityGetPathName =
    unsafe extern "system" fn(&UObjectBase, Option<&UObject>, &mut FString);
impl UObjectBase {
    // pub fn get_path_name(&self, stop_outer: Option<&UObject>) -> String {
    //     let mut string = FString::new();
    //     unsafe {
    //         (globals().uobject_base_utility_get_path_name())(self, stop_outer, &mut string);
    //     }
    //     string.to_string()
    // }
}

#[derive(Debug)]
#[repr(C)]
pub struct FMalloc {
    vtable: *const FMallocVTable,
}
unsafe impl Sync for FMalloc {}
unsafe impl Send for FMalloc {}
impl FMalloc {
    pub fn malloc(&self, count: usize, alignment: u32) -> *mut c_void {
        unsafe { ((*self.vtable).malloc)(self, count, alignment) }
    }
    pub fn realloc(&self, original: *mut c_void, count: usize, alignment: u32) -> *mut c_void {
        unsafe { ((*self.vtable).realloc)(self, original, count, alignment) }
    }
    pub fn free(&self, original: *mut c_void) {
        unsafe { ((*self.vtable).free)(self, original) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FMallocVTable {
    pub __vec_del_dtor: *const (),
    pub exec: *const (),
    pub malloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub try_malloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub realloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub try_realloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub free: unsafe extern "system" fn(this: &FMalloc, original: *mut c_void),
    pub quantize_size: *const (),
    pub get_allocation_size: *const (),
    pub trim: *const (),
    pub setup_tls_caches_on_current_thread: *const (),
    pub clear_and_disable_tlscaches_on_current_thread: *const (),
    pub initialize_stats_metadata: *const (),
    pub update_stats: *const (),
    pub get_allocator_stats: *const (),
    pub dump_allocator_stats: *const (),
    pub is_internally_thread_safe: *const (),
    pub validate_heap: *const (),
    pub get_descriptive_name: *const (),
}

#[derive(Debug)]
#[repr(C)]
pub struct FWindowsCriticalSection(UnsafeCell<CRITICAL_SECTION>);
impl FWindowsCriticalSection {
    fn crit_ptr_mut(&self) -> *mut CRITICAL_SECTION {
        &self.0 as *const _ as *mut _
    }
    unsafe fn lock(&self) {
        tracing::info!("LOCKING objects");
        EnterCriticalSection(self.crit_ptr_mut());
    }
    unsafe fn unlock(&self) {
        tracing::info!("UNLOCKING objects");
        LeaveCriticalSection(self.crit_ptr_mut());
    }
}

pub struct CriticalSectionGuard<'crit, 'data, T: ?Sized + 'data> {
    critical_section: &'crit FWindowsCriticalSection,
    data: &'data UnsafeCell<T>,
}
impl<'crit, 'data, T: ?Sized> CriticalSectionGuard<'crit, 'data, T> {
    fn lock(critical_section: &'crit FWindowsCriticalSection, data: &'data UnsafeCell<T>) -> Self {
        unsafe {
            critical_section.lock();
        }
        Self {
            critical_section,
            data,
        }
    }
}
impl<T: ?Sized> Drop for CriticalSectionGuard<'_, '_, T> {
    fn drop(&mut self) {
        unsafe { self.critical_section.unlock() }
    }
}
impl<T: ?Sized> Deref for CriticalSectionGuard<'_, '_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.data.get() }
    }
}
impl<T: ?Sized> DerefMut for CriticalSectionGuard<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectCreateListener;

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectDeleteListener;

type ObjectIndex = i32;

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectArray {
    obj_first_gcindex: i32,
    obj_last_non_gcindex: i32,
    max_objects_not_considered_by_gc: i32,
    open_for_disregard_for_gc: bool,

    obj_objects: UnsafeCell<FChunkedFixedUObjectArray>,
    obj_objects_critical: FWindowsCriticalSection,
    obj_available_list: [u8; 0x88],
    uobject_create_listeners: TArray<*const FUObjectCreateListener>,
    uobject_delete_listeners: TArray<*const FUObjectDeleteListener>,
    uobject_delete_listeners_critical: FWindowsCriticalSection,
    master_serial_number: std::sync::atomic::AtomicI32,
}
impl FUObjectArray {
    pub fn objects(&self) -> CriticalSectionGuard<'_, '_, FChunkedFixedUObjectArray> {
        CriticalSectionGuard::lock(&self.obj_objects_critical, &self.obj_objects)
    }
    pub fn allocate_serial_number(&self, index: ObjectIndex) -> i32 {
        use std::sync::atomic::Ordering;

        let objects = unsafe { &*self.obj_objects.get() };
        let item = objects.item(index);

        let current = item.serial_number.load(Ordering::SeqCst);
        if current != 0 {
            current
        } else {
            let new = self.master_serial_number.fetch_add(1, Ordering::SeqCst);

            let exchange =
                item.serial_number
                    .compare_exchange(0, new, Ordering::SeqCst, Ordering::SeqCst);
            match exchange {
                Ok(_) => new,
                Err(old) => old,
            }
        }
    }
}

pub struct ObjectIterator<'a> {
    array: &'a FChunkedFixedUObjectArray,
    index: i32,
}
impl<'a> Iterator for ObjectIterator<'a> {
    type Item = Option<&'a UObjectBase>;
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.array.num_elements as usize;
        (size, Some(size))
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let n = n as i32;
        if self.index < n {
            self.index = n;
        }
        self.next()
    }
    fn next(&mut self) -> Option<Option<&'a UObjectBase>> {
        if self.index >= self.array.num_elements {
            None
        } else {
            let obj = unsafe { self.array.item(self.index).object.as_ref() };

            self.index += 1;
            Some(obj)
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FChunkedFixedUObjectArray {
    pub objects: *const *const FUObjectItem,
    pub pre_allocated_objects: *const FUObjectItem,
    pub max_elements: i32,
    pub num_elements: i32,
    pub max_chunks: i32,
    pub num_chunks: i32,
}
impl FChunkedFixedUObjectArray {
    pub fn iter(&self) -> ObjectIterator<'_> {
        ObjectIterator {
            array: self,
            index: 0,
        }
    }
    fn item_ptr(&self, index: ObjectIndex) -> *const FUObjectItem {
        let per_chunk = self.max_elements / self.max_chunks;

        unsafe {
            (*self.objects.add((index / per_chunk) as usize)).add((index % per_chunk) as usize)
        }
    }
    fn item(&self, index: ObjectIndex) -> &FUObjectItem {
        unsafe { &*self.item_ptr(index) }
    }
    fn item_mut(&mut self, index: ObjectIndex) -> &mut FUObjectItem {
        unsafe { &mut *(self.item_ptr(index) as *mut FUObjectItem) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectItem {
    pub object: *const UObjectBase,
    pub flags: i32,
    pub cluster_root_index: i32,
    pub serial_number: std::sync::atomic::AtomicI32,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FWeakObjectPtr {
    object_index: i32,
    object_serial_number: i32,
}
impl FWeakObjectPtr {
    pub fn new(object: &UObjectBase) -> Self {
        Self::new_from_index(object.internal_index)
    }
    pub fn new_from_index(index: ObjectIndex) -> Self {
        Self {
            object_index: index,
            // serial allocation performs only atomic operations
            object_serial_number: unsafe {
                globals()
                    .guobject_array_unchecked()
                    .allocate_serial_number(index)
            },
        }
    }
    pub fn get(&self, object_array: &FUObjectArray) -> Option<&UObjectBase> {
        // TODO check valid
        unsafe {
            let objects = &*object_array.obj_objects.get();
            let item = objects.item(self.object_index);
            Some(&*item.object)
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone)]
    pub struct EObjectFlags: u32 {
        const RF_NoFlags = 0x0000;
        const RF_Public = 0x0001;
        const RF_Standalone = 0x0002;
        const RF_MarkAsNative = 0x0004;
        const RF_Transactional = 0x0008;
        const RF_ClassDefaultObject = 0x0010;
        const RF_ArchetypeObject = 0x0020;
        const RF_Transient = 0x0040;
        const RF_MarkAsRootSet = 0x0080;
        const RF_TagGarbageTemp = 0x0100;
        const RF_NeedInitialization = 0x0200;
        const RF_NeedLoad = 0x0400;
        const RF_KeepForCooker = 0x0800;
        const RF_NeedPostLoad = 0x1000;
        const RF_NeedPostLoadSubobjects = 0x2000;
        const RF_NewerVersionExists = 0x4000;
        const RF_BeginDestroyed = 0x8000;
        const RF_FinishDestroyed = 0x00010000;
        const RF_BeingRegenerated = 0x00020000;
        const RF_DefaultSubObject = 0x00040000;
        const RF_WasLoaded = 0x00080000;
        const RF_TextExportTransient = 0x00100000;
        const RF_LoadCompleted = 0x00200000;
        const RF_InheritableComponentTemplate = 0x00400000;
        const RF_DuplicateTransient = 0x00800000;
        const RF_StrongRefOnFrame = 0x01000000;
        const RF_NonPIEDuplicateTransient = 0x02000000;
        const RF_Dynamic = 0x04000000;
        const RF_WillBeLoaded = 0x08000000;
    }
}
bitflags::bitflags! {
    #[derive(Debug, Clone)]
    pub struct EFunctionFlags: u32 {
        const FUNC_None = 0x0000;
        const FUNC_Final = 0x0001;
        const FUNC_RequiredAPI = 0x0002;
        const FUNC_BlueprintAuthorityOnly = 0x0004;
        const FUNC_BlueprintCosmetic = 0x0008;
        const FUNC_Net = 0x0040;
        const FUNC_NetReliable = 0x0080;
        const FUNC_NetRequest = 0x0100;
        const FUNC_Exec = 0x0200;
        const FUNC_Native = 0x0400;
        const FUNC_Event = 0x0800;
        const FUNC_NetResponse = 0x1000;
        const FUNC_Static = 0x2000;
        const FUNC_NetMulticast = 0x4000;
        const FUNC_UbergraphFunction = 0x8000;
        const FUNC_MulticastDelegate = 0x00010000;
        const FUNC_Public = 0x00020000;
        const FUNC_Private = 0x00040000;
        const FUNC_Protected = 0x00080000;
        const FUNC_Delegate = 0x00100000;
        const FUNC_NetServer = 0x00200000;
        const FUNC_HasOutParms = 0x00400000;
        const FUNC_HasDefaults = 0x00800000;
        const FUNC_NetClient = 0x01000000;
        const FUNC_DLLImport = 0x02000000;
        const FUNC_BlueprintCallable = 0x04000000;
        const FUNC_BlueprintEvent = 0x08000000;
        const FUNC_BlueprintPure = 0x10000000;
        const FUNC_EditorOnly = 0x20000000;
        const FUNC_Const = 0x40000000;
        const FUNC_NetValidate = 0x80000000;
        const FUNC_AllFlags = 0xffffffff;
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct EClassFlags: i32 {
        const CLASS_None = 0x0000;
        const CLASS_Abstract = 0x0001;
        const CLASS_DefaultConfig = 0x0002;
        const CLASS_Config = 0x0004;
        const CLASS_Transient = 0x0008;
        const CLASS_Parsed = 0x0010;
        const CLASS_MatchedSerializers = 0x0020;
        const CLASS_ProjectUserConfig = 0x0040;
        const CLASS_Native = 0x0080;
        const CLASS_NoExport = 0x0100;
        const CLASS_NotPlaceable = 0x0200;
        const CLASS_PerObjectConfig = 0x0400;
        const CLASS_ReplicationDataIsSetUp = 0x0800;
        const CLASS_EditInlineNew = 0x1000;
        const CLASS_CollapseCategories = 0x2000;
        const CLASS_Interface = 0x4000;
        const CLASS_CustomConstructor = 0x8000;
        const CLASS_Const = 0x00010000;
        const CLASS_LayoutChanging = 0x00020000;
        const CLASS_CompiledFromBlueprint = 0x00040000;
        const CLASS_MinimalAPI = 0x00080000;
        const CLASS_RequiredAPI = 0x00100000;
        const CLASS_DefaultToInstanced = 0x00200000;
        const CLASS_TokenStreamAssembled = 0x00400000;
        const CLASS_HasInstancedReference = 0x00800000;
        const CLASS_Hidden = 0x01000000;
        const CLASS_Deprecated = 0x02000000;
        const CLASS_HideDropDown = 0x04000000;
        const CLASS_GlobalUserConfig = 0x08000000;
        const CLASS_Intrinsic = 0x10000000;
        const CLASS_Constructed = 0x20000000;
        const CLASS_ConfigDoNotCheckDefaults = 0x40000000;
        const CLASS_NewerVersionExists = i32::MIN;
    }


    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct EClassCastFlags : u64 {
        const CASTCLASS_None = 0x0000000000000000;
        const CASTCLASS_UField = 0x0000000000000001;
        const CASTCLASS_FInt8Property = 0x0000000000000002;
        const CASTCLASS_UEnum = 0x0000000000000004;
        const CASTCLASS_UStruct = 0x0000000000000008;
        const CASTCLASS_UScriptStruct = 0x0000000000000010;
        const CASTCLASS_UClass = 0x0000000000000020;
        const CASTCLASS_FByteProperty = 0x0000000000000040;
        const CASTCLASS_FIntProperty = 0x0000000000000080;
        const CASTCLASS_FFloatProperty = 0x0000000000000100;
        const CASTCLASS_FUInt64Property = 0x0000000000000200;
        const CASTCLASS_FClassProperty = 0x0000000000000400;
        const CASTCLASS_FUInt32Property = 0x0000000000000800;
        const CASTCLASS_FInterfaceProperty = 0x0000000000001000;
        const CASTCLASS_FNameProperty = 0x0000000000002000;
        const CASTCLASS_FStrProperty = 0x0000000000004000;
        const CASTCLASS_FProperty = 0x0000000000008000;
        const CASTCLASS_FObjectProperty = 0x0000000000010000;
        const CASTCLASS_FBoolProperty = 0x0000000000020000;
        const CASTCLASS_FUInt16Property = 0x0000000000040000;
        const CASTCLASS_UFunction = 0x0000000000080000;
        const CASTCLASS_FStructProperty = 0x0000000000100000;
        const CASTCLASS_FArrayProperty = 0x0000000000200000;
        const CASTCLASS_FInt64Property = 0x0000000000400000;
        const CASTCLASS_FDelegateProperty = 0x0000000000800000;
        const CASTCLASS_FNumericProperty = 0x0000000001000000;
        const CASTCLASS_FMulticastDelegateProperty = 0x0000000002000000;
        const CASTCLASS_FObjectPropertyBase = 0x0000000004000000;
        const CASTCLASS_FWeakObjectProperty = 0x0000000008000000;
        const CASTCLASS_FLazyObjectProperty = 0x0000000010000000;
        const CASTCLASS_FSoftObjectProperty = 0x0000000020000000;
        const CASTCLASS_FTextProperty = 0x0000000040000000;
        const CASTCLASS_FInt16Property = 0x0000000080000000;
        const CASTCLASS_FDoubleProperty = 0x0000000100000000;
        const CASTCLASS_FSoftClassProperty = 0x0000000200000000;
        const CASTCLASS_UPackage = 0x0000000400000000;
        const CASTCLASS_ULevel = 0x0000000800000000;
        const CASTCLASS_AActor = 0x0000001000000000;
        const CASTCLASS_APlayerController = 0x0000002000000000;
        const CASTCLASS_APawn = 0x0000004000000000;
        const CASTCLASS_USceneComponent = 0x0000008000000000;
        const CASTCLASS_UPrimitiveComponent = 0x0000010000000000;
        const CASTCLASS_USkinnedMeshComponent = 0x0000020000000000;
        const CASTCLASS_USkeletalMeshComponent = 0x0000040000000000;
        const CASTCLASS_UBlueprint = 0x0000080000000000;
        const CASTCLASS_UDelegateFunction = 0x0000100000000000;
        const CASTCLASS_UStaticMeshComponent = 0x0000200000000000;
        const CASTCLASS_FMapProperty = 0x0000400000000000;
        const CASTCLASS_FSetProperty = 0x0000800000000000;
        const CASTCLASS_FEnumProperty = 0x0001000000000000;
        const CASTCLASS_USparseDelegateFunction = 0x0002000000000000;
        const CASTCLASS_FMulticastInlineDelegateProperty = 0x0004000000000000;
        const CASTCLASS_FMulticastSparseDelegateProperty = 0x0008000000000000;
        const CASTCLASS_FFieldPathProperty = 0x0010000000000000;
        const CASTCLASS_FLargeWorldCoordinatesRealProperty = 0x0080000000000000;
        const CASTCLASS_FOptionalProperty = 0x0100000000000000;
        const CASTCLASS_FVerseValueProperty = 0x0200000000000000;
        const CASTCLASS_UVerseVMClass = 0x0400000000000000;
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct  EPropertyFlags: u64 {
        const CPF_None = 0x0000;
        const CPF_Edit = 0x0001;
        const CPF_ConstParm = 0x0002;
        const CPF_BlueprintVisible = 0x0004;
        const CPF_ExportObject = 0x0008;
        const CPF_BlueprintReadOnly = 0x0010;
        const CPF_Net = 0x0020;
        const CPF_EditFixedSize = 0x0040;
        const CPF_Parm = 0x0080;
        const CPF_OutParm = 0x0100;
        const CPF_ZeroConstructor = 0x0200;
        const CPF_ReturnParm = 0x0400;
        const CPF_DisableEditOnTemplate = 0x0800;
        const CPF_NonNullable = 0x1000;
        const CPF_Transient = 0x2000;
        const CPF_Config = 0x4000;
        const CPF_RequiredParm = 0x8000;
        const CPF_DisableEditOnInstance = 0x00010000;
        const CPF_EditConst = 0x00020000;
        const CPF_GlobalConfig = 0x00040000;
        const CPF_InstancedReference = 0x00080000;
        const CPF_ExperimentalExternalObjects = 0x00100000;
        const CPF_DuplicateTransient = 0x00200000;
        const CPF_SaveGame = 0x01000000;
        const CPF_NoClear = 0x02000000;
        const CPF_Virtual = 0x04000000;
        const CPF_ReferenceParm = 0x08000000;
        const CPF_BlueprintAssignable = 0x10000000;
        const CPF_Deprecated = 0x20000000;
        const CPF_IsPlainOldData = 0x40000000;
        const CPF_RepSkip = 0x80000000;
        const CPF_RepNotify = 0x100000000;
        const CPF_Interp = 0x200000000;
        const CPF_NonTransactional = 0x400000000;
        const CPF_EditorOnly = 0x800000000;
        const CPF_NoDestructor = 0x1000000000;
        const CPF_AutoWeak = 0x4000000000;
        const CPF_ContainsInstancedReference = 0x8000000000;
        const CPF_AssetRegistrySearchable = 0x10000000000;
        const CPF_SimpleDisplay = 0x20000000000;
        const CPF_AdvancedDisplay = 0x40000000000;
        const CPF_Protected = 0x80000000000;
        const CPF_BlueprintCallable = 0x100000000000;
        const CPF_BlueprintAuthorityOnly = 0x200000000000;
        const CPF_TextExportTransient = 0x400000000000;
        const CPF_NonPIEDuplicateTransient = 0x800000000000;
        const CPF_ExposeOnSpawn = 0x1000000000000;
        const CPF_PersistentInstance = 0x2000000000000;
        const CPF_UObjectWrapper = 0x4000000000000;
        const CPF_HasGetValueTypeHash = 0x8000000000000;
        const CPF_NativeAccessSpecifierPublic = 0x10000000000000;
        const CPF_NativeAccessSpecifierProtected = 0x20000000000000;
        const CPF_NativeAccessSpecifierPrivate = 0x40000000000000;
        const CPF_SkipSerialization = 0x80000000000000;
        const CPF_TObjectPtr = 0x100000000000000;
        const CPF_ExperimentalOverridableLogic = 0x200000000000000;
        const CPF_ExperimentalAlwaysOverriden = 0x400000000000000;
        const CPF_ExperimentalNeverOverriden = 0x800000000000000;
        const CPF_AllowSelfReference = 0x1000000000000000;
    }
}

// unsafe because wrong cast flags can result in UB
unsafe trait ObjTrait {
    const CAST_FLAGS: EClassCastFlags;
}
// unsafe because wrong cast flags can result in UB
pub unsafe trait FieldTrait {
    const CAST_FLAGS: EClassCastFlags;
    type PropValue;
    fn cast<T: FieldTrait>(&self) -> Option<&T>
    where
        Self: std::ops::Deref<Target = FFieldBase>,
    {
        FFieldBase::cast(self)
    }
}
trait PropTrait: FieldTrait + Deref<Target = FProperty> {
    unsafe fn value<'o>(&self, object: &'o UObjectBase) -> &'o Self::PropValue {
        &*std::ptr::from_ref(object)
            .byte_offset(self.offset_internal as isize)
            .cast()
    }
    unsafe fn value_mut<'o>(&self, object: &'o mut UObjectBase) -> &'o mut Self::PropValue {
        &mut *std::ptr::from_mut(object)
            .byte_offset(self.offset_internal as isize)
            .cast()
    }
    unsafe fn value_mut_ptr<'o>(&self, object: *mut UObjectBase) -> &'o mut Self::PropValue {
        &mut *object.byte_offset(self.offset_internal as isize).cast()
    }
}
impl<T> PropTrait for T where T: FieldTrait + Deref<Target = FProperty> {}

#[derive(Debug)]
#[repr(C)]
pub struct UObjectBase {
    pub vtable: *const c_void,
    pub object_flags: EObjectFlags,
    pub internal_index: i32,
    pub class_private: *const UClass,
    pub name_private: FName,
    pub outer_private: *const UObject,
}

impl UObjectBase {
    pub fn class(&self) -> &UClass {
        // TODO thread safe class access? i guess if we have a reference to the containing object it's fine?
        unsafe { self.class_private.as_ref().unwrap() }
    }
    pub fn is<T: ObjTrait>(&self) -> bool {
        self.class().class_cast_flags.contains(T::CAST_FLAGS)
    }
    pub fn cast<T: ObjTrait>(&self) -> Option<&T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&Self, &T>(self) })
    }
    pub fn cast_mut<T: ObjTrait>(&mut self) -> Option<&mut T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&mut Self, &mut T>(self) })
    }
    pub fn path(&self) -> String {
        let mut path = String::new();

        let class = unsafe { self.class_private.as_ref().unwrap() };

        path.push_str(&class.name_private.to_string());
        path.push(' ');
        self.append_path(&mut path);
        path
    }
    fn append_path(&self, path: &mut String) {
        if let Some(outer) = unsafe { self.outer_private.as_ref() } {
            outer.append_path(path);
            path.push('.');
        }
        path.push_str(&self.name_private.to_string())
    }
    pub fn props<'o>(&'o self) -> IterFieldsBound<'o> {
        let inner = self.class().iter_props();
        IterFieldsBound {
            object: self,
            inner,
        }
    }
    pub fn props_mut<'o>(&'o mut self) -> IterFieldsBoundMut<'o> {
        let inner = unsafe { &*std::ptr::from_ref(self.class()) }.iter_props();
        IterFieldsBoundMut {
            object: self,
            inner,
        }
    }
}

pub struct IterFieldsBound<'o> {
    object: &'o UObjectBase,
    inner: IterProps<'o>,
}
impl<'o> Iterator for IterFieldsBound<'o> {
    type Item = BoundField<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|f| BoundField {
            object: self.object,
            field: f,
        })
    }
}
pub struct BoundField<'o> {
    object: &'o UObjectBase,
    pub field: &'o FField,
}
impl<'o> BoundField<'o> {
    pub fn get<P: PropTrait>(&self) -> Option<&'o <P as FieldTrait>::PropValue> {
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value(self.object) })
    }
}

pub struct IterFieldsBoundMut<'o> {
    object: &'o mut UObjectBase,
    inner: IterProps<'o>,
}
impl<'o> Iterator for IterFieldsBoundMut<'o> {
    type Item = BoundFieldMut<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|f| BoundFieldMut {
            object: std::ptr::from_mut(self.object),
            field: f,
        })
    }
}
pub struct BoundFieldMut<'o> {
    object: *mut UObjectBase,
    pub field: &'o FField,
}
impl<'o> BoundFieldMut<'o> {
    pub fn get<P: PropTrait>(&mut self) -> Option<&'o mut <P as FieldTrait>::PropValue> {
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value_mut_ptr(self.object) })
    }
}

impl_deref!(UObjectBaseUtility, uobject_base: UObjectBase);
#[derive(Debug)]
#[repr(C)]
pub struct UObjectBaseUtility {
    pub uobject_base: UObjectBase,
}

impl_deref!(UObject, uobject_base_utility: UObjectBaseUtility);
unsafe impl ObjTrait for UObject {
    // TODO not sure if this works? special case UObject?
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_None;
}
#[derive(Debug)]
#[repr(C)]
pub struct UObject {
    pub uobject_base_utility: UObjectBaseUtility,
}

#[derive(Debug)]
#[repr(C)]
struct FOutputDevice {
    vtable: *const c_void,
    b_suppress_event_tag: bool,
    b_auto_emit_line_terminator: bool,
}

impl_deref!(UField, uobject: UObject);
unsafe impl ObjTrait for UField {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_UField;
}
#[derive(Debug)]
#[repr(C)]
pub struct UField {
    pub uobject: UObject,
    pub next: *const UField,
}

#[derive(Debug)]
#[repr(C)]
pub struct FStructBaseChain {
    pub struct_base_chain_array: *const *const FStructBaseChain,
    pub num_struct_bases_in_chain_minus_one: i32,
}

#[derive(Debug)]
#[repr(C)]
struct FFieldClass {
    pub name: FName,
    id: u64,
    cast_flags: EClassCastFlags, // u64
    class_flags: EClassFlags,
    super_class: *const FFieldClass,
    default_object: *const FField,
    // TODO
    // ConstructFn: extern "system" fn(*const [const] FFieldVariant, *const [const] FName, EObjectFlags) -> *const FField,
    // UnqiueNameIndexCounter: FThreadSafeCounter,
}

#[derive(Debug)]
#[repr(C)]
struct FFieldVariant {
    container: *const c_void,
    b_is_uobject: bool,
}

unsafe impl FieldTrait for FFieldBase {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_None;
    type PropValue = Infallible;
}
#[derive(Debug)]
#[repr(C)]
pub struct FFieldBase {
    vtable: *const (),
    class_private: *const FFieldClass,
    owner: FFieldVariant,
    next: *const FField,
    name_private: FName,
    flags_private: EObjectFlags,
}
impl FFieldBase {
    pub fn class(&self) -> &FFieldClass {
        unsafe { self.class_private.as_ref().unwrap() }
    }
    pub fn is<T: FieldTrait>(&self) -> bool {
        self.class().cast_flags.contains(T::CAST_FLAGS)
    }
    pub fn cast<T: FieldTrait>(&self) -> Option<&T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&Self, &T>(self) })
    }
    pub fn name(&self) -> FName {
        self.name_private
    }
}
impl_deref!(FField, base: FFieldBase);
unsafe impl FieldTrait for FField {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_None;
    type PropValue = Infallible;
}
#[derive(Debug)]
#[repr(C)]
pub struct FField {
    base: FFieldBase,
}

impl_deref!(FProperty, ffield: FField);
unsafe impl FieldTrait for FProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FProperty;
    type PropValue = Infallible;
}
#[derive(Debug)]
#[repr(C)]
pub struct FProperty {
    ffield: FField,
    array_dim: i32,
    element_size: i32,
    property_flags: EPropertyFlags,
    rep_index: u16,
    blueprint_replication_condition: u16, // TEnumAsByte<enum ELifetimeCondition>,
    offset_internal: i32,
    rep_notify_func: FName,
    property_link_next: *const FProperty,
    next_ref: *const FProperty,
    destructor_link_next: *const FProperty,
    post_construct_link_next: *const FProperty,
}

impl FProperty {
    pub fn offset(&self) -> i32 {
        self.offset_internal
    }
}

macro_rules! impl_basic_prop {
    ($name:ident, $value:ty, $cast_flag:ident) => {
        impl_deref!($name, fproperty: FProperty);
        unsafe impl FieldTrait for $name {
            const CAST_FLAGS: EClassCastFlags = EClassCastFlags::$cast_flag;
            type PropValue = $value;
        }
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            fproperty: FProperty,
        }
    };
}
impl_basic_prop!(FStrProperty, FString, CASTCLASS_FStrProperty);
impl_basic_prop!(FNameProperty, FName, CASTCLASS_FNameProperty);
impl_basic_prop!(FInt8Property, i8, CASTCLASS_FInt8Property);
impl_basic_prop!(FInt16Property, i16, CASTCLASS_FInt16Property);
impl_basic_prop!(FIntProperty, i32, CASTCLASS_FIntProperty);
impl_basic_prop!(FInt64Property, i64, CASTCLASS_FInt64Property);
impl_basic_prop!(FByteProperty, u8, CASTCLASS_FByteProperty); // TODO enum
impl_basic_prop!(FUInt16Property, u16, CASTCLASS_FUInt16Property);
impl_basic_prop!(FUInt32Property, u32, CASTCLASS_FUInt32Property);
impl_basic_prop!(FUInt64Property, u64, CASTCLASS_FUInt64Property);
impl_basic_prop!(FFloatProperty, f32, CASTCLASS_FFloatProperty);
impl_basic_prop!(FDoubleProperty, f64, CASTCLASS_FDoubleProperty);

impl_deref!(UStruct, ufield: UField);
unsafe impl ObjTrait for UStruct {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_UStruct;
}
#[derive(Debug)]
#[repr(C)]
pub struct UStruct {
    pub ufield: UField,
    pub fstruct_base_chain: FStructBaseChain,
    pub super_struct: *const UStruct,
    pub children: *const UField,
    pub child_properties: *const FField,
    pub properties_size: i32,
    pub min_alignment: i32,
    pub script: TArray<u8>,
    pub property_link: *const FProperty,
    pub ref_link: *const FProperty,
    pub destructor_link: *const FProperty,
    pub post_construct_link: *const FProperty,
    pub script_and_property_object_references: TArray<*const UObject>,
    pub unresolved_script_properties: *const (), //TODO pub TArray<TTuple<TFieldPath<FField>,int>,TSizedDefaultAllocator<32> >*
    pub unversioned_schema: *const (),           //TODO const FUnversionedStructSchema*
}

impl UStruct {
    pub fn iter_props<'o>(&'o self) -> IterProps<'o> {
        IterProps {
            r#struct: self,
            next_super: self,
            next_field: std::ptr::null(),
        }
    }
}

pub struct IterProps<'o> {
    r#struct: &'o UStruct,
    next_super: *const UStruct,
    next_field: *const FField,
}
impl<'o> Iterator for IterProps<'o> {
    type Item = &'o FField;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(f) = unsafe { self.next_field.as_ref() } {
                self.next_field = f.next;
                return Some(f);
            } else if let Some(s) = unsafe { self.next_super.as_ref() } {
                self.next_field = s.child_properties;
                self.next_super = s.super_struct;
            } else {
                return None;
            }
        }
    }
}

impl_deref!(UFunction, ustruct: UStruct);
unsafe impl ObjTrait for UFunction {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_UFunction;
}
#[derive(Debug)]
#[repr(C)]
pub struct UFunction {
    pub ustruct: UStruct,
    pub function_flags: EFunctionFlags,
    pub num_parms: u8,
    pub parms_size: u16,
    pub return_value_offset: u16,
    pub rpc_id: u16,
    pub rpc_response_id: u16,
    pub first_property_to_init: *const FProperty,
    pub event_graph_function: *const UFunction,
    pub event_graph_call_offset: i32,
    pub func: unsafe extern "system" fn(*mut UObject, *mut kismet::FFrame, *mut c_void),
}

impl_deref!(UClass, ustruct: UStruct);
unsafe impl ObjTrait for UClass {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_UClass;
}
#[derive(Debug)]
#[repr(C)]
pub struct UClass {
    pub ustruct: UStruct,
    pub class_constructor: *const (), // extern "system" fn(*const [const] FObjectInitializer),
    pub class_vtable_helper_ctor_caller: *const (), // extern "system" fn(*const FVTableHelper) -> *const UObject,
    pub cpp_class_static_functions: *const (),      // FUObjectCppClassStaticFunctions,
    pub class_unique: i32,
    pub first_owned_class_rep: i32,
    pub cooked: bool,
    pub layout_changing: bool,
    pub class_flags: EClassFlags,
    pub class_cast_flags: EClassCastFlags,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FName {
    pub comparison_index: FNameEntryId,
    pub number: u32,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FNameEntryId {
    pub value: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct TSharedPtr<T> {
    pub object: *const T,
    pub reference_controller: *const FReferenceControllerBase,
}

#[derive(Debug)]
#[repr(C)]
pub struct FReferenceControllerBase {
    pub shared_reference_count: i32,
    pub weak_reference_count: i32,
}

pub type FString = TArray<u16>;
impl From<&str> for FString {
    fn from(value: &str) -> Self {
        Self::from(
            widestring::U16CString::from_str(value)
                .unwrap()
                .as_slice_with_nul(),
        )
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct TArray<T> {
    data: *const T,
    num: i32,
    max: i32,
}
impl<T> TArray<T> {
    fn new() -> Self {
        Self {
            data: std::ptr::null(),
            num: 0,
            max: 0,
        }
    }
}
impl<T> Drop for TArray<T> {
    fn drop(&mut self) {
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.data.cast_mut(),
                self.num as usize,
            ))
        }
        globals().gmalloc().free(self.data as *mut c_void);
    }
}
impl<T> Default for TArray<T> {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            num: 0,
            max: 0,
        }
    }
}
impl<T> TArray<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: globals().gmalloc().malloc(
                capacity * std::mem::size_of::<T>(),
                std::mem::align_of::<T>() as u32,
            ) as *const T,
            num: 0,
            max: capacity as i32,
        }
    }
    pub fn len(&self) -> usize {
        self.num as usize
    }
    pub fn capacity(&self) -> usize {
        self.max as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn as_slice(&self) -> &[T] {
        if self.num == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.data, self.num as usize) }
        }
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if self.num == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.data as *mut _, self.num as usize) }
        }
    }
    pub fn clear(&mut self) {
        let elems: *mut [T] = self.as_mut_slice();

        unsafe {
            self.num = 0;
            std::ptr::drop_in_place(elems);
        }
    }
    pub fn push(&mut self, new_value: T) {
        if self.num >= self.max {
            self.max = u32::next_power_of_two((self.max + 1) as u32) as i32;
            let new = globals().gmalloc().realloc(
                self.data as *mut c_void,
                self.max as usize * std::mem::size_of::<T>(),
                std::mem::align_of::<T>() as u32,
            ) as *const T;
            self.data = new;
        }
        unsafe {
            std::ptr::write(self.data.add(self.num as usize).cast_mut(), new_value);
        }
        self.num += 1;
    }
    pub fn extend(&mut self, other: &[T])
    where
        T: Copy,
    {
        for o in other {
            self.push(*o);
        }
    }
}

impl<T> From<&[T]> for TArray<T>
where
    T: Copy,
{
    fn from(value: &[T]) -> Self {
        let mut new = Self::with_capacity(value.len());
        // TODO this is probably unsound
        new.num = value.len() as i32;
        new.as_mut_slice().copy_from_slice(value);
        new
    }
}

impl Display for FString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let slice = self.as_slice();
        let last = slice.len()
            - slice
                .iter()
                .cloned()
                .rev()
                .position(|c| c != 0)
                .unwrap_or_default();
        write!(
            f,
            "{}",
            widestring::U16Str::from_slice(&slice[..last])
                .to_string()
                .unwrap()
        )
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct FVector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct FLinearColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub mod kismet {
    use super::*;

    #[derive(Debug)]
    #[repr(C)]
    pub struct FFrame {
        pub base: FOutputDevice,
        pub node: *const c_void,
        pub object: *mut UObject,
        pub code: *const c_void,
        pub locals: *const c_void,
        pub most_recent_property: *const FProperty,
        pub most_recent_property_address: *const c_void,
        pub flow_stack: [u8; 0x30],
        pub previous_frame: *const FFrame,
        pub out_parms: *const c_void,
        pub property_chain_for_compiled_in: *const FField,
        pub current_native_function: *const c_void,
        pub b_array_context_failed: bool,
    }

    pub fn arg<T: Sized>(stack: &mut FFrame, output: &mut T) {
        let output = output as *const _ as *mut _;
        unsafe {
            if stack.code.is_null() {
                let cur = stack.property_chain_for_compiled_in;
                stack.property_chain_for_compiled_in = (&*cur).next;
                (globals().fframe_step_explicit_property())(stack, output, cur as *const FProperty);
            } else {
                (globals().fframe_step())(stack, stack.object, output);
            }
        }
    }
}
