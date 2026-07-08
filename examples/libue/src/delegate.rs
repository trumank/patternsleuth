use super::*;

/// `FScriptDelegate` — a single-cast dynamic delegate binding: a weak pointer
/// to the bound object plus the name of the UFunction to call on it. POD (no
/// heap ownership inside), so it's `Copy` and TArrays of it need no element
/// destructors.
///
/// Layout mirrors `TScriptDelegate<FWeakObjectPtr>` (ScriptDelegates.h): an
/// `FWeakObjectPtr` (two i32s) followed by an `FName` — 16 bytes.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FScriptDelegate {
    pub object: FWeakObjectPtr,
    pub function_name: FName,
}

const _: () = assert!(std::mem::size_of::<FScriptDelegate>() == 16);

impl Default for FScriptDelegate {
    fn default() -> Self {
        Self::unbound()
    }
}

/// Two bindings are equal when they target the same object and function name —
/// the identity UE uses for `AddUnique`/`Remove`.
impl PartialEq for FScriptDelegate {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object && self.function_name == other.function_name
    }
}
impl Eq for FScriptDelegate {}

impl FScriptDelegate {
    pub fn unbound() -> Self {
        Self {
            object: FWeakObjectPtr::default(),
            function_name: FName {
                comparison_index: FNameEntryId { value: 0 }, // NAME_None
                number: 0,
            },
        }
    }

    /// Bind to `(object, function_name)`. Mirrors `BindUFunction`: it only
    /// stores the pair — the function is resolved by name at broadcast time.
    pub fn new(object: &UObjectBase, function_name: FName) -> Self {
        Self {
            object: FWeakObjectPtr::new(object),
            function_name,
        }
    }

    /// NAME_None lives at comparison index 0 in the global name table.
    pub fn function_name_is_none(&self) -> bool {
        self.function_name.comparison_index.value == 0
    }

    /// Bound when the function name isn't NAME_None and the object still
    /// resolves (even if pending kill — matches the REPL's lenient view).
    pub fn is_bound(&self) -> bool {
        !self.function_name_is_none() && self.object.get_even_if_pending_kill().is_some()
    }

    /// Resolve the bound object, or `None` if stale/destroyed.
    pub fn object(&self) -> Option<&UObjectBase> {
        self.object.get_even_if_pending_kill()
    }
}

/// `FMulticastScriptDelegate` — a multicast dynamic delegate: an invocation
/// list of `FScriptDelegate`s. Layout is a single `TArray<FScriptDelegate>`,
/// matching `TMulticastScriptDelegate<FWeakObjectPtr>`.
///
/// This is the *inline* storage used by `FMulticastInlineDelegateProperty`, and
/// also what `FMulticastSparseDelegateProperty::GetMulticastDelegate`
/// materializes out of the global side table. We only ever *read* it directly —
/// mutation goes through the property's `AddDelegate`/`RemoveDelegate`/
/// `ClearDelegate` vtable methods so inline and sparse storage are handled
/// uniformly and with the engine's own semantics.
#[repr(C)]
#[derive(Debug, Default)]
pub struct FMulticastScriptDelegate {
    pub invocation_list: TArray<FScriptDelegate>,
}

impl FMulticastScriptDelegate {
    pub fn num(&self) -> i32 {
        self.invocation_list.num
    }

    pub fn is_bound(&self) -> bool {
        self.invocation_list.num > 0
    }

    pub fn as_slice(&self) -> &[FScriptDelegate] {
        self.invocation_list.as_slice()
    }

    /// `AddUnique` — append `d` unless an equal binding is already present.
    /// Mirrors `FMulticastScriptDelegate::AddUnique` for inline storage.
    pub fn add_unique(&mut self, d: FScriptDelegate) {
        if !self.invocation_list.as_slice().contains(&d) {
            self.invocation_list.push(d);
        }
    }

    /// Remove every binding equal to `d`, compacting the list in place.
    /// `FScriptDelegate` is POD, so no per-element destruction is needed.
    pub fn remove_matching(&mut self, d: &FScriptDelegate) {
        let n = self.invocation_list.num as usize;
        let s = self.invocation_list.as_mut_slice();
        let mut w = 0usize;
        for r in 0..n {
            if s[r] != *d {
                s[w] = s[r];
                w += 1;
            }
        }
        self.invocation_list.num = w as i32;
    }

    /// Empty the invocation list (keeps the backing allocation; the elements
    /// are POD so there's nothing to destroy).
    pub fn clear(&mut self) {
        self.invocation_list.num = 0;
    }
}
