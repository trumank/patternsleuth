pub mod image;
pub mod process;
#[cfg(feature = "symbols")]
pub mod symbols;
#[cfg(feature = "symbols")]
pub mod uesym;

pub mod scanner {
    pub use patternsleuth_scanner::*;
}

use scanner::{Pattern, Xref};
use std::{
    borrow::Cow,
    collections::HashMap,
    ops::{Range, RangeFrom, RangeTo},
    path::Path,
};

use anyhow::{Context, Result, bail};
use object::{File, Object, ObjectSection};

use image::Image;

#[derive(Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub struct Resolution {
    pub address: usize,
}

#[derive(Debug, Clone)]
pub struct Scan {
    pub section: Option<object::SectionKind>,
    pub scan_type: ScanType,
}
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
    ) -> Self {
        Self {
            sig,
            name,
            scan: Scan {
                section,
                scan_type: pattern.into(),
            },
        }
    }
    pub fn xref(sig: S, name: String, section: Option<object::SectionKind>, xref: Xref) -> Self {
        Self {
            sig,
            name,
            scan: Scan {
                section,
                scan_type: xref.into(),
            },
        }
    }
}

#[derive(Debug)]
pub struct ScanResult<'a, S> {
    pub results: Vec<(&'a PatternConfig<S>, Resolution)>,
}
impl<S: std::fmt::Debug + PartialEq> ScanResult<'_, S> {
    pub fn get_unique_sig_address(&self, sig: S) -> Result<usize> {
        let mut address = None;
        for (config, res) in &self.results {
            if config.sig == sig {
                if let Some(existing) = address {
                    if existing != res.address {
                        bail!("sig {sig:?} matched multiple addresses")
                    }
                } else {
                    address = Some(res.address)
                }
            }
        }
        address.with_context(|| format!("sig {sig:?} not found"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeFunction {
    pub range: Range<usize>,
    pub unwind: usize,
}
impl RuntimeFunction {
    pub fn read<'data>(
        memory: &(impl MemoryTrait<'data> + ?Sized),
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

pub trait SectionedMemoryTrait<'data>: MemoryTrait<'data> + Send + Sync {
    fn sections(&self) -> Box<dyn Iterator<Item = &dyn SectionTrait<'data>> + '_>;
    fn get_section_containing(
        &self,
        address: usize,
    ) -> Result<&dyn SectionTrait<'data>, MemoryAccessError> {
        self.sections()
            .find(|section| {
                address >= section.address() && address < section.address() + section.data().len()
            })
            .ok_or(MemoryAccessError::MemoryOutOfBoundsError)
    }
}
pub trait SectionTrait<'a>: MemoryBlockTrait<'a> + MemoryTrait<'a> + Send + Sync {
    fn name(&self) -> &str;
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
    MisalginedAddress(usize, usize),
}
impl std::error::Error for MemoryAccessError {}
impl std::fmt::Display for MemoryAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MemoryOutOfBoundsError => write!(f, "MemoryOutOfBoundsError"),
            Self::Utf8Error => write!(f, "Utf8Error"),
            Self::Utf16Error => write!(f, "Utf16Error"),
            Self::MisalginedAddress(addr, align) => {
                write!(f, "MisalginedAddress: address {:#x} != {:#x}", addr, align)
            }
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

    fn captures(
        &self,
        pattern: &Pattern,
        address: usize,
    ) -> Result<Option<Vec<patternsleuth_scanner::Capture<'_>>>, MemoryAccessError> {
        // TODO bounds check data passed to captures
        Ok(pattern.captures(self.range_from(address..)?, address, 0))
    }
}

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

#[derive(Default, Debug, PartialEq)]
pub struct SectionFlags {
    kind: Option<object::SectionKind>,
}

pub struct NamedMemorySection<'data> {
    name: String,
    flags: SectionFlags,
    address: usize,
    data: Cow<'data, [u8]>,
}

impl<'data> NamedMemorySection<'data> {
    fn new<D: Into<Cow<'data, [u8]>>>(
        name: String,
        address: usize,
        flags: SectionFlags,
        data: D,
    ) -> Self {
        Self {
            name,
            flags,
            address,
            data: data.into(),
        }
    }
}
impl NamedMemorySection<'_> {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
impl<'data> MemoryBlockTrait<'data> for NamedMemorySection<'data> {
    fn address(&self) -> usize {
        self.address
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
}

struct Memory<'data> {
    pub sections: Vec<NamedMemorySection<'data>>,
}
impl<'data> SectionedMemoryTrait<'data> for Memory<'data> {
    fn sections(&self) -> Box<dyn Iterator<Item = &dyn SectionTrait<'data>> + '_> {
        Box::new(
            self.sections
                .iter()
                .map(|s| -> &dyn SectionTrait<'data> { s }),
        )
    }
}
impl<'data> SectionTrait<'data> for NamedMemorySection<'data> {
    fn name(&self) -> &str {
        &self.name
    }
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
                        SectionFlags {
                            kind: Some(s.kind()),
                        },
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
                        SectionFlags {
                            kind: Some(s.kind()),
                        },
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
                        SectionFlags {
                            kind: Some(s.kind()),
                        },
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
                address >= section.address && address < section.address + section.data.len()
            })
            .ok_or(MemoryAccessError::MemoryOutOfBoundsError)
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
