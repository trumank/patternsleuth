#![feature(portable_simd, str_split_whitespace_remainder)]

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
    borrow::Cow,
    collections::HashMap,
    ops::{Index, Range, RangeFrom, RangeTo},
    path::Path,
};

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

pub struct ImageBuilder<P: AsRef<Path>> {
    symbols: Option<P>,
    functions: bool,
}
impl<P: AsRef<Path>> Default for ImageBuilder<P> {
    fn default() -> Self {
        Self {
            symbols: None,
            functions: false,
        }
    }
}
impl<P: AsRef<Path>> ImageBuilder<P> {
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

pub struct Image<'data> {
    pub base_address: usize,
    pub exception_directory_range: Range<usize>,
    pub exception_children_cache: HashMap<usize, Vec<RuntimeFunction>>,
    pub memory: Memory<'data>,
    pub symbols: Option<HashMap<usize, String>>,
}
impl<'data> Image<'data> {
    pub fn builder<P: AsRef<Path>>() -> ImageBuilder<P> {
        Default::default()
    }
    fn read<P: AsRef<Path>>(
        data: &'data [u8],
        exe_path: Option<P>,
        load_functions: bool,
    ) -> Result<Image<'data>> {
        let object = object::File::parse(data)?;
        let memory = Memory::new(&object)?;

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

        let exception_directory_range = match object {
            object::File::Pe64(ref inner) => {
                let exception_directory = inner
                    .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                    .context("no exception directory")?;

                let (address, size) = exception_directory.address_range();
                base_address + address as usize..base_address + (address + size) as usize
            }
            _ => 0..0,
        };

        let mut new = Image {
            base_address,
            exception_directory_range,
            exception_children_cache: Default::default(),
            memory,
            symbols,
        };

        if load_functions {
            new.populate_exception_cache();
        }
        Ok(new)
    }
    fn populate_exception_cache(&mut self) {
        for i in self.exception_directory_range.clone().step_by(12) {
            let f = RuntimeFunction::read(&self.memory, self.base_address, i);
            self.exception_children_cache.insert(f.range.start, vec![]);

            let Some(section) = self.memory.get_section_containing(f.unwind) else {
                // TODO disabled cause spammy
                //println!("invalid unwind info addr {:x}", f.unwind);
                continue;
            };

            let mut unwind = f.unwind;
            let has_chain_info = section.section.index(unwind) >> 3 == 0x4;
            if has_chain_info {
                let unwind_code_count = section.section.index(unwind + 2);

                unwind += 4 + 2 * unwind_code_count as usize;
                if unwind % 4 != 0 {
                    // align
                    unwind += 2;
                }

                if section.address() + section.data().len() > unwind + 12 {
                    let chained = RuntimeFunction::read(section, self.base_address, unwind);

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
    }
    /// Get function containing `address` from the exception directory
    pub fn get_function(&self, address: usize) -> Option<RuntimeFunction> {
        let size = 12;
        let mut min = 0;
        let mut max = self.exception_directory_range.len() / size - 1;

        while min <= max {
            let i = (max + min) / 2;
            let addr = i * size + self.exception_directory_range.start;

            let addr_begin = self.base_address + self.memory.u32_le(addr) as usize;
            if addr_begin <= address {
                let addr_end = self.base_address + self.memory.u32_le(addr + 4) as usize;
                if addr_end > address {
                    let unwind = self.base_address + self.memory.u32_le(addr + 8) as usize;

                    return Some(RuntimeFunction {
                        range: addr_begin..addr_end,
                        unwind,
                    });
                } else {
                    min = i + 1;
                }
            } else {
                max = i - 1;
            }
        }
        None
    }
    /// Get root function containing `address` from the exception directory. This can be used to
    /// find the start address of a function given an address in the body.
    pub fn get_root_function(&self, address: usize) -> Option<RuntimeFunction> {
        if let Some(f) = self.get_function(address) {
            let mut f = RuntimeFunction {
                range: f.range,
                unwind: f.unwind,
            };

            loop {
                let mut unwind_addr = f.unwind;

                let Some(section) = self.memory.get_section_containing(unwind_addr) else {
                    dbg!("out of bounds reading unwind info");
                    return None;
                };

                let has_chain_info = section.section.index(unwind_addr) >> 3 == 0x4;
                if has_chain_info {
                    let unwind_code_count = section.section.index(unwind_addr + 2);

                    unwind_addr += 4 + 2 * unwind_code_count as usize;
                    if unwind_addr % 4 != 0 {
                        // align
                        unwind_addr += 2;
                    }

                    if section.address() + section.data().len() > unwind_addr + 12 {
                        f = RuntimeFunction::read(section, self.base_address, unwind_addr);
                    } else {
                        todo!("not adding chain info {unwind_addr}");
                    }
                } else {
                    return Some(f);
                }
            }
        } else {
            None
        }
    }
    /// Recursively get all child functions of `address` (exact). This is pulled from
    /// `exception_children_cache` so will be empty if it has not been populated.
    pub fn get_child_functions(&self, address: usize) -> Vec<RuntimeFunction> {
        let mut queue = vec![address];
        let mut all_children = vec![self.get_function(address).unwrap()];
        while let Some(next) = queue.pop() {
            if let Some(children) = self.exception_children_cache.get(&next) {
                for child in children {
                    queue.push(child.range().start);
                    all_children.push(child.clone());
                }
            }
        }
        all_children
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
    ) -> Self {
        let addr_begin = base_address + memory.u32_le(address) as usize;
        let addr_end = base_address + memory.u32_le(address + 4) as usize;
        let unwind = base_address + memory.u32_le(address + 8) as usize;

        RuntimeFunction {
            range: addr_begin..addr_end,
            unwind,
        }
    }
}
impl RuntimeFunction {
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
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
    fn index(&self, address: usize) -> u8;
    /// Return slice of u8 at `range`
    fn range(&self, range: Range<usize>) -> &[u8];
    /// Return slice of u8 from start of `range` to end of block
    fn range_from(&self, range: RangeFrom<usize>) -> &[u8];
    /// Return slice of u8 from end of `range` to start of block (not useful because start of block
    /// is unknown to caller)
    fn range_to(&self, range: RangeTo<usize>) -> &[u8];
}

/// Memory accessor helpers
pub trait MemoryAccessorTrait<'data>: MemoryTrait<'data> {
    /// Return i32 at `address`
    fn i32_le(&self, address: usize) -> i32 {
        i32::from_le_bytes(
            self.range(address..address + std::mem::size_of::<i32>())
                .try_into()
                .unwrap(),
        )
    }
    /// Return u32 at `address`
    fn u32_le(&self, address: usize) -> u32 {
        u32::from_le_bytes(
            self.range(address..address + std::mem::size_of::<u32>())
                .try_into()
                .unwrap(),
        )
    }
    /// Return u64 at `address`
    fn u64_le(&self, address: usize) -> u64 {
        u64::from_le_bytes(
            self.range(address..address + std::mem::size_of::<u64>())
                .try_into()
                .unwrap(),
        )
    }
    /// Return ptr (usize) at `address`
    fn ptr(&self, address: usize) -> usize {
        self.u64_le(address) as usize
    }
    /// Return instruction relative address at `address`
    fn rip4(&self, address: usize) -> usize {
        (address + 4)
            .checked_add_signed(self.i32_le(address) as isize)
            .unwrap()
    }

    /// Read null terminated string from `address`
    fn read_string(&self, address: usize) -> String {
        let data = &self
            .range_from(address..)
            .iter()
            .cloned()
            .take_while(|n| *n != 0)
            .collect::<Vec<u8>>();

        std::str::from_utf8(data).unwrap().to_string()
    }

    /// Read null terminated wide string from `address`
    fn read_wstring(&self, address: usize) -> String {
        let data = &self
            .range_from(address..)
            .chunks(2)
            .map(|chunk| ((chunk[1] as u16) << 8) + chunk[0] as u16)
            .take_while(|n| *n != 0)
            .collect::<Vec<u16>>();

        String::from_utf16(data).unwrap()
    }
}

impl<'data, T: MemoryTrait<'data>> MemoryAccessorTrait<'data> for T {}

impl<'data, T: MemoryBlockTrait<'data>> MemoryTrait<'data> for T {
    fn index(&self, address: usize) -> u8 {
        self.data()[address - self.address()]
    }
    fn range(&self, range: Range<usize>) -> &[u8] {
        &self.data()[range.start - self.address()..range.end - self.address()]
    }
    fn range_from(&self, range: RangeFrom<usize>) -> &[u8] {
        &self.data()[range.start - self.address()..]
    }
    fn range_to(&self, range: RangeTo<usize>) -> &[u8] {
        &self.data()[..range.end - self.address()]
    }
}

impl<'data> MemoryTrait<'data> for Memory<'data> {
    fn index(&self, address: usize) -> u8 {
        self.get_section_containing(address).unwrap().index(address)
    }
    fn range(&self, range: Range<usize>) -> &[u8] {
        self.get_section_containing(range.start)
            .unwrap()
            .range(range)
    }
    fn range_from(&self, range: RangeFrom<usize>) -> &[u8] {
        self.get_section_containing(range.start)
            .unwrap()
            .range_from(range)
    }
    fn range_to(&self, range: RangeTo<usize>) -> &[u8] {
        self.get_section_containing(range.end)
            .unwrap()
            .range_to(range)
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
    pub fn get_section_containing(&self, address: usize) -> Option<&NamedMemorySection<'data>> {
        self.sections.iter().find(|section| {
            address >= section.section.address
                && address < section.section.address + section.section.data.len()
        })
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
