#![allow(non_snake_case, non_camel_case_types)]

use std::{ffi::c_void, fmt::Display, sync::Mutex};

use crate::{globals, guobject_array_unchecked};

pub static GMALLOC: GMalloc = GMalloc {
    ptr: Mutex::new(None),
};

pub static FFRAME_STEP_EXPLICIT_PROPERTY: Mutex<Option<FnFFrame_StepExplicitProperty>> =
    Mutex::new(None);
pub static FFRAME_STEP: Mutex<Option<FnFFrame_Step>> = Mutex::new(None);

pub type FnFFrame_Step =
    unsafe extern "system" fn(stack: &mut kismet::FFrame, *mut UObject, result: *mut c_void);
pub type FnFFrame_StepExplicitProperty = unsafe extern "system" fn(
    stack: &mut kismet::FFrame,
    result: *mut c_void,
    property: *const FProperty,
);

pub fn FName_ToString(name: &FName) -> String {
    unsafe {
        type FnFNameToString = unsafe extern "system" fn(&FName, &mut FString);

        let fnametostring: FnFNameToString =
            std::mem::transmute(globals().resolution.fnametostring.0);
        let mut string = FString::new();
        fnametostring(name, &mut string);
        string.to_string()
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct GMalloc {
    ptr: Mutex<Option<*const *const FMalloc>>,
}
unsafe impl Sync for GMalloc {}
unsafe impl Send for GMalloc {}
impl GMalloc {
    pub fn set(&self, gmalloc: *const c_void) {
        *self.ptr.lock().unwrap() = Some(gmalloc as *const *const FMalloc);
    }
    pub fn get(&self) -> &FMalloc {
        unsafe { &**self.ptr.lock().unwrap().unwrap() }
    }
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
        unsafe { ((*self.vtable).Malloc)(self, count, alignment) }
    }
    pub fn realloc(&self, original: *mut c_void, count: usize, alignment: u32) -> *mut c_void {
        unsafe { ((*self.vtable).Realloc)(self, original, count, alignment) }
    }
    pub fn free(&self, original: *mut c_void) {
        unsafe { ((*self.vtable).Free)(self, original) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FMallocVTable {
    pub __vecDelDtor: unsafe extern "system" fn(), // TODO
    pub Exec: unsafe extern "system" fn(),         // TODO
    pub Malloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub TryMalloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub Realloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub TryRealloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub Free: unsafe extern "system" fn(this: &FMalloc, original: *mut c_void),
    pub QuantizeSize: unsafe extern "system" fn(), // TODO
    pub GetAllocationSize: unsafe extern "system" fn(), // TODO
    pub Trim: unsafe extern "system" fn(),         // TODO
    pub SetupTLSCachesOnCurrentThread: unsafe extern "system" fn(), // TODO
    pub ClearAndDisableTLSCachesOnCurrentThread: unsafe extern "system" fn(), // TODO
    pub InitializeStatsMetadata: unsafe extern "system" fn(), // TODO
    pub UpdateStats: unsafe extern "system" fn(),  // TODO
    pub GetAllocatorStats: unsafe extern "system" fn(), // TODO
    pub DumpAllocatorStats: unsafe extern "system" fn(), // TODO
    pub IsInternallyThreadSafe: unsafe extern "system" fn(), // TODO
    pub ValidateHeap: unsafe extern "system" fn(), // TODO
    pub GetDescriptiveName: unsafe extern "system" fn(), // TODO
}

#[derive(Debug)]
#[repr(C)]
pub struct FWindowsCriticalSection([u8; 0x28]);

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
    pub ObjFirstGCIndex: i32,
    pub ObjLastNonGCIndex: i32,
    pub MaxObjectsNotConsideredByGC: i32,
    pub OpenForDisregardForGC: bool,

    pub ObjObjects: FChunkedFixedUObjectArray,
    pub ObjObjectsCritical: FWindowsCriticalSection,
    pub ObjAvailableList: [u8; 0x88],
    pub UObjectCreateListeners: TArray<*const FUObjectCreateListener>,
    pub UObjectDeleteListeners: TArray<*const FUObjectDeleteListener>,
    pub UObjectDeleteListenersCritical: FWindowsCriticalSection,
    pub MasterSerialNumber: std::sync::atomic::AtomicI32,
}
impl FUObjectArray {
    pub fn iter(&self) -> ObjectIterator<'_> {
        ObjectIterator {
            array: self,
            index: 0,
        }
    }
    fn item_ptr(&self, index: ObjectIndex) -> *const FUObjectItem {
        let per_chunk = self.ObjObjects.MaxElements / self.ObjObjects.MaxChunks;

        unsafe {
            (*self.ObjObjects.Objects.add((index / per_chunk) as usize))
                .add((index % per_chunk) as usize)
        }
    }
    fn item(&self, index: ObjectIndex) -> &FUObjectItem {
        unsafe { &*self.item_ptr(index) }
    }
    fn item_mut(&mut self, index: ObjectIndex) -> &mut FUObjectItem {
        unsafe { &mut *(self.item_ptr(index) as *mut FUObjectItem) }
    }
    pub fn allocate_serial_number(&self, index: ObjectIndex) -> i32 {
        use std::sync::atomic::Ordering;

        let item = self.item(index);

        let current = item.SerialNumber.load(Ordering::SeqCst);
        if current != 0 {
            current
        } else {
            let new = self.MasterSerialNumber.fetch_add(1, Ordering::SeqCst);

            let exchange =
                item.SerialNumber
                    .compare_exchange(0, new, Ordering::SeqCst, Ordering::SeqCst);
            match exchange {
                Ok(_) => new,
                Err(old) => old,
            }
        }
    }
}

pub struct ObjectIterator<'a> {
    array: &'a FUObjectArray,
    index: i32,
}
impl<'a> Iterator for ObjectIterator<'a> {
    type Item = Option<&'a UObjectBase>;
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.array.ObjObjects.NumElements as usize;
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
        if self.index >= self.array.ObjObjects.NumElements {
            None
        } else {
            let obj = unsafe { self.array.item(self.index).Object.as_ref() };

            self.index += 1;
            Some(obj)
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FChunkedFixedUObjectArray {
    pub Objects: *const *const FUObjectItem,
    pub PreAllocatedObjects: *const FUObjectItem,
    pub MaxElements: i32,
    pub NumElements: i32,
    pub MaxChunks: i32,
    pub NumChunks: i32,
}

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectItem {
    pub Object: *const UObjectBase,
    pub Flags: i32,
    pub ClusterRootIndex: i32,
    pub SerialNumber: std::sync::atomic::AtomicI32,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FWeakObjectPtr {
    ObjectIndex: i32,
    ObjectSerialNumber: i32,
}
impl FWeakObjectPtr {
    pub fn new(object: &UObjectBase) -> Self {
        Self::new_from_index(object.InternalIndex)
    }
    pub fn new_from_index(index: ObjectIndex) -> Self {
        Self {
            ObjectIndex: index,
            // serial allocation performs only atomic operations
            ObjectSerialNumber: unsafe { guobject_array_unchecked().allocate_serial_number(index) },
        }
    }
    pub fn get(&self, object_array: &FUObjectArray) -> Option<&UObjectBase> {
        // TODO check valid
        let item = object_array.item(self.ObjectIndex);
        unsafe { Some(&*item.Object) }
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

#[derive(Debug)]
#[repr(C)]
pub struct UObjectBase {
    pub vftable: *const std::ffi::c_void,
    /* offset 0x0008 */ pub ObjectFlags: EObjectFlags,
    /* offset 0x000c */ pub InternalIndex: i32,
    /* offset 0x0010 */ pub ClassPrivate: *const UClass,
    /* offset 0x0018 */ pub NamePrivate: FName,
    /* offset 0x0020 */ pub OuterPrivate: *const UObject,
}

#[derive(Debug)]
#[repr(C)]
pub struct UObjectBaseUtility {
    pub UObjectBase: UObjectBase,
}

#[derive(Debug)]
#[repr(C)]
pub struct UObject {
    pub UObjectBaseUtility: UObjectBaseUtility,
}

#[derive(Debug)]
#[repr(C)]
struct FOutputDevice {
    vtable: *const c_void,
    /* offset 0x0008 */ bSuppressEventTag: bool,
    /* offset 0x0009 */ bAutoEmitLineTerminator: bool,
}

#[derive(Debug)]
#[repr(C)]
pub struct UField {
    pub UObject: UObject,
    pub Next: *const UField,
}

#[derive(Debug)]
#[repr(C)]
pub struct FStructBaseChain {
    /* offset 0x0000 */ pub StructBaseChainArray: *const *const FStructBaseChain,
    /* offset 0x0008 */ pub NumStructBasesInChainMinusOne: i32,
}

#[derive(Debug)]
#[repr(C)]
struct FFieldClass {
    // TODO
    /* offset 0x0000 */
    Name: FName,
    /* offset 0x0008 */ //unhandled_primitive.kind /* UQuad */ Id;
    /* offset 0x0010 */ //unhandled_primitive.kind /* UQuad */ CastFlags;
    /* offset 0x0018 */ //EClassFlags ClassFlags;
    /* offset 0x0020 */ //FFieldClass* SuperClass;
    /* offset 0x0028 */ //FField* DefaultObject;
    /* offset 0x0030 */ //Type0x1159e /* TODO: figure out how to name it */* ConstructFn;
    /* offset 0x0038 */ //FThreadSafeCounter UnqiueNameIndexCounter;
}

#[derive(Debug)]
#[repr(C)]
struct FFieldVariant {
    /* offset 0x0000 */ container: *const c_void,
    /* offset 0x0008 */ bIsUObject: bool,
}

#[derive(Debug)]
#[repr(C)]
pub struct FField {
    /* offset 0x0008 */ ClassPrivate: *const FFieldClass,
    /* offset 0x0010 */ Owner: FFieldVariant,
    /* offset 0x0020 */ Next: *const FField,
    /* offset 0x0028 */ NamePrivate: FName,
    /* offset 0x0030 */ FlagsPrivate: EObjectFlags,
}

pub struct FProperty {
    // TODO
    /* offset 0x0000 */ //pub FField: FField,
    /* offset 0x0038 */ //pub ArrayDim: i32,
    /* offset 0x003c */ //pub ElementSize: i32,
    /* offset 0x0040 */ //EPropertyFlags PropertyFlags;
    /* offset 0x0048 */ //unhandled_primitive.kind /* UShort */ RepIndex;
    /* offset 0x004a */ //TEnumAsByte<enum ELifetimeCondition> BlueprintReplicationCondition;
    /* offset 0x004c */ //int32_t Offset_Internal;
    /* offset 0x0050 */ //FName RepNotifyFunc;
    /* offset 0x0058 */ //FProperty* PropertyLinkNext;
    /* offset 0x0060 */ //FProperty* NextRef;
    /* offset 0x0068 */ //FProperty* DestructorLinkNext;
    /* offset 0x0070 */ //FProperty* PostConstructLinkNext;
}

#[derive(Debug)]
#[repr(C)]
pub struct UStruct {
    /* offset 0x0000 */ pub UField: UField,
    /* offset 0x0030 */ pub FStructBaseChain: FStructBaseChain,
    /* offset 0x0040 */ pub SuperStruct: *const UStruct,
    /* offset 0x0048 */ pub Children: *const UField,
    /* offset 0x0050 */ pub ChildProperties: *const FField,
    /* offset 0x0058 */ pub PropertiesSize: i32,
    /* offset 0x005c */ pub MinAlignment: i32,
    /* offset 0x0060 */ pub Script: TArray<u8>,
    /* offset 0x0070 */ pub PropertyLink: *const FProperty,
    /* offset 0x0078 */ pub RefLink: *const FProperty,
    /* offset 0x0080 */ pub DestructorLink: *const FProperty,
    /* offset 0x0088 */ pub PostConstructLink: *const FProperty,
    /* offset 0x0090 */
    pub ScriptAndPropertyObjectReferences: TArray<*const UObject>,
    /* offset 0x00a0 */
    pub UnresolvedScriptProperties: *const (), //TODO pub TArray<TTuple<TFieldPath<FField>,int>,TSizedDefaultAllocator<32> >*
    /* offset 0x00a8 */
    pub UnversionedSchema: *const (), //TODO const FUnversionedStructSchema*
}

#[derive(Debug)]
#[repr(C)]
pub struct UClass {
    /* offset 0x0000 */ pub UStruct: UStruct,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FName {
    /* offset 0x0000 */ pub ComparisonIndex: FNameEntryId,
    /* offset 0x0004 */ pub Number: u32,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FNameEntryId {
    /* offset 0x0000 */ pub Value: u32,
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

#[derive(Debug)]
#[repr(C)]
pub struct TArray<T> {
    pub data: *const T,
    pub num: i32,
    pub max: i32,
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
        GMALLOC.get().free(self.data as *mut c_void);
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
    pub fn as_slice(&self) -> &[T] {
        if self.num == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.data, self.num as usize) }
        }
    }
    pub fn as_slice_mut(&mut self) -> &mut [T] {
        if self.num == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.data as *mut _, self.num as usize) }
        }
    }
    pub fn from_slice(slice: &[T]) -> TArray<T> {
        TArray {
            data: slice.as_ptr(),
            num: slice.len() as i32,
            max: slice.len() as i32,
        }
    }
}

impl Display for FString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            widestring::U16Str::from_slice(self.as_slice())
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
        /* offset 0x0000 */ pub base: FOutputDevice,
        /* offset 0x0010 */ pub node: *const c_void,
        /* offset 0x0018 */ pub object: *mut UObject,
        /* offset 0x0020 */ pub code: *const c_void,
        /* offset 0x0028 */ pub locals: *const c_void,
        /* offset 0x0030 */ pub most_recent_property: *const FProperty,
        /* offset 0x0038 */ pub most_recent_property_address: *const c_void,
        /* offset 0x0040 */ pub flow_stack: [u8; 0x30],
        /* offset 0x0070 */ pub previous_frame: *const FFrame,
        /* offset 0x0078 */ pub out_parms: *const c_void,
        /* offset 0x0080 */ pub property_chain_for_compiled_in: *const FField,
        /* offset 0x0088 */ pub current_native_function: *const c_void,
        /* offset 0x0090 */ pub b_array_context_failed: bool,
    }

    pub fn arg<T: Sized>(stack: &mut FFrame, output: &mut T) {
        //dbg!(&stack);
        let output = output as *const _ as *mut _;
        unsafe {
            //simple_log::info!("{:x?}", stack);
            if stack.code.is_null() {
                let cur = stack.property_chain_for_compiled_in;
                stack.property_chain_for_compiled_in = (*cur).Next;
                FFRAME_STEP_EXPLICIT_PROPERTY.lock().unwrap().unwrap()(
                    stack,
                    output,
                    cur as *const FProperty,
                );
            } else {
                FFRAME_STEP.lock().unwrap().unwrap()(stack, stack.object, output);
            }
        }
    }
}
