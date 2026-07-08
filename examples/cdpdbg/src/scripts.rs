use jmap_kismet_test::kismet_text::tokens::PLAIN;
use libue as ue;

use std::collections::HashMap;

use crate::live_resolver::{LiveNameMap, LiveResolver};
use jmap_kismet_test::bytecode::emitter;
use jmap_kismet_test::bytecode::parser::ScriptParser;
use jmap_kismet_test::bytecode::reader::ScriptReader;
use jmap_kismet_test::bytecode::types::Address;
use jmap_kismet_test::kismet_text::gc_roots::collect_gc_roots;
use jmap_kismet_test::kismet_text::lexer::Lexer;
use jmap_kismet_test::kismet_text::object::{FunctionBlock, ObjectBlock};
use jmap_kismet_test::kismet_text::parser::parse_object;
use jmap_kismet_test::kismet_text::serializer::{SourceMapping, serialize_object_mapped};

// ── Script data ─────────────────────────────────────────────────────────────

/// A disassembled Blueprint class: one source file covering every UFunction
/// in the class, emitted by the class-level serializer.
pub struct Script {
    pub script_id: String,
    pub class_path: String,
    pub url: String,
    pub source: String,
    pub line_count: usize,
    pub functions: Vec<ScriptFunction>,
}
unsafe impl Send for Script {}
unsafe impl Sync for Script {}

/// One function block within a `Script`.
pub struct ScriptFunction {
    pub short_name: String,
    pub full_path: String,
    pub weak: ue::TWeakObjectPtr<ue::UFunction>,
    /// Global (0-indexed) line range of this function's block within
    /// `Script::source`, from `function …` header to closing `}`.
    pub start_line: usize,
    pub end_line: usize,
    /// Bytecode-offset ↔ source mappings, with line numbers already rebased
    /// to the class-level source coordinates.
    pub source_map: Vec<SourceMapping>,
}

impl Script {
    /// Index of the function whose block contains `line`, or None if the
    /// line falls outside every function (e.g. the class header).
    pub fn function_for_line(&self, line: usize) -> Option<usize> {
        self.functions
            .iter()
            .position(|f| line >= f.start_line && line <= f.end_line)
    }

    pub fn function_for_weak(&self, weak: &ue::TWeakObjectPtr<ue::UFunction>) -> Option<usize> {
        self.functions.iter().position(|f| f.weak == *weak)
    }

    /// Find source line for a bytecode offset within a specific function
    /// (largest source-map offset ≤ requested).
    pub fn offset_to_line(&self, func_idx: usize, bytecode_offset: u64) -> Option<usize> {
        let f = self.functions.get(func_idx)?;
        f.source_map
            .iter()
            .rev()
            .find(|m| m.bytecode_offset <= bytecode_offset as usize)
            .map(|m| m.line)
    }

    /// Map a (line, col) source location to a (function index, bytecode offset),
    /// preferring exact column matches and falling back to line start.
    pub fn location_to_offset(&self, line: usize, col: usize) -> Option<(usize, u64)> {
        let func_idx = self.function_for_line(line)?;
        let f = &self.functions[func_idx];
        if let Some(m) = f.source_map.iter().find(|m| m.line == line && m.col == col) {
            return Some((func_idx, m.bytecode_offset as u64));
        }
        let m = f.source_map.iter().find(|m| m.line == line)?;
        Some((func_idx, m.bytecode_offset as u64))
    }

    /// Live script bytes for a given function, or empty if the UFunction has
    /// been collected.
    pub fn function_bytes(&self, func_idx: usize) -> &[u8] {
        self.functions
            .get(func_idx)
            .and_then(|f| f.weak.get())
            .map(|uf| uf.script().as_slice())
            .unwrap_or(&[])
    }
}

// ── Build ───────────────────────────────────────────────────────────────────

/// Build one `Script` per UClass by iterating the object array for UClasses
/// and walking each class's own `children` list for its direct UFunctions.
pub fn build_scripts(guobject_array: &ue::FUObjectArray) -> Vec<Script> {
    let mut out = Vec::new();
    for maybe_obj in guobject_array.objects().iter() {
        let Some(obj) = maybe_obj else { continue };
        if obj.cast::<ue::UClass>().is_none() {
            continue;
        }
        let funcs = collect_class_functions(obj);
        if funcs.is_empty() {
            continue;
        }
        if let Some(script) = build_class_script(obj, &funcs) {
            out.push(script);
        }
    }
    tracing::info!("Built {} class scripts", out.len());
    out
}

/// Build a class Script from a post-loaded UClass by walking its `children`
/// list for direct UFunctions. Returns `None` if `class_obj` is not a UClass
/// or has no functions with bytecode.
pub fn try_build_class_script(class_obj: &ue::UObjectBase) -> Option<Script> {
    class_obj.cast::<ue::UClass>()?;
    let funcs = collect_class_functions(class_obj);
    if funcs.is_empty() {
        return None;
    }
    build_class_script(class_obj, &funcs)
}

fn collect_class_functions(class: &ue::UObjectBase) -> Vec<&ue::UFunction> {
    let ustruct = unsafe { &*(class as *const _ as *const ue::UStruct) };
    ustruct
        .iter_children()
        .filter_map(|field| field.cast::<ue::UFunction>())
        .filter(|f| !f.script().as_slice().is_empty())
        .collect()
}

fn build_class_script(class_obj: &ue::UObjectBase, funcs: &[&ue::UFunction]) -> Option<Script> {
    if funcs.is_empty() {
        return None;
    }
    let class_path = class_obj.object_path();

    // Build (FunctionBlock, weak) pairs for every UFunction in the class.
    let mut entries: Vec<(FunctionBlock, ue::TWeakObjectPtr<ue::UFunction>)> = Vec::new();
    for f in funcs {
        let base: &ue::UObjectBase = f;
        let full_path = base.object_path();
        let short_name = full_path
            .rsplit_once(':')
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| full_path.clone());
        let is_ubergraph = f
            .function_flags()
            .contains(ue::EFunctionFlags::FUNC_UbergraphFunction);
        let params = collect_param_names(f);

        let names = LiveNameMap;
        let reader = ScriptReader::new(f.script().as_slice(), &names);
        let mut bc_parser = ScriptParser::new(reader);
        let body = bc_parser.parse_all();
        let weak = ue::TWeakObjectPtr::<ue::UFunction>::new(base);

        entries.push((
            FunctionBlock {
                short_name,
                full_path,
                params,
                body,
                is_ubergraph,
            },
            weak,
        ));
    }

    // Ubergraph functions go last, preserving discovery order within each group.
    // The library's `serialize_object_mapped` emits in the same order, so the
    // `weaks` vec below stays aligned with the returned `FunctionLayout`s.
    entries.sort_by_key(|(b, _)| b.is_ubergraph as u8);

    let (blocks, weaks): (Vec<FunctionBlock>, Vec<ue::TWeakObjectPtr<ue::UFunction>>) =
        entries.into_iter().unzip();
    let object = ObjectBlock {
        class_path: class_path.clone(),
        functions: blocks,
    };

    // LiveResolver only needs a representative UFunction to derive the class
    // context — any function in the class will do.
    let first_obj: &ue::UObjectBase = funcs[0];
    let resolver = LiveResolver::new(first_obj, funcs[0]);
    let output = serialize_object_mapped(&object, &resolver, &PLAIN);
    let line_count = output.source.lines().count();

    let script_functions: Vec<ScriptFunction> = output
        .functions
        .into_iter()
        .zip(weaks)
        .map(|(layout, weak)| ScriptFunction {
            short_name: layout.short_name,
            full_path: layout.full_path,
            weak,
            start_line: layout.start_line,
            end_line: layout.end_line,
            source_map: layout.source_map,
        })
        .collect();

    let script_id = format!("class:{class_path}");

    Some(Script {
        script_id,
        url: format!("kismet://{class_path}"),
        class_path,
        source: output.source,
        line_count,
        functions: script_functions,
    })
}

fn collect_param_names(func: &ue::UFunction) -> Vec<String> {
    func.params().map(|p| p.name().to_string()).collect()
}

/// Error from [`set_script_source`]. Carries an optional byte-offset span
/// into the submitted source so the CDP layer can surface a line/column to
/// the DevTools client.
#[derive(Debug, Clone)]
pub struct SetScriptError {
    pub message: String,
    pub span: Option<(usize, usize)>,
}

impl SetScriptError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }
}

impl From<jmap_kismet_test::kismet_text::ParseError> for SetScriptError {
    fn from(err: jmap_kismet_test::kismet_text::ParseError) -> Self {
        Self {
            message: err.message,
            span: Some((err.span.start, err.span.end)),
        }
    }
}

/// Parse edited class-level source, emit bytecode for each function, and
/// swap every matching UFunction's script TArray in place. After patching
/// the class Script is rebuilt so `source`/`source_map` reflect the new
/// bytecode. Any existing breakpoints on this script are invalidated and
/// should be cleared by the caller.
pub fn set_script_source(script: &mut Script, source: &str) -> Result<(), SetScriptError> {
    // Snapshot every live UFunction by short name via raw pointers so we
    // don't carry borrows from `script` across the final reassignment.
    let live_funcs: Vec<(String, *const ue::UFunction)> = script
        .functions
        .iter()
        .filter_map(|f| {
            f.weak
                .get()
                .map(|uf| (f.short_name.clone(), std::ptr::from_ref(uf)))
        })
        .collect();
    if live_funcs.is_empty() {
        return Err(SetScriptError::plain(
            "no live UFunction in class (all GC'd)",
        ));
    }

    // Build a class-anchored resolver from any live function in the class.
    // LiveResolver copies what it needs out of the &refs, so the short-lived
    // borrow here doesn't need to outlive parse_object.
    let first_raw = live_funcs[0].1;
    let resolver = {
        let first_func: &ue::UFunction = unsafe { &*first_raw };
        let first_obj: &ue::UObjectBase = first_func;
        LiveResolver::new(first_obj, first_func)
    };

    // Lex + parse into a class-level ObjectBlock. `parse_object` handles
    // cross-function ubergraph label fixup, so callers into the ubergraph
    // end up with real offsets in their `IntConst` sentinels.
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let object = parse_object(tokens, &resolver)?;

    if object.class_path != script.class_path {
        return Err(SetScriptError::plain(format!(
            "class mismatch: source declares '{}', script is '{}'",
            object.class_path, script.class_path
        )));
    }

    // short_name → target UFunction raw pointer, from the live snapshot.
    let mut by_name: HashMap<&str, *const ue::UFunction> = HashMap::new();
    for (name, raw) in &live_funcs {
        by_name.insert(name.as_str(), *raw);
    }

    // Emit bytecode for every parsed function against its live target, and
    // collect the set of UObject addresses the new bytecode will reference.
    // These need to go into the UStruct's ScriptAndPropertyObjectReferences
    // so the GC keeps them alive — the on-disk loader normally populates
    // that list, but runtime-emitted bytecode bypasses it.
    //
    // We collect all patches first, so a later emit failure can't leave the
    // class half-updated.
    let names = LiveNameMap;
    let mut patches: Vec<(*mut ue::UFunction, Vec<u8>, Vec<Address>)> =
        Vec::with_capacity(object.functions.len());
    for block in &object.functions {
        let target = by_name
            .get(block.short_name.as_str())
            .copied()
            .ok_or_else(|| {
                SetScriptError::plain(format!(
                    "unknown function '{}' — not present on class '{}'",
                    block.short_name, script.class_path
                ))
            })?;
        let bytecode = emitter::emit_all(&block.body, &names);
        let gc_roots: Vec<Address> = collect_gc_roots(&block.body, &resolver)
            .into_iter()
            .collect();
        patches.push((target.cast_mut(), bytecode, gc_roots));
    }

    // Swap every UFunction's script TArray. Leak the old allocation so any
    // thread still executing the old bytecode won't fault. Mark each patched
    // function as a GC root — the edited bytecode should survive reference
    // pressure even if no live blueprint path keeps the function alive —
    // and extend ScriptAndPropertyObjectReferences with the new bytecode's
    // UObject refs.
    for (func_ptr, bytecode, gc_roots) in &patches {
        unsafe {
            let script_field = (**func_ptr).script_mut() as *mut ue::TArray<u8>;
            let old = std::ptr::read(script_field);
            std::mem::forget(old);
            std::ptr::write(script_field, ue::TArray::from(bytecode.as_slice()));

            let refs = (**func_ptr).script_and_property_object_references_mut();
            let existing: std::collections::HashSet<*const ue::UObject> =
                refs.as_slice().iter().copied().collect();
            for addr in gc_roots {
                let ptr = addr.0 as *const ue::UObject;
                if ptr.is_null() || existing.contains(&ptr) {
                    continue;
                }
                refs.push(ptr);
            }

            let obj_base: &ue::UObjectBase = &**func_ptr;
            obj_base.add_to_root();
        }
    }

    tracing::info!(
        "setScriptSource [{}]: updated {} functions",
        script.class_path,
        patches.len()
    );

    // Rebuild the class Script from live UE state so source + source_map
    // reflect the newly-emitted bytecode. Walk up to the owning UClass from
    // any live function in the class.
    let first_obj: &ue::UObjectBase = unsafe { &*first_raw };
    let class_obj = unsafe { first_obj.outer_private().as_ref() }
        .ok_or_else(|| SetScriptError::plain("function has no outer class"))?;
    // Root the owning class too, so the edited bytecode's referents stay live.
    class_obj.add_to_root();
    let rebuilt = try_build_class_script(class_obj)
        .ok_or_else(|| SetScriptError::plain("failed to rebuild class script after patching"))?;
    *script = rebuilt;

    Ok(())
}
