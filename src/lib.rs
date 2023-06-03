#![feature(portable_simd)]

pub mod patterns;
pub mod scanner;

use std::ops::{Index, Range};

use anyhow::{bail, Result};
use object::{File, Object, ObjectSection};

use patterns::Sig;

#[derive(Debug)]
enum PatternMode {
    Direct,
    Relative32(isize),
}

#[derive(Debug)]
pub struct Pattern {
    sig: Vec<u8>,
    mask: Vec<u8>,
    mode: PatternMode,
}

impl Pattern {
    pub fn new(s: &str) -> Result<Self> {
        let mut sig = vec![];
        let mut mask = vec![];
        let mut mode = PatternMode::Direct;

        for w in s.split_whitespace() {
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
            } else if let Some(r) = w.strip_prefix("R32") {
                if r.is_empty() {
                    mode = PatternMode::Relative32(sig.len() as isize);
                    continue;
                } else if let Ok(offset) = isize::from_str_radix(r, 16) {
                    mode = PatternMode::Relative32(sig.len() as isize + offset);
                    continue;
                }
            }
            bail!("bad pattern word \"{}\"", w);
        }

        Ok(Self { sig, mask, mode })
    }
    #[inline]
    fn is_match(&self, data: &[u8], index: usize) -> bool {
        for i in 0..self.mask.len() {
            if data[index + i] & self.mask[i] != self.sig[i] {
                return false;
            }
        }
        true
    }
    fn compute_result(&self, data: &[u8], base_address: usize, index: usize) -> usize {
        base_address
            + match self.mode {
                PatternMode::Direct => index,
                PatternMode::Relative32(offset) => {
                    let n = index.checked_add_signed(offset).unwrap();
                    n.checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .unwrap()
                }
            }
    }
}

#[derive(Debug)]
pub struct Resolution {
    /// name of section pattern was found in
    pub section: String,
    /// intermediate addresses of interest before reaching the final address
    pub stages: Vec<usize>,
    /// final, fully resolved address
    pub address: Option<usize>,
}

type Resolve = fn(memory: &MountedPE, section: String, match_address: usize) -> Resolution;
pub struct PatternConfig {
    pub sig: Sig,
    pub name: String,
    pub section: Option<object::SectionKind>,
    pub pattern: Pattern,
    pub resolve: Resolve,
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
            section,
            pattern,
            resolve,
        }
    }
}

pub struct PESection<'data> {
    pub name: String,
    pub address: usize,
    pub data: &'data [u8],
}

impl<'data> PESection<'data> {
    fn new(name: String, address: usize, data: &'data [u8]) -> Self {
        Self {
            name,
            address,
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
                    && index.end < section.address + section.data.len()
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
