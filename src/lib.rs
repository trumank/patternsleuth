#![feature(portable_simd)]

pub mod patterns;
pub mod scanner;

use std::ops::{Index, Range};

use anyhow::{bail, Result};
use object::{File, Object, ObjectSection};

use patterns::Sig;

#[derive(Debug)]
pub struct Pattern {
    pub sig: Vec<u8>,
    pub mask: Vec<u8>,
    pub custom_offset: usize,
}

impl Pattern {
    pub fn new(s: &str) -> Result<Self> {
        let mut sig = vec![];
        let mut mask = vec![];
        let mut custom_offset = 0;
        for (i, w) in s.split_whitespace().enumerate() {
            if let Ok(b) = u8::from_str_radix(w, 16) {
                sig.push(b);
                mask.push(0xff);
                continue;
            } else if w == "??" {
                if sig.is_empty() {
                    bail!("first byte cannot be \"??\"");
                }
                sig.push(0);
                mask.push(0);
                continue;
            } else if w == "|" {
                custom_offset = i;
                continue;
            }
            bail!("bad pattern word \"{}\"", w);
        }

        Ok(Self {
            sig,
            mask,
            custom_offset,
        })
    }
    /// Create a pattern from a literal Vec<u8> with `mask` filled with 0xff and `custom_offset = 0`.
    pub fn from_bytes(sig: Vec<u8>) -> Result<Self> {
        Ok(Self {
            mask: vec![0xff; sig.len()],
            sig,
            custom_offset: 0,
        })
    }
    #[inline]
    pub fn is_match(&self, data: &[u8], index: usize) -> bool {
        for i in 0..self.mask.len() {
            if data[index + i] & self.mask[i] != self.sig[i] {
                return false;
            }
        }
        true
    }
    /// compute virtual address from address relative to section as well as account for
    /// custom_offset
    pub fn compute_result(&self, _data: &[u8], base_address: usize, index: usize) -> usize {
        base_address + index + self.custom_offset
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Xref(pub usize);

pub struct ResolveContext<'memory> {
    pub memory: &'memory MountedPE<'memory>,
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

pub struct Scan {
    pub section: Option<object::SectionKind>,
    pub scan_type: ScanType,
    pub resolve: Resolve,
}
pub enum ScanType {
    Pattern(Pattern),
    Xref(Xref),
}
impl ScanType {
    pub fn unwrap_pattern(&self) -> &Pattern {
        match self {
            Self::Pattern(pattern) => pattern,
            _ => unreachable!(),
        }
    }
}
impl From<Pattern> for ScanType {
    fn from(value: Pattern) -> Self {
        Self::Pattern(value)
    }
}

type Resolve = fn(ctx: ResolveContext) -> Resolution;
pub struct PatternConfig {
    pub sig: Sig,
    pub name: String,
    pub scan: Scan,
}
impl PatternConfig {
    fn new(
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
    pub fn new(object: &'data File) -> Result<Self> {
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
