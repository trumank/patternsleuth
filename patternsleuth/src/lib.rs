#![feature(portable_simd)]

pub mod patterns;
pub mod symbols;

pub mod scanner {
    pub use patternsleuth_scanner::*;
}

use std::{
    collections::{BTreeMap, HashMap},
    ops::{Index, Range},
    path::Path,
};

use scanner::{Pattern, Xref};

use anyhow::{Context, Result};
use byteorder::{ReadBytesExt, LE};
use object::{File, Object, ObjectSection};

use patterns::Sig;

pub struct ResolveContext<'data> {
    pub exe: &'data Executable<'data>,
    pub memory: &'data MountedPE<'data>,
    pub section: String,
    pub match_address: usize,
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
    data: &'data [u8],
    pub object: object::File<'data>,
    pub memory: MountedPE<'data>,
    functions: Vec<RuntimeFunction>,
    pub symbols: Option<HashMap<usize, String>>,
}
impl<'data> Executable<'data> {
    pub fn read<P: AsRef<Path>>(
        data: &'data [u8],
        exe_path: P,
        load_symbols: bool,
    ) -> Result<Executable<'data>> {
        let object = object::File::parse(data)?;
        let memory = MountedPE::new(&object)?;

        use std::io::Cursor;

        let base_address = object.relative_address_base() as usize;

        let pdb_path = exe_path.as_ref().with_extension("pdb");
        let symbols = (load_symbols && pdb_path.exists())
            .then(|| symbols::dump_pdb_symbols(pdb_path, base_address))
            .transpose()?;

        let functions = match object {
            object::File::Pe64(ref inner) => {
                // TODO entries are sorted so it should be possible to binary search entries
                // directly from the directory rather than read them all up front
                let exception_directory = inner
                    .data_directory(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
                    .context("no exception directory")?;

                let data = exception_directory.data(data, &inner.section_table())?;
                let count = data.len() / 12;
                let mut cur = Cursor::new(data);

                let mut functions: BTreeMap<usize, RuntimeFunction> = Default::default();
                let mut children = vec![];
                for _ in 0..count {
                    let range = base_address + cur.read_u32::<LE>()? as usize
                        ..base_address + cur.read_u32::<LE>()? as usize;
                    let unwind = base_address + cur.read_u32::<LE>()? as usize;

                    let function = RuntimeFunction {
                        range,
                        unwind,
                        children: vec![],
                    };

                    // TODO this is totally gross. need to implement a nice way to stream sparse sections
                    let section = memory
                        .get_section_containing(unwind)
                        .context("out of bounds reading unwind info")?;
                    let mut offset = unwind - section.address;

                    let has_chain_info = section.data[offset] >> 3 == 0x4;
                    if has_chain_info {
                        let unwind_code_count = section.data[offset + 2];

                        offset += 4 + 2 * unwind_code_count as usize;
                        if offset % 4 != 0 {
                            // align
                            offset += 2;
                        }

                        let mut tmp = [0; 4];
                        tmp.copy_from_slice(&section.data[offset..offset + 4]);
                        let parent = base_address + u32::from_le_bytes(tmp) as usize;
                        children.push((parent, function.clone()));
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
        };
        /*
        for (i, window) in functions.windows(2).enumerate() {
            match window {
                [a, b] => {
                    if a.range.end > b.range.start {
                        println!("overlapping exception table @ {} len = {}: {a:x?} {b:x?}", i, functions.len());
                    }
                    if a.range.start >= b.range.start {
                        println!("out of order exception table @ {} len = {}: {a:x?} {b:x?}", i, functions.len());
                    }
                }
                _ => unreachable!()
            }
        }
        */

        Ok(Executable {
            data,
            object,
            memory,
            functions,
            symbols,
        })
    }
    pub fn get_function(&self, address: usize) -> Option<&RuntimeFunction> {
        // TODO figure out what to do in the rare case of overlapping entries
        match self
            .functions
            .binary_search_by_key(&address, |f| f.range.start)
        {
            Ok(i) => Some(&self.functions[i]),
            Err(i) => {
                let f = &self.functions.get(i - 1);
                f.filter(|f| f.range.contains(&address))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeFunction {
    pub range: Range<usize>,
    pub unwind: usize,
    pub children: Vec<RuntimeFunction>,
}
impl RuntimeFunction {
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

pub struct PESection<'data> {
    pub name: String,
    pub address: usize,
    pub kind: object::SectionKind,
    pub data: &'data [u8],
}

impl<'data> PESection<'data> {
    fn new(name: String, address: usize, kind: object::SectionKind, data: &'data [u8]) -> Self {
        Self {
            name,
            address,
            kind,
            data,
        }
    }
}

pub struct MountedPE<'data> {
    sections: Vec<PESection<'data>>,
}

impl<'data> MountedPE<'data> {
    pub fn new(object: &File<'data>) -> Result<Self> {
        Ok(Self {
            sections: object
                .sections()
                .map(|s| {
                    Ok(PESection::new(
                        s.name()?.to_string(),
                        s.address() as usize,
                        s.kind(),
                        s.data()?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn get_section_containing(&self, address: usize) -> Option<&PESection> {
        self.sections.iter().find(|section| {
            address >= section.address && address < section.address + section.data.len()
        })
    }
    pub fn find<F>(&self, kind: object::SectionKind, filter: F) -> Option<usize>
    where
        F: Fn(usize, &[u8]) -> bool,
    {
        self.sections.iter().find_map(|section| {
            if section.kind == kind {
                section.data.windows(4).enumerate().find_map(|(i, slice)| {
                    filter(section.address + i, slice).then_some(section.address + i)
                })
            } else {
                None
            }
        })
    }
}
impl<'data> Index<usize> for MountedPE<'data> {
    type Output = u8;
    fn index(&self, index: usize) -> &Self::Output {
        self.sections
            .iter()
            .find_map(|section| section.data.get(index - section.address))
            .unwrap()
    }
}
impl<'data> Index<Range<usize>> for MountedPE<'data> {
    type Output = [u8];
    fn index(&self, index: Range<usize>) -> &Self::Output {
        self.sections
            .iter()
            .find_map(|section| {
                if index.start >= section.address
                    && index.end <= section.address + section.data.len()
                {
                    let relative_range = index.start - section.address..index.end - section.address;
                    Some(&section.data[relative_range])
                } else {
                    None
                }
            })
            .unwrap()
    }
}
