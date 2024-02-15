use std::{collections::HashMap, ops::Range};

use crate::{Memory, MemoryAccessError, RuntimeFunction};

use super::{Image, ImageType};
use gimli::{BaseAddresses, CieOrFde, EhFrame, NativeEndian, UnwindSection};
use object::{Object, ObjectSection};
use anyhow::{Context, Result};

pub struct ElfImage {
    pub functions: Option<Vec<Range<usize>>>,
}

impl ElfImage {
    pub fn get_function(&self, image: &Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        self.get_root_function(image, address)
    }
    pub fn get_root_function(&self, image: &Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        let x = self.functions.as_ref().unwrap();
        x.iter().find(|p| p.contains(&address)).map(|a| 
            Ok(Some(
                RuntimeFunction {
                    range: a.clone(),
                    unwind: 0
                }
            ))
        ).unwrap_or(Ok(None))
    }
    pub fn get_child_functions(&self, image: &Image<'_>, address: usize) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        match self.get_function(image, address) {
            Ok(Some(f)) => Ok(vec![f]),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e)
        }
    }
}

// read_inner 
impl ElfImage {
    pub fn read_inner<'data, P: AsRef<std::path::Path>>(
        base_addr: Option<usize>,
        exe_path: Option<P>,
        load_functions: bool,
        memory: Memory<'_>,
        object: object::File<'_>,
    ) -> Result<Image<'data>, anyhow::Error> {
        let base_address = object.relative_address_base() as usize;
        let eh_frame = object.section_by_name(".eh_frame").unwrap();
        let eh_frame_hdr = object.section_by_name(".eh_frame_hdr").unwrap();
        let text = object.section_by_name(".text").unwrap();
        let bases = BaseAddresses::default()
                    .set_eh_frame_hdr(eh_frame_hdr.address() as _)
                    .set_eh_frame(eh_frame.address())
                    .set_text(text.address() as _);
        let eh_frameparsed = EhFrame::new(
            eh_frame.data().unwrap(), 
            NativeEndian
        );
        let mut entries = eh_frameparsed.entries(&bases);

        let mut result = Vec::<Range<usize>>::new();
        let mut syms = HashMap::default();
        
        while let Some(entry) = entries.next().context("Iter over entry failed")? {
            match entry {
                CieOrFde::Fde(partial) => {
                    let fde = partial
                        .parse(&mut EhFrame::cie_from_offset)
                        .context("Failed parse fde item")?;
                    // right now it's real address
                    let start = fde.initial_address() as usize;
                    let len = fde.len() as usize;
                    result.push(start .. (start + len));
                    syms.insert(start, format!("sub_{}", start));
                }
                CieOrFde::Cie(_) => {},
            }
        }
        result.sort_by(|a,b| a.start.cmp(&b.start));
        Ok(Image {
            base_address: base_address,
            memory: memory,
            symbols: Some(syms),
            imports: HashMap::default(),
            image_type: ImageType::ElfImage(ElfImage {
                functions: Some(result)
            }),
        })
    }
}