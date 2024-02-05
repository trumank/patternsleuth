#[cfg(feature = "patterns")]
pub mod patterns;
pub mod process;
pub mod resolvers;
#[cfg(feature = "symbols")]
pub mod symbols;

pub mod scanner {
    pub use patternsleuth_scanner::*;
}

use std::{
    borrow::Cow, cmp::Ordering, collections::HashMap, ops::{Index, Range, RangeFrom, RangeTo}, path::Path
};

use itertools::Itertools;
use scanner::{Pattern, Xref};

use anyhow::{bail, Context, Result};
use object::{File, Object, ObjectSection};

pub struct ResolveContext<'data, 'pattern> {
    pub exe: &'data Image<'data>,
    pub memory: &'data Memory<'data>,
    pub section: String,
    pub match_address: usize,
    pub scan: &'pattern Scan,
}

#[derive(Debug)]
pub struct Resolution {
    /// intermediate addresses of interest before reaching the final address
    /// can be used for inspecting/debugging patterns (shown with the --disassemble flag)
    pub stages: Vec<usize>,
    /// final, fully resolved address
    pub res: ResolutionType,
}

#[derive(Debug)]
pub enum ResolutionAction {
    /// Performing another scan
    Continue(Scan),
    /// Finish scan
    Finish(ResolutionType),
}
impl From<ResolutionType> for ResolutionAction {
    fn from(val: ResolutionType) -> Self {
        ResolutionAction::Finish(val)
    }
}
impl From<Option<usize>> for ResolutionAction {
    fn from(opt_address: Option<usize>) -> Self {
        match opt_address {
            Some(addr) => ResolutionType::Address(addr),
            None => ResolutionType::Failed,
        }
        .into()
    }
}
impl From<usize> for ResolutionAction {
    fn from(address: usize) -> Self {
        ResolutionType::Address(address).into()
    }
}
impl From<Option<String>> for ResolutionAction {
    fn from(opt_string: Option<String>) -> Self {
        match opt_string {
            Some(string) => ResolutionType::String(string),
            None => ResolutionType::Failed,
        }
        .into()
    }
}
impl From<String> for ResolutionAction {
    fn from(string: String) -> Self {
        ResolutionType::String(string).into()
    }
}

#[derive(Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum ResolutionType {
    /// address of resolved match
    Address(usize),
    /// string resolution (e.g. Unreal Engine version)
    String(String),
    /// report no data and just count successful matches
    Count,
    /// error during resolution or fails some criteria
    Failed,
}

impl From<Option<usize>> for ResolutionType {
    fn from(opt_address: Option<usize>) -> Self {
        match opt_address {
            Some(addr) => ResolutionType::Address(addr),
            None => ResolutionType::Failed,
        }
    }
}
impl From<usize> for ResolutionType {
    fn from(address: usize) -> Self {
        ResolutionType::Address(address)
    }
}
impl From<Option<String>> for ResolutionType {
    fn from(opt_string: Option<String>) -> Self {
        match opt_string {
            Some(string) => ResolutionType::String(string),
            None => ResolutionType::Failed,
        }
    }
}
impl From<String> for ResolutionType {
    fn from(string: String) -> Self {
        ResolutionType::String(string)
    }
}

#[derive(Debug, Clone)]
pub struct Scan {
    pub section: Option<object::SectionKind>,
    pub scan_type: ScanType,
    pub resolve: Resolve,
}
/*
impl Scan {
    pub fn resolve(&self, ctx: ResolveContext) -> Resolution {
        let mut stages = ResolveStages(vec![]);
        Resolution {
            res: (self.resolve)(ctx, &mut stages),
            stages: stages.0,
        }
    }
}
*/
#[derive(Debug, Clone)]
pub enum ScanType {
    Pattern(Pattern),
    Xref(Xref),
}
impl ScanType {
    pub fn get_pattern(&self) -> Option<&Pattern> {
        match self {
            Self::Pattern(pattern) => Some(pattern),
            _ => None,
        }
    }
    pub fn get_xref(&self) -> Option<&Xref> {
        match self {
            Self::Xref(xref) => Some(xref),
            _ => None,
        }
    }
}
impl From<Pattern> for ScanType {
    fn from(value: Pattern) -> Self {
        Self::Pattern(value)
    }
}
impl From<Xref> for ScanType {
    fn from(value: Xref) -> Self {
        Self::Xref(value)
    }
}

#[derive(Debug, Clone)]
pub struct ResolveStages(pub Vec<usize>);

type Resolve = fn(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction;

#[derive(Debug)]
pub struct PatternConfig<S> {
    pub sig: S,
    pub name: String,
    pub scan: Scan,
}
impl<S> PatternConfig<S> {
    pub fn new(
        sig: S,
        name: String,
        section: Option<object::SectionKind>,
        pattern: Pattern,
        resolve: Resolve,
    ) -> Self {
        Self {
            sig,
            name,
            scan: Scan {
                section,
                scan_type: pattern.into(),
                resolve,
            },
        }
    }
    pub fn xref(
        sig: S,
        name: String,
        section: Option<object::SectionKind>,
        xref: Xref,
        resolve: Resolve,
    ) -> Self {
        Self {
            sig,
            name,
            scan: Scan {
                section,
                scan_type: xref.into(),
                resolve,
            },
        }
    }
}

#[derive(Debug)]
pub struct ScanResult<'a, S> {
    pub results: Vec<(&'a PatternConfig<S>, Resolution)>,
}
impl<'a, S: std::fmt::Debug + PartialEq> ScanResult<'a, S> {
    pub fn get_unique_sig_address(&self, sig: S) -> Result<usize> {
        let mut address = None;
        for (config, res) in &self.results {
            if config.sig == sig {
                match res.res {
                    ResolutionType::Address(addr) => {
                        if let Some(existing) = address {
                            if existing != addr {
                                bail!("sig {sig:?} matched multiple addresses")
                            }
                        } else {
                            address = Some(addr)
                        }
                    }
                    _ => bail!("sig {sig:?} matched a non-address"),
                }
            }
        }
        address.with_context(|| format!("sig {sig:?} not found"))
    }
}

#[derive(Default)]
pub struct ImageBuilder {
    functions: bool,
}
pub struct ImageBuilderWithSymbols<P: AsRef<Path>> {
    symbols: Option<P>,
    functions: bool,
}
impl ImageBuilder {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols<P: AsRef<Path>>(self, exe_path: P) -> ImageBuilderWithSymbols<P> {
        ImageBuilderWithSymbols {
            symbols: Some(exe_path),
            functions: self.functions,
        }
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read::<&str>(data, None, self.functions)
    }
}
impl<P: AsRef<Path>> ImageBuilderWithSymbols<P> {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols(mut self, exe_path: P) -> Self {
        self.symbols = Some(exe_path);
        self
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read(data, self.symbols, self.functions)
    }
}

#[cfg(target_os="windows")]
pub struct Image<'data> {
    pub base_address: usize,
    pub exception_directory_range: Range<usize>,
    pub exception_children_cache: HashMap<usize, Vec<RuntimeFunction>>,
    pub memory: Memory<'data>,
    pub symbols: Option<HashMap<usize, String>>,
    pub imports: HashMap<String, HashMap<String, usize>>,
}
#[cfg(target_os="windows")]
impl<'data> Image<'data> {
    fn read<P: AsRef<Path>>(
        data: &'data [u8],
        exe_path: Option<P>,
        load_functions: bool,
    ) -> Result<Image<'data>> {
        let object = object::File::parse(data)?;
        let memory = Memory::new(&object)?;
        Self::read_inner(exe_path, load_functions, memory, object)
    }
    fn read_inner<'memory, P: AsRef<Path>>(
        exe_path: Option<P>,
        load_functions: bool,
        memory: Memory<'memory>,
        object: object::File,
    ) -> Result<Image<'memory>> {
        let base_address = object.relative_address_base() as usize;

        #[allow(unused_variables)]
        let symbols = if let Some(exe_path) = exe_path {
            #[cfg(not(feature = "symbols"))]
            unreachable!();
            #[cfg(feature = "symbols")]
            {
                let pdb_path = exe_path.as_ref().with_extension("pdb");
                pdb_path
                    .exists()
                    .then(|| symbols::dump_pdb_symbols(pdb_path, base_address))
                    .transpose()?
            }
        } else {
            None
        };

        let get_ex_dir = || -> Result<Range<usize>> {
            Ok(match object {
                object::File::Pe64(ref inner) => {
                    let exception_directory = inner
                        .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                        .context("no exception directory")?;

                    let (address, size) = exception_directory.address_range();
                    base_address + address as usize..base_address + (address + size) as usize
                }
                _ => bail!("not a PE file"),
            })
        };

        let get_imports = || -> Result<_> {
            Ok(match object {
                object::File::Pe64(ref inner) => {
                    use object::pe::ImageNtHeaders64;
                    use object::read::pe::ImageThunkData;
                    use object::LittleEndian as LE;

                    let mut imports: HashMap<String, HashMap<String, usize>> = Default::default();

                    let import_table = inner.import_table()?.unwrap();
                    let mut import_descs = import_table.descriptors()?;

                    while let Some(import_desc) = import_descs.next()? {
                        let mut cur = HashMap::new();

                        let Ok(lib_name) = import_table.name(import_desc.name.get(LE)) else {
                            continue;
                        };
                        let lib_name = std::str::from_utf8(lib_name)?.to_ascii_lowercase();
                        let mut thunks =
                            import_table.thunks(import_desc.original_first_thunk.get(LE))?;
                        let mut address = base_address + import_desc.first_thunk.get(LE) as usize;
                        while let Some(thunk) = thunks.next::<ImageNtHeaders64>()? {
                            if let Ok((_hint, name)) = import_table.hint_name(thunk.address()) {
                                cur.insert(std::str::from_utf8(name)?.to_owned(), address);
                                address += 8;
                            }
                        }
                        imports.insert(lib_name, cur);
                    }
                    imports
                }
                _ => bail!("not a PE file"),
            })
        };

        let mut new = Image {
            base_address,
            exception_directory_range: get_ex_dir().unwrap_or_default(),
            exception_children_cache: Default::default(),
            memory,
            symbols,
            imports: get_imports().unwrap_or_default(),
        };

        if load_functions {
            new.populate_exception_cache()?;
        }
        Ok(new)
    }
    fn populate_exception_cache(&mut self) -> Result<(), MemoryAccessError> {
        for i in self.exception_directory_range.clone().step_by(12) {
            let f = RuntimeFunction::read(&self.memory, self.base_address, i)?;
            self.exception_children_cache.insert(f.range.start, vec![]);

            let Ok(section) = self.memory.get_section_containing(f.unwind) else {
                // TODO disabled cause spammy
                //println!("invalid unwind info addr {:x}", f.unwind);
                continue;
            };

            let mut unwind = f.unwind;
            let has_chain_info = section.section.index(unwind)? >> 3 == 0x4;
            if has_chain_info {
                let unwind_code_count = section.section.index(unwind + 2)?;

                unwind += 4 + 2 * unwind_code_count as usize;
                if unwind % 4 != 0 {
                    // align
                    unwind += 2;
                }

                if section.address() + section.data().len() > unwind + 12 {
                    let chained = RuntimeFunction::read(section, self.base_address, unwind)?;

                    // TODO disabled because it spams the log too much
                    //let referenced = self.get_function(chained.range.start);

                    //assert_eq!(Some(&chained), referenced.as_ref());
                    //if Some(&chained) != referenced.as_ref() {
                    //println!("mismatch {:x?} {referenced:x?}", Some(&chained));
                    //}

                    self.exception_children_cache
                        .entry(chained.range.start)
                        .or_default()
                        .push(f);
                } else {
                    println!("invalid unwind addr {:x}", unwind);
                }
            }
        }

        //println!("{:#x?}", self.exception_children_cache);
        Ok(())
    }
    /// Get function containing `address` from the exception directory
    pub fn get_function(
        &self,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        let size = 12;
        let mut min = 0;
        let mut max = self.exception_directory_range.len() / size - 1;

        while min <= max {
            let i = (max + min) / 2;
            let addr = i * size + self.exception_directory_range.start;

            let addr_begin = self.base_address + self.memory.u32_le(addr)? as usize;
            if addr_begin <= address {
                let addr_end = self.base_address + self.memory.u32_le(addr + 4)? as usize;
                if addr_end > address {
                    let unwind = self.base_address + self.memory.u32_le(addr + 8)? as usize;

                    return Ok(Some(RuntimeFunction {
                        range: addr_begin..addr_end,
                        unwind,
                    }));
                } else {
                    min = i + 1;
                }
            } else {
                max = i - 1;
            }
        }
        Ok(None)
    }
    /// Get root function containing `address` from the exception directory. This can be used to
    /// find the start address of a function given an address in the body.
    pub fn get_root_function(
        &self,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        if let Some(f) = self.get_function(address)? {
            let mut f = RuntimeFunction {
                range: f.range,
                unwind: f.unwind,
            };

            loop {
                let mut unwind_addr = f.unwind;

                let section = self.memory.get_section_containing(unwind_addr)?;

                let has_chain_info = section.section.index(unwind_addr)? >> 3 == 0x4;
                if has_chain_info {
                    let unwind_code_count = section.section.index(unwind_addr + 2)?;

                    unwind_addr += 4 + 2 * unwind_code_count as usize;
                    if unwind_addr % 4 != 0 {
                        // align
                        unwind_addr += 2;
                    }

                    if section.address() + section.data().len() > unwind_addr + 12 {
                        f = RuntimeFunction::read(section, self.base_address, unwind_addr)?;
                    } else {
                        todo!("not adding chain info {unwind_addr}");
                    }
                } else {
                    return Ok(Some(f));
                }
            }
        } else {
            Ok(None)
        }
    }
    /// Recursively get all child functions of `address` (exact). This is pulled from
    /// `exception_children_cache` so will be empty if it has not been populated.
    pub fn get_child_functions(
        &self,
        address: usize,
    ) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        let mut queue = vec![address];
        let mut all_children = vec![self.get_function(address)?.unwrap()];
        while let Some(next) = queue.pop() {
            if let Some(children) = self.exception_children_cache.get(&next) {
                for child in children {
                    queue.push(child.range().start);
                    all_children.push(child.clone());
                }
            }
        }
        Ok(all_children)
    }
}


#[cfg(target_os="linux")]
pub struct Image<'data> {
    pub base_address: usize,
    pub memory: Memory<'data>,
    pub symbols: Vec<Range<usize>>,
}

#[cfg(target_os="linux")]
impl<'data> Image<'data> {
    fn read<P: AsRef<Path>>(
        data: &'data [u8],
        exe_path: Option<P>,
        load_functions: bool,
    ) -> Result<Image<'data>> {
        todo!("Not supported on Linux")
    }
    
    fn read_inner<'memory, P: AsRef<Path>>(
        exe_path: Option<P>,
        load_functions: bool,
        memory: Memory<'memory>,
        object: object::File,
    ) -> Result<Image<'memory>> {
        todo!("Not supported on Linux")
    }

    pub fn get_function(
        &self,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        self.get_root_function(address)
    }

    pub fn get_root_function(
        &self,
        address: usize,
    ) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        let idx = self.symbols.binary_search_by(|p| match p.start.cmp(&address) {
            Ordering::Equal => Ordering::Greater,
            ord => ord,
        }).unwrap_err();
        if idx >= self.symbols.len() {
            Ok(None)
        } else {
            let range = &self.symbols[idx];
            if range.contains(&address) {
                Ok(Some(RuntimeFunction {
                    range: range.clone(),
                    unwind: 0
                }))
            } else {
                Ok(None)
            }
        }
    }

    pub fn get_child_functions(
        &self,
        address: usize,
    ) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        todo!("Not supported on Linux")
    }
}

// OS-independent
impl<'data> Image<'data> {
    pub fn builder() -> ImageBuilder {
        Default::default()
    }
    pub fn resolve<T: Send + Sync>(
        &self,
        resolver: &'static resolvers::ResolverFactory<T>,
    ) -> resolvers::Result<T> {
        resolvers::resolve(self, resolver)
    }

    pub fn resolve_many(
        &self,
        resolvers: &[fn() -> &'static resolvers::DynResolverFactory],
    ) -> Vec<resolvers::Result<std::sync::Arc<dyn resolvers::Resolution>>> {
        resolvers::resolve_many(self, resolvers)
    }

    pub fn scan<'patterns, S>(
        &self,
        pattern_configs: &'patterns [PatternConfig<S>],
    ) -> Result<ScanResult<'patterns, S>> {
        let mut results = vec![];

        struct PendingScan {
            original_config_index: usize,
            stages: ResolveStages,
            scan: Scan,
        }

        let mut scan_queue = pattern_configs
            .iter()
            .enumerate()
            .map(|(index, config)| PendingScan {
                original_config_index: index,
                stages: ResolveStages(vec![]),
                scan: config.scan.clone(), // TODO clone isn't ideal but makes handling multi-stage scans a lot easier
            })
            .collect::<Vec<_>>();

        while !scan_queue.is_empty() {
            let mut new_queue = vec![];
            for section in self.memory.sections() {
                let base_address = section.address();
                let section_name = section.name();
                let data = section.data();

                let (pattern_scans, patterns): (Vec<_>, Vec<_>) = scan_queue
                    .iter()
                    .filter_map(|scan| {
                        scan.scan
                            .section
                            .map(|s| s == section.kind())
                            .unwrap_or(true)
                            .then(|| {
                                scan.scan
                                    .scan_type
                                    .get_pattern()
                                    .map(|pattern| (scan, pattern))
                            })
                            .flatten()
                    })
                    .unzip();

                let (xref_scans, xrefs): (Vec<_>, Vec<_>) = scan_queue
                    .iter()
                    .filter_map(|scan| {
                        scan.scan
                            .section
                            .map(|s| s == section.kind())
                            .unwrap_or(true)
                            .then(|| scan.scan.scan_type.get_xref().map(|xref| (scan, xref)))
                            .flatten()
                    })
                    .unzip();

                let scan_results = scanner::scan_pattern(&patterns, base_address, data)
                    .into_iter()
                    .chain(scanner::scan_xref(&xrefs, base_address, data))
                    .zip(pattern_scans.iter().chain(xref_scans.iter()));

                for (addresses, scan) in scan_results {
                    for address in addresses {
                        let mut stages = scan.stages.clone();
                        let action = (scan.scan.resolve)(
                            ResolveContext {
                                exe: self,
                                memory: &self.memory,
                                section: section_name.to_owned(),
                                match_address: address,
                                scan: &scan.scan,
                            },
                            &mut stages,
                        );
                        match action {
                            ResolutionAction::Continue(new_scan) => {
                                new_queue.push(PendingScan {
                                    original_config_index: scan.original_config_index,
                                    stages,
                                    scan: new_scan,
                                });
                            }
                            ResolutionAction::Finish(res) => {
                                results.push((
                                    &pattern_configs[scan.original_config_index],
                                    Resolution {
                                        stages: stages.0,
                                        res,
                                    },
                                ));
                            }
                        }
                    }
                }
            }
            scan_queue = new_queue;
        }

        Ok(ScanResult { results })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeFunction {
    pub range: Range<usize>,
    pub unwind: usize,
}
impl RuntimeFunction {
    pub fn read<'data>(
        memory: &impl MemoryTrait<'data>,
        base_address: usize,
        address: usize,
    ) -> Result<Self, MemoryAccessError> {
        let addr_begin = base_address + memory.u32_le(address)? as usize;
        let addr_end = base_address + memory.u32_le(address + 4)? as usize;
        let unwind = base_address + memory.u32_le(address + 8)? as usize;

        Ok(RuntimeFunction {
            range: addr_begin..addr_end,
            unwind,
        })
    }
}
impl RuntimeFunction {
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum MemoryAccessError {
    MemoryOutOfBoundsError,
    Utf8Error,
    Utf16Error,
}
impl std::error::Error for MemoryAccessError {}
impl std::fmt::Display for MemoryAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MemoryOutOfBoundsError => write!(f, "MemoryOutOfBoundsError"),
            Self::Utf8Error => write!(f, "Utf8Error"),
            Self::Utf16Error => write!(f, "Utf16Error"),
        }
    }
}
impl From<std::str::Utf8Error> for MemoryAccessError {
    fn from(_: std::str::Utf8Error) -> Self {
        Self::Utf8Error
    }
}
impl From<std::string::FromUtf16Error> for MemoryAccessError {
    fn from(_: std::string::FromUtf16Error) -> Self {
        Self::Utf16Error
    }
}

/// Continuous section of memory
pub trait MemoryBlockTrait<'data> {
    /// Return starting address of block
    fn address(&self) -> usize;
    /// Returned contained memory
    fn data(&self) -> &[u8];
}

/// Potentially sparse section of memory
pub trait MemoryTrait<'data> {
    /// Return u8 at `address`
    fn index(&self, address: usize) -> Result<u8, MemoryAccessError>;
    /// Return slice of u8 at `range`
    fn range(&self, range: Range<usize>) -> Result<&[u8], MemoryAccessError>;
    /// Return slice of u8 from start of `range` to end of block
    fn range_from(&self, range: RangeFrom<usize>) -> Result<&[u8], MemoryAccessError>;
    /// Return slice of u8 from end of `range` to start of block (not useful because start of block
    /// is unknown to caller)
    fn range_to(&self, range: RangeTo<usize>) -> Result<&[u8], MemoryAccessError>;
}

/// Memory accessor helpers
pub trait MemoryAccessorTrait<'data>: MemoryTrait<'data> {
    /// Return i16 at `address`
    fn i16_le(&self, address: usize) -> Result<i16, MemoryAccessError> {
        Ok(i16::from_le_bytes(
            self.range(address..address + std::mem::size_of::<i16>())?
                .try_into()
                .unwrap(),
        ))
    }
    /// Return u16 at `address`
    fn u16_le(&self, address: usize) -> Result<u16, MemoryAccessError> {
        Ok(u16::from_le_bytes(
            self.range(address..address + std::mem::size_of::<u16>())?
                .try_into()
                .unwrap(),
        ))
    }
    /// Return i32 at `address`
    fn i32_le(&self, address: usize) -> Result<i32, MemoryAccessError> {
        Ok(i32::from_le_bytes(
            self.range(address..address + std::mem::size_of::<i32>())?
                .try_into()
                .unwrap(),
        ))
    }
    /// Return u32 at `address`
    fn u32_le(&self, address: usize) -> Result<u32, MemoryAccessError> {
        Ok(u32::from_le_bytes(
            self.range(address..address + std::mem::size_of::<u32>())?
                .try_into()
                .unwrap(),
        ))
    }
    /// Return u64 at `address`
    fn u64_le(&self, address: usize) -> Result<u64, MemoryAccessError> {
        Ok(u64::from_le_bytes(
            self.range(address..address + std::mem::size_of::<u64>())?
                .try_into()
                .unwrap(),
        ))
    }
    /// Return ptr (usize) at `address`
    fn ptr(&self, address: usize) -> Result<usize, MemoryAccessError> {
        Ok(self.u64_le(address)? as usize)
    }
    /// Return instruction relative address at `address`
    fn rip4(&self, address: usize) -> Result<usize, MemoryAccessError> {
        Ok((address + 4)
            .checked_add_signed(self.i32_le(address)? as isize)
            .unwrap())
    }

    /// Read null terminated string from `address`
    fn read_string(&self, address: usize) -> Result<String, MemoryAccessError> {
        let data = &self
            .range_from(address..)?
            .iter()
            .cloned()
            .take_while(|n| *n != 0)
            .collect::<Vec<u8>>();

        Ok(std::str::from_utf8(data)?.to_string())
    }

    /// Read null terminated wide string from `address`
    fn read_wstring(&self, address: usize) -> Result<String, MemoryAccessError> {
        let data = &self
            .range_from(address..)?
            .chunks(2)
            .map(|chunk| ((chunk[1] as u16) << 8) + chunk[0] as u16)
            .take_while(|n| *n != 0)
            .collect::<Vec<u16>>();

        Ok(String::from_utf16(data)?)
    }
}

impl<'data, T: MemoryTrait<'data>> MemoryAccessorTrait<'data> for T {}

impl<'data, T: MemoryBlockTrait<'data>> MemoryTrait<'data> for T {
    fn index(&self, address: usize) -> Result<u8, MemoryAccessError> {
        // TODO bounds
        Ok(self.data()[address - self.address()])
    }
    fn range(&self, range: Range<usize>) -> Result<&[u8], MemoryAccessError> {
        // TODO bounds
        Ok(&self.data()[range.start - self.address()..range.end - self.address()])
    }
    fn range_from(&self, range: RangeFrom<usize>) -> Result<&[u8], MemoryAccessError> {
        // TODO bounds
        Ok(&self.data()[range.start - self.address()..])
    }
    fn range_to(&self, range: RangeTo<usize>) -> Result<&[u8], MemoryAccessError> {
        // TODO bounds
        Ok(&self.data()[..range.end - self.address()])
    }
}

impl<'data> MemoryTrait<'data> for Memory<'data> {
    fn index(&self, address: usize) -> Result<u8, MemoryAccessError> {
        self.get_section_containing(address)?.index(address)
    }
    fn range(&self, range: Range<usize>) -> Result<&[u8], MemoryAccessError> {
        self.get_section_containing(range.start)?.range(range)
    }
    fn range_from(&self, range: RangeFrom<usize>) -> Result<&[u8], MemoryAccessError> {
        self.get_section_containing(range.start)?.range_from(range)
    }
    fn range_to(&self, range: RangeTo<usize>) -> Result<&[u8], MemoryAccessError> {
        self.get_section_containing(range.end)?.range_to(range)
    }
}

pub struct MemorySection<'data> {
    address: usize,
    data: Cow<'data, [u8]>,
}
impl<'data> MemoryBlockTrait<'data> for MemorySection<'data> {
    fn address(&self) -> usize {
        self.address
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
}

pub struct NamedMemorySection<'data> {
    name: String,
    kind: object::SectionKind,
    section: MemorySection<'data>,
}

impl<'data> NamedMemorySection<'data> {
    fn new<T: Into<Cow<'data, [u8]>>>(
        name: String,
        address: usize,
        kind: object::SectionKind,
        data: T,
    ) -> Self {
        Self {
            name,
            kind,
            section: MemorySection {
                address,
                data: data.into(),
            },
        }
    }
}
impl NamedMemorySection<'_> {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> object::SectionKind {
        self.kind
    }
    pub fn address(&self) -> usize {
        self.section.address()
    }
    pub fn data(&self) -> &[u8] {
        self.section.data()
    }
    pub fn len(&self) -> usize {
        self.section.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.section.data.is_empty()
    }
}
impl<'data> MemoryBlockTrait<'data> for NamedMemorySection<'data> {
    fn address(&self) -> usize {
        self.section.address()
    }
    fn data(&self) -> &[u8] {
        self.section.data()
    }
}

pub struct Memory<'data> {
    sections: Vec<NamedMemorySection<'data>>,
}

impl<'data> Memory<'data> {
    pub fn new(object: &File<'data>) -> Result<Self> {
        Ok(Self {
            sections: object
                .sections()
                .map(|s| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address() as usize,
                        s.kind(),
                        s.data()?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn new_external_data(sections: Vec<(object::Section<'_, '_>, Vec<u8>)>) -> Result<Self> {
        Ok(Self {
            sections: sections
                .into_iter()
                .map(|(s, d)| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address() as usize,
                        s.kind(),
                        d,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn new_internal_data(
        sections: Vec<(object::Section<'_, '_>, &'data [u8])>,
    ) -> Result<Self> {
        Ok(Self {
            sections: sections
                .into_iter()
                .map(|(s, d)| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address() as usize,
                        s.kind(),
                        d,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn sections(&self) -> &[NamedMemorySection] {
        &self.sections
    }
    pub fn get_section_containing(
        &self,
        address: usize,
    ) -> Result<&NamedMemorySection<'data>, MemoryAccessError> {
        self.sections
            .iter()
            .find(|section| {
                address >= section.section.address
                    && address < section.section.address + section.section.data.len()
            })
            .ok_or(MemoryAccessError::MemoryOutOfBoundsError)
    }
    pub fn find<F>(&self, kind: object::SectionKind, filter: F) -> Option<usize>
    where
        F: Fn(usize, &[u8]) -> bool,
    {
        self.sections.iter().find_map(|section| {
            if section.kind == kind {
                section
                    .section
                    .data
                    .windows(4)
                    .enumerate()
                    .find_map(|(i, slice)| {
                        filter(section.section.address + i, slice)
                            .then_some(section.section.address + i)
                    })
            } else {
                None
            }
        })
    }
}
impl<'data> Index<usize> for Memory<'data> {
    type Output = u8;
    fn index(&self, index: usize) -> &Self::Output {
        self.sections
            .iter()
            .find_map(|section| section.section.data.get(index - section.section.address))
            .unwrap()
    }
}
impl<'data> Index<Range<usize>> for Memory<'data> {
    type Output = [u8];
    fn index(&self, index: Range<usize>) -> &Self::Output {
        self.sections
            .iter()
            .find_map(|section| {
                if index.start >= section.section.address
                    && index.end <= section.section.address + section.section.data.len()
                {
                    let relative_range =
                        index.start - section.section.address..index.end - section.section.address;
                    Some(&section.section.data[relative_range])
                } else {
                    None
                }
            })
            .unwrap()
    }
}

pub trait Addressable {
    fn rip(&self) -> usize;
    fn ptr(&self) -> usize;
    fn u32(&self) -> u32;
}
impl Addressable for patternsleuth_scanner::Capture<'_> {
    fn rip(&self) -> usize {
        (self.address + 4)
            .checked_add_signed(i32::from_le_bytes(self.data.try_into().unwrap()) as isize)
            .unwrap()
    }
    fn ptr(&self) -> usize {
        usize::from_le_bytes(self.data.try_into().unwrap())
    }
    fn u32(&self) -> u32 {
        u32::from_le_bytes(self.data.try_into().unwrap())
    }
}

pub trait Matchable<'data> {
    fn captures(
        &'data self,
        pattern: &Pattern,
        address: usize,
    ) -> Result<Option<Vec<patternsleuth_scanner::Capture<'data>>>, MemoryAccessError>;
}

impl<'data> Matchable<'data> for Memory<'data> {
    fn captures(
        &'data self,
        pattern: &Pattern,
        address: usize,
    ) -> Result<Option<Vec<patternsleuth_scanner::Capture<'data>>>, MemoryAccessError> {
        let s = self.get_section_containing(address)?;
        // TODO bounds check data passed to captures
        Ok(pattern.captures(s.data(), s.address(), address - s.address()))
    }
}

pub mod disassemble {
    use std::{collections::HashSet, ops::Range};

    use iced_x86::{Decoder, DecoderOptions, FlowControl, Formatter, Instruction, NasmFormatter};

    use crate::{Image, MemoryAccessError, MemoryTrait};

    pub fn function_range(
        exe: &Image<'_>,
        address: usize,
    ) -> Result<Range<usize>, MemoryAccessError> {
        let min = address;
        let mut max = min;
        disassemble(exe, address, |inst| {
            let cur = inst.ip() as usize;
            if Some(address) != exe.get_root_function(cur)?.map(|f| f.range.start) {
                return Ok(Control::Break);
            }
            max = max.max(cur + inst.len());
            Ok(Control::Continue)
        })?;
        Ok(min..max)
    }

    pub fn disassemble_single<'mem, 'img: 'mem>(
        exe: &'img Image<'mem>,
        address: usize,
    ) -> Result<Option<Instruction>, MemoryAccessError> {
        Ok(Decoder::with_ip(
            64,
            exe.memory.range_from(address..)?,
            address as u64,
            DecoderOptions::NONE,
        )
        .iter()
        .next())
    }

    pub enum Control {
        Continue,
        Break,
        Exit,
    }

    pub fn disassemble<'mem, 'img: 'mem, F>(
        exe: &'img Image<'mem>,
        address: usize,
        mut visitor: F,
    ) -> Result<(), MemoryAccessError>
    where
        F: FnMut(&Instruction) -> Result<Control, MemoryAccessError>,
    {
        struct Ctx<'mem, 'img: 'mem> {
            exe: &'img Image<'mem>,
            queue: Vec<usize>,
            visited: HashSet<usize>,
            address: usize,
            block: &'mem [u8],
            decoder: Decoder<'mem>,
            instruction: Instruction,
        }

        let block = exe.memory.range_from(address..)?;
        let mut ctx = Ctx {
            exe,
            queue: Default::default(),
            visited: Default::default(),
            address,
            block,
            decoder: Decoder::with_ip(64, block, address as u64, DecoderOptions::NONE),
            instruction: Default::default(),
        };

        impl<'img, 'mem> Ctx<'img, 'mem> {
            #[allow(unused)]
            fn print(&self) {
                let mut formatter = NasmFormatter::new();
                //formatter.options_mut().set_digit_separator("`");
                //formatter.options_mut().set_first_operand_char_index(10);

                let mut output = String::new();
                formatter.format(&self.instruction, &mut output);

                // Eg. "00007FFAC46ACDB2 488DAC2400FFFFFF     lea       rbp,[rsp-100h]"
                print!("{:016X} ", self.instruction.ip());
                let start_index = self.instruction.ip() as usize - self.address;
                let instr_bytes = &self.block[start_index..start_index + self.instruction.len()];
                for b in instr_bytes.iter() {
                    print!("{:02X}", b);
                }
                if instr_bytes.len() < 0x10 {
                    for _ in 0..0x10 - instr_bytes.len() {
                        print!("  ");
                    }
                }
                println!(" {}", output);
            }
            fn start(&mut self, address: usize) -> Result<(), MemoryAccessError> {
                //println!("starting at {address:x}");
                self.address = address;
                self.block = self.exe.memory.range_from(self.address..)?;
                self.decoder =
                    Decoder::with_ip(64, self.block, self.address as u64, DecoderOptions::NONE);
                Ok(())
            }
            /// Returns true if pop was successful
            fn pop(&mut self) -> Result<bool, MemoryAccessError> {
                Ok(if let Some(next) = self.queue.pop() {
                    self.start(next)?;
                    true
                } else {
                    false
                })
            }
        }

        while ctx.decoder.can_decode() {
            ctx.decoder.decode_out(&mut ctx.instruction);

            let addr = ctx.instruction.ip() as usize;

            if ctx.visited.contains(&addr) {
                if ctx.pop()? {
                    continue;
                } else {
                    break;
                }
            } else {
                ctx.visited.insert(addr);
                match visitor(&ctx.instruction)? {
                    Control::Continue => {}
                    Control::Break => {
                        if ctx.pop()? {
                            continue;
                        } else {
                            break;
                        }
                    }
                    Control::Exit => {
                        break;
                    }
                }
            }

            /*
            if !matches!(ctx.instruction.flow_control(), FlowControl::Next) {
                //println!();
            }
            ctx.print();
            if !matches!(ctx.instruction.flow_control(), FlowControl::Next) {
                //println!();
            }
            */

            match ctx.instruction.flow_control() {
                FlowControl::Next => {}
                FlowControl::UnconditionalBranch => {
                    // TODO figure out how to handle tail calls
                    ctx.start(ctx.instruction.near_branch_target() as usize)?;
                }
                //FlowControl::IndirectBranch => todo!(),
                FlowControl::ConditionalBranch => {
                    ctx.queue
                        .push(ctx.instruction.near_branch_target() as usize);
                }
                FlowControl::Return => {
                    if !ctx.pop()? {
                        break;
                    }
                }
                //FlowControl::Call => todo!(),
                //FlowControl::IndirectCall => todo!(),
                //FlowControl::Interrupt => todo!(),
                //FlowControl::XbeginXabortXend => todo!(),
                //FlowControl::Exception => todo!(),
                _ => {}
            }
        }
        Ok(())
    }
}
