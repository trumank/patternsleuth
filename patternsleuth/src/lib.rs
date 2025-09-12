pub mod image;
pub mod process;
pub mod resolvers;
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
    ops::{Range, RangeFrom},
    path::Path,
};

use anyhow::{Context, Result, bail};
use image::Image;
use object::elf::SHF_ALLOC;
use object::pe::IMAGE_SCN_MEM_READ;
use object::{File, Object, ObjectSection, SectionFlags};

pub struct ResolveContext<'data, 'pattern> {
    pub exe: &'data Image<'data>,
    pub memory: &'data Memory<'data>,
    pub section: String,
    pub match_address: usize,
    pub scan: &'pattern Scan,
}

#[derive(Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub struct Resolution {
    pub address: u64,
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
    pub fn get_unique_sig_address(&self, sig: S) -> Result<u64> {
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
    pub range: Range<u64>,
    pub unwind: u64,
}
impl RuntimeFunction {
    pub fn read<'data>(
        memory: &impl MemoryTrait<'data>,
        base_address: u64,
        address: u64,
    ) -> Result<Self, MemoryAccessError> {
        let addr_begin = base_address + memory.u32_le(address)? as u64;
        let addr_end = base_address + memory.u32_le(address + 4)? as u64;
        let unwind = base_address + memory.u32_le(address + 8)? as u64;

        Ok(RuntimeFunction {
            range: addr_begin..addr_end,
            unwind,
        })
    }
}
impl RuntimeFunction {
    pub fn range(&self) -> Range<u64> {
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
    MisalginedAddress(u64, u64),
}
impl std::error::Error for MemoryAccessError {}
impl std::fmt::Display for MemoryAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MemoryOutOfBoundsError => write!(f, "MemoryOutOfBoundsError"),
            Self::Utf8Error => write!(f, "Utf8Error"),
            Self::Utf16Error => write!(f, "Utf16Error"),
            Self::MisalginedAddress(addr, align) => {
                write!(f, "MisalginedAddress: address {addr:#x} != {align:#x}")
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

/// Potentially sparse section of memory
pub trait MemoryTrait<'data> {
    /// Return slice of u8 at `range`
    fn range(&self, range: Range<u64>) -> Result<&[u8], MemoryAccessError> {
        let slice = self.range_from(range.start..)?;
        let len = (range.end - range.start) as usize;
        if len <= slice.len() {
            Ok(&slice[0..len])
        } else {
            Err(MemoryAccessError::MemoryOutOfBoundsError)
        }
    }
    /// Return slice of u8 from start of `range` to end of block
    fn range_from(&self, range: RangeFrom<u64>) -> Result<&[u8], MemoryAccessError>;

    fn array<const T: usize>(&self, address: u64) -> Result<[u8; T], MemoryAccessError> {
        Ok(self.range(address..address + T as u64)?.try_into().unwrap())
    }

    /// Return u8 at `address`
    fn u8(&self, address: u64) -> Result<u8, MemoryAccessError> {
        Ok(self.array::<1>(address)?[0])
    }
    /// Return i16 at `address`
    fn i16_le(&self, address: u64) -> Result<i16, MemoryAccessError> {
        Ok(i16::from_le_bytes(self.array(address)?))
    }
    /// Return u16 at `address`
    fn u16_le(&self, address: u64) -> Result<u16, MemoryAccessError> {
        Ok(u16::from_le_bytes(self.array(address)?))
    }
    /// Return i32 at `address`
    fn i32_le(&self, address: u64) -> Result<i32, MemoryAccessError> {
        Ok(i32::from_le_bytes(self.array(address)?))
    }
    /// Return u32 at `address`
    fn u32_le(&self, address: u64) -> Result<u32, MemoryAccessError> {
        Ok(u32::from_le_bytes(self.array(address)?))
    }
    /// Return u64 at `address`
    fn u64_le(&self, address: u64) -> Result<u64, MemoryAccessError> {
        Ok(u64::from_le_bytes(self.array(address)?))
    }
    /// Return ptr (u64) at `address`
    fn ptr(&self, address: u64) -> Result<u64, MemoryAccessError> {
        self.u64_le(address)
    }
    /// Return instruction relative address at `address`
    fn rip4(&self, address: u64) -> Result<u64, MemoryAccessError> {
        Ok((address + 4)
            .checked_add_signed(self.i32_le(address)? as i64)
            .unwrap())
    }

    /// Read null terminated string from `address`
    fn read_string(&self, address: u64) -> Result<String, MemoryAccessError> {
        let data = &self
            .range_from(address..)?
            .iter()
            .cloned()
            .take_while(|n| *n != 0)
            .collect::<Vec<u8>>();

        Ok(std::str::from_utf8(data)?.to_string())
    }

    /// Read null terminated wide string from `address`
    fn read_wstring(&self, address: u64) -> Result<String, MemoryAccessError> {
        let data = &self
            .range_from(address..)?
            .chunks(2)
            .map(|chunk| ((chunk[1] as u16) << 8) + chunk[0] as u16)
            .take_while(|n| *n != 0)
            .collect::<Vec<u16>>();

        Ok(String::from_utf16(data)?)
    }
}

impl<'data> MemoryTrait<'data> for NamedMemorySection<'data> {
    fn range_from(&self, range: RangeFrom<u64>) -> Result<&[u8], MemoryAccessError> {
        if (self.address()..self.address() + self.len() as u64).contains(&range.start) {
            Ok(&self.data()[(range.start - self.address()) as usize..])
        } else {
            Err(MemoryAccessError::MemoryOutOfBoundsError)
        }
    }
}

impl<'data> MemoryTrait<'data> for Memory<'data> {
    fn range_from(&self, range: RangeFrom<u64>) -> Result<&[u8], MemoryAccessError> {
        self.get_section_containing(range.start)?.range_from(range)
    }
}

pub struct NamedMemorySection<'data> {
    name: String,
    kind: object::SectionKind,
    address: u64,
    data: Cow<'data, [u8]>,
}

impl<'data> NamedMemorySection<'data> {
    fn new<T: Into<Cow<'data, [u8]>>>(
        name: String,
        address: u64,
        kind: object::SectionKind,
        data: T,
    ) -> Self {
        Self {
            name,
            kind,
            address,
            data: data.into(),
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
    pub fn address(&self) -> u64 {
        self.address
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

pub struct Memory<'data> {
    sections: Vec<NamedMemorySection<'data>>,
}

impl<'data> Memory<'data> {
    fn is_section_scannable(section_flags: SectionFlags) -> bool {
        match section_flags {
            SectionFlags::Coff { characteristics } => {
                // Must have MEM_READ
                if (characteristics & IMAGE_SCN_MEM_READ) == 0 {
                    return false;
                }
                
                // Exclude uninitialized data sections
                if (characteristics & 0x00000080) != 0 {
                    return false;
                }
                
                if (characteristics & 0xE0000000) == 0xE0000000 {
                    return false;  // Only if ALL three top bits are set
                }
                
                true
            },
            SectionFlags::Elf { sh_flags } => (sh_flags & SHF_ALLOC as u64) != 0,
            _ => true,
        }
    }
    pub fn new(object: &File<'data>) -> Result<Self> {
        Ok(Self {
            sections: object
                .sections()
                .filter(|s| Self::is_section_scannable(s.flags()))
                .map(|s| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address(),
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
                .filter(|(s, _)| Self::is_section_scannable(s.flags()))
                .map(|(s, d)| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address(),
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
                .filter(|(s, _)| Self::is_section_scannable(s.flags()))
                .map(|(s, d)| {
                    Ok(NamedMemorySection::new(
                        s.name()?.to_string(),
                        s.address(),
                        s.kind(),
                        d,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn sections(&self) -> &[NamedMemorySection<'_>] {
        &self.sections
    }
    pub fn get_section_containing(
        &self,
        address: u64,
    ) -> Result<&NamedMemorySection<'data>, MemoryAccessError> {
        self.sections
            .iter()
            .find(|section| {
                address >= section.address && address < section.address + section.len() as u64
            })
            .ok_or(MemoryAccessError::MemoryOutOfBoundsError)
    }

    pub fn captures(
        &'data self,
        pattern: &Pattern,
        address: u64,
    ) -> Result<Option<Vec<patternsleuth_scanner::Capture<'data>>>, MemoryAccessError> {
        let s = self.get_section_containing(address)?;
        // TODO bounds check data passed to captures
        Ok(pattern.captures(
            s.data(),
            s.address() as usize,
            (address - s.address()) as usize,
        ))
    }
}

pub trait Addressable {
    fn rip(&self) -> u64;
    fn ptr(&self) -> u64;
    fn u32(&self) -> u32;
}
impl Addressable for patternsleuth_scanner::Capture<'_> {
    fn rip(&self) -> u64 {
        (self.address as u64 + 4)
            .checked_add_signed(i32::from_le_bytes(self.data.try_into().unwrap()) as i64)
            .unwrap()
    }
    fn ptr(&self) -> u64 {
        u64::from_le_bytes(self.data.try_into().unwrap())
    }
    fn u32(&self) -> u32 {
        u32::from_le_bytes(self.data.try_into().unwrap())
    }
}

pub mod disassemble {
    use std::{collections::HashSet, ops::Range};

    use iced_x86::{Decoder, DecoderOptions, FlowControl, Formatter, Instruction, NasmFormatter};

    use crate::{Image, MemoryAccessError, MemoryTrait};

    pub fn function_range(exe: &Image<'_>, address: u64) -> Result<Range<u64>, MemoryAccessError> {
        let min = address;
        let mut max = min;
        disassemble(exe, address, |inst| {
            let cur = inst.ip();
            if Some(address) != exe.get_root_function(cur)?.map(|f| f.range.start) {
                return Ok(Control::Break);
            }
            max = max.max(cur + inst.len() as u64);
            Ok(Control::Continue)
        })?;
        Ok(min..max)
    }

    pub fn disassemble_single<'mem, 'img: 'mem>(
        exe: &'img Image<'mem>,
        address: u64,
    ) -> Result<Option<Instruction>, MemoryAccessError> {
        Ok(Decoder::with_ip(
            64,
            exe.memory.range_from(address..)?,
            address,
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
        address: u64,
        mut visitor: F,
    ) -> Result<(), MemoryAccessError>
    where
        F: FnMut(&Instruction) -> Result<Control, MemoryAccessError>,
    {
        struct Ctx<'mem, 'img: 'mem> {
            exe: &'img Image<'mem>,
            queue: Vec<u64>,
            visited: HashSet<u64>,
            address: u64,
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
            decoder: Decoder::with_ip(64, block, address, DecoderOptions::NONE),
            instruction: Default::default(),
        };

        impl Ctx<'_, '_> {
            #[allow(unused)]
            fn print(&self) {
                let mut formatter = NasmFormatter::new();
                //formatter.options_mut().set_digit_separator("`");
                //formatter.options_mut().set_first_operand_char_index(10);

                let mut output = String::new();
                formatter.format(&self.instruction, &mut output);

                // Eg. "00007FFAC46ACDB2 488DAC2400FFFFFF     lea       rbp,[rsp-100h]"
                print!("{:016X} ", self.instruction.ip());
                let start_index = self.instruction.ip() - self.address;
                let instr_bytes = &self.block
                    [start_index as usize..start_index as usize + self.instruction.len()];
                for b in instr_bytes.iter() {
                    print!("{b:02X}");
                }
                if instr_bytes.len() < 0x10 {
                    for _ in 0..0x10 - instr_bytes.len() {
                        print!("  ");
                    }
                }
                println!(" {output}");
            }
            fn start(&mut self, address: u64) -> Result<(), MemoryAccessError> {
                //println!("starting at {address:x}");
                self.address = address;
                self.block = self.exe.memory.range_from(self.address..)?;
                self.decoder = Decoder::with_ip(64, self.block, self.address, DecoderOptions::NONE);
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

            let addr = ctx.instruction.ip();

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
                    ctx.start(ctx.instruction.near_branch_target())?;
                }
                //FlowControl::IndirectBranch => todo!(),
                FlowControl::ConditionalBranch => {
                    ctx.queue.push(ctx.instruction.near_branch_target());
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
