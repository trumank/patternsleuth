#![feature(portable_simd)]

pub mod patterns;
pub mod symbols;

pub mod scanner {
    pub use patternsleuth_scanner::*;
}

use std::{
    collections::{BTreeMap, HashMap},
    ops::{Index, Range, RangeFrom, RangeTo},
    path::Path,
};

use scanner::{Pattern, Xref};

use anyhow::{Context, Result};
use byteorder::{ReadBytesExt, LE};
use object::{File, Object, ObjectSection};

use patterns::Sig;

pub struct ResolveContext<'data, 'pattern> {
    pub exe: &'data Executable<'data>,
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
    /// error during resolution or failes some criteria
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
pub struct PatternConfig {
    pub sig: Sig,
    pub name: String,
    pub scan: Scan,
}
impl PatternConfig {
    pub fn new(
        sig: Sig,
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
        sig: Sig,
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

pub struct Executable<'data> {
    pub data: &'data [u8],
    pub exception_data: MemorySection<'data>,
    pub object: object::File<'data>,
    pub memory: Memory<'data>,
    pub functions: Option<Vec<RuntimeFunctionNode>>,
    pub symbols: Option<HashMap<usize, String>>,
}
fn read_functions(
    object: &File<'_>,
    data: &[u8],
    memory: &Memory<'_>,
) -> Result<Vec<RuntimeFunctionNode>> {
    Ok(match object {
        object::File::Pe64(ref inner) => {
            let exception_directory = inner
                .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                .context("no exception directory")?;

            let base_address = object.relative_address_base() as usize;

            let data = exception_directory.data(data, &inner.section_table())?;
            let count = data.len() / 12;
            let mut cur = std::io::Cursor::new(data);

            let mut functions: BTreeMap<usize, RuntimeFunctionNode> = Default::default();
            let mut children = vec![];
            for _ in 0..count {
                let range = base_address + cur.read_u32::<LE>()? as usize
                    ..base_address + cur.read_u32::<LE>()? as usize;
                let unwind = base_address + cur.read_u32::<LE>()? as usize;

                let function = RuntimeFunctionNode {
                    range,
                    unwind,
                    children: vec![],
                };

                let section = memory
                    .get_section_containing(unwind)
                    .context("out of bounds reading unwind info")?;

                let mut offset = unwind - section.section.address;

                let has_chain_info = section.section.data[offset] >> 3 == 0x4;
                if has_chain_info {
                    let unwind_code_count = section.section.data[offset + 2];

                    offset += 4 + 2 * unwind_code_count as usize;
                    if offset % 4 != 0 {
                        // align
                        offset += 2;
                    }

                    if section.section.data.len() > offset {
                        let parent = base_address
                            + u32::from_le_bytes(
                                section.section.data[offset..offset + 4].try_into().unwrap(),
                            ) as usize;
                        children.push((parent, function.clone()));
                    } else {
                        dbg!("not adding chain info {offset}");
                    }
                }
                functions.insert(function.range.start, function);
            }
            for (parent, child) in children {
                functions.get_mut(&parent).unwrap().children.push(child);
            }

            functions.into_values().collect()
        }
        object::File::Pe32(_) => {
            vec![]
        }
        _ => todo!("{:?}", object::FileKind::parse(data)?),
    })
}
impl<'data> Executable<'data> {
    pub fn read<P: AsRef<Path>>(
        data: &'data [u8],
        exe_path: P,
        load_symbols: bool,
        load_functions: bool,
    ) -> Result<Executable<'data>> {
        let object = object::File::parse(data)?;
        let memory = Memory::new(&object)?;

        let base_address = object.relative_address_base() as usize;

        let pdb_path = exe_path.as_ref().with_extension("pdb");
        let symbols = (load_symbols && pdb_path.exists())
            .then(|| symbols::dump_pdb_symbols(pdb_path, base_address))
            .transpose()?;

        let exception_data = match object {
            object::File::Pe64(ref inner) => {
                let exception_directory = inner
                    .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                    .context("no exception directory")?;

                MemorySection {
                    address: base_address
                        + exception_directory
                            .virtual_address
                            .get(object::LittleEndian) as usize,
                    data: exception_directory.data(data, &inner.section_table())?,
                }
            }
            _ => MemorySection {
                address: 0,
                data: &[],
            },
        };

        let functions = load_functions
            .then(|| {
                read_functions(&object, data, &memory)
                    .map_err(|e| println!("Failed to parse exceptions: {e}"))
                    .ok()
            })
            .flatten();

        Ok(Executable {
            data,
            exception_data,
            object,
            memory,
            functions,
            symbols,
        })
    }
    fn read_exception(&self, address: usize) -> Option<RuntimeFunction> {
        let base_address = self.object.relative_address_base() as usize;

        for i in (self.exception_data.address()
            ..self.exception_data.address + self.exception_data.data.len())
            .step_by(12)
        {
            let addr_begin = base_address + self.exception_data.u32_le(i) as usize;
            if addr_begin <= address {
                let addr_end = base_address + self.exception_data.u32_le(i + 4) as usize;
                if addr_end > address {
                    let unwind = base_address + self.exception_data.u32_le(i + 8) as usize;

                    return Some(RuntimeFunction {
                        range: addr_begin..addr_end,
                        unwind,
                    });
                }
            }
        }
        None
    }
    pub fn get_function(&self, address: usize) -> Option<RuntimeFunctionNode> {
        let base_address = self.object.relative_address_base() as usize;

        // TODO binary search
        for i in (self.exception_data.address()
            ..self.exception_data.address + self.exception_data.data.len())
            .step_by(12)
        {
            let addr_begin = base_address + self.exception_data.u32_le(i) as usize;
            if addr_begin <= address {
                let addr_end = base_address + self.exception_data.u32_le(i + 4) as usize;
                if addr_end > address {
                    let unwind = base_address + self.exception_data.u32_le(i + 8) as usize;

                    let mut f = RuntimeFunctionNode {
                        range: addr_begin..addr_end,
                        unwind,
                        children: vec![],
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
                                let addr_begin =
                                    base_address + section.u32_le(unwind_addr) as usize;
                                let addr_end =
                                    base_address + section.u32_le(unwind_addr + 4) as usize;
                                let unwind =
                                    base_address + section.u32_le(unwind_addr + 8) as usize;

                                let mut children = std::mem::take(&mut f.children);
                                children.push(f.clone());

                                f = RuntimeFunctionNode {
                                    range: addr_begin..addr_end,
                                    unwind,
                                    children,
                                };
                            } else {
                                todo!("not adding chain info {unwind_addr}");
                            }
                        } else {
                            return Some(f);
                        }
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeFunctionNode {
    pub range: Range<usize>,
    pub unwind: usize,
    pub children: Vec<RuntimeFunctionNode>,
}
#[derive(Debug, Clone)]
pub struct RuntimeFunction {
    pub range: Range<usize>,
    pub unwind: usize,
}
impl RuntimeFunctionNode {
    /// Return range accounting for chained entries
    /// This may include gaps not belonging to the function if chains are sparse
    pub fn full_range(&self) -> Range<usize> {
        let start = std::iter::once(self)
            .chain(self.children.iter())
            .map(|f| f.range.start)
            .min()
            .unwrap();
        let end = std::iter::once(self)
            .chain(self.children.iter())
            .map(|f| f.range.end)
            .max()
            .unwrap();
        start..end
    }
}

/// Continuous section of memory
pub trait MemoryBlockTrait<'data> {
    /// Return starting address of block
    fn address(&self) -> usize;
    /// Returned contained memory
    fn data(&self) -> &'data [u8];
}

/// Potentially sparse section of memory
pub trait MemoryTrait<'data> {
    /// Return u8 at `address`
    fn index(&self, address: usize) -> u8;
    /// Return slice of u8 at `range`
    fn range(&self, range: Range<usize>) -> &'data [u8];
    /// Return slice of u8 from start of `range` to end of block
    fn range_from(&self, range: RangeFrom<usize>) -> &'data [u8];
    /// Return slice of u8 from end of `range` to start of block (not useful because start of block
    /// is unknown to caller)
    fn range_to(&self, range: RangeTo<usize>) -> &'data [u8];
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
    fn range(&self, range: Range<usize>) -> &'data [u8] {
        &self.data()[range.start - self.address()..range.end - self.address()]
    }
    fn range_from(&self, range: RangeFrom<usize>) -> &'data [u8] {
        &self.data()[range.start - self.address()..]
    }
    fn range_to(&self, range: RangeTo<usize>) -> &'data [u8] {
        &self.data()[..range.end - self.address()]
    }
}

impl<'data> MemoryTrait<'data> for Memory<'data> {
    fn index(&self, address: usize) -> u8 {
        self.get_section_containing(address).unwrap().index(address)
    }
    fn range(&self, range: Range<usize>) -> &'data [u8] {
        self.get_section_containing(range.start)
            .unwrap()
            .range(range)
    }
    fn range_from(&self, range: RangeFrom<usize>) -> &'data [u8] {
        self.get_section_containing(range.start)
            .unwrap()
            .range_from(range)
    }
    fn range_to(&self, range: RangeTo<usize>) -> &'data [u8] {
        self.get_section_containing(range.end)
            .unwrap()
            .range_to(range)
    }
}

pub struct MemorySection<'data> {
    address: usize,
    data: &'data [u8],
}
impl<'data> MemoryBlockTrait<'data> for MemorySection<'data> {
    fn address(&self) -> usize {
        self.address
    }
    fn data(&self) -> &'data [u8] {
        self.data
    }
}

pub struct NamedMemorySection<'data> {
    name: String,
    kind: object::SectionKind,
    section: MemorySection<'data>,
}

impl<'data> NamedMemorySection<'data> {
    fn new(name: String, address: usize, kind: object::SectionKind, data: &'data [u8]) -> Self {
        Self {
            name,
            kind,
            section: MemorySection { address, data },
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
    fn data(&self) -> &'data [u8] {
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
